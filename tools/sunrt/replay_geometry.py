"""CPU replay of exported BVH2 words; no renderer, GPU, or list-format assumptions.

The future list adapter must emit the actual shader rays (including lift/epsilon,
seed and horizon rejection), not synthetic directions. Float32 CPU arithmetic can
differ from shader FMA, so compare GPU hit records before interpreting near ties.
"""
import argparse
import collections
import csv
import hashlib
import json
from pathlib import Path
import numpy as np

F = np.float32
LEAF = 0x80000000
COUNTERS = ('tlas_nodes', 'tlas_box_tests', 'tlas_instance_entries', 'instance_box_tests',
            'instance_box_entries', 'blas_visits', 'blas_nodes',
            'blas_box_tests', 'triangle_tests')


def next_up(t):
    return np.nextafter(F(t), F(np.inf), dtype=np.float32)


def inverse(d):
    return F(1) / np.where(np.abs(d) < F(1e-30),
                           np.where(d >= 0, F(1e-30), F(-1e-30)), d)


class Geometry:
    def __init__(self, metadata):
        self.path = Path(metadata)
        self.meta = json.loads(self.path.read_text())
        m = self.meta
        if m['version'] != 1 or m['format'] != 'reflection-bvh2-u32-le':
            raise ValueError('unsupported geometry format')
        if m['path'] != 'nonrt-software-reflections' or m['hardware_acceleration_structures']:
            raise ValueError('software geometry required')
        if (m['header_words'], m['instance_words']) != (8, 24):
            raise ValueError('unexpected record layout')
        binary = self.path.with_suffix('.words.bin')
        if binary.stat().st_size != m['payload_words'] * 4:
            raise ValueError('incomplete payload')
        self.w = np.memmap(binary, dtype='<u4', mode='r')
        self.f = self.w.view('<f4')
        if int(self.w[0]) != m['written_instances']:
            raise ValueError('instance count differs from header')
        if list(map(int, self.w[1:4])) != [m['tlas'][k] for k in ('base_word', 'root', 'refits')]:
            raise ValueError('TLAS metadata differs from payload')
        included = sum(p['included'] for d in m['source_draws'] for p in d['placements'])
        if included != m['written_instances']:
            raise ValueError('included provenance differs from packing')

    def tree(self, root, base, tlas=False):
        pending = [(root, 0)]
        visited = set()
        depth = nodes = leaves = primitives = max_pending = 0
        while pending:
            max_pending = max(max_pending, len(pending))
            child, level = pending.pop()
            depth = max(depth, level)
            if child & LEAF:
                leaves += 1
                if tlas:
                    assert (child & ~LEAF) < int(self.w[0])
                    primitives += 1
                else:
                    primitives += child & 7
                continue
            if child in visited:
                raise ValueError('cycle or shared internal tree node')
            visited.add(child)
            nodes += 1
            at = base + child * 16
            assert at + 16 <= len(self.w)
            pending.extend((int(self.w[at+k]), level+1) for k in (12, 13))
        return dict(nodes=nodes, leaves=leaves, primitives=primitives,
                    depth=depth, max_pending=max_pending)

    def inventory(self):
        meshes = []
        by_base = {}
        for m in self.meta['meshes']:
            result = self.tree(m['root'], m['node_base_word'])
            assert result['nodes'] == m['nodes']
            assert result['primitives'] == m['triangles']
            assert result['depth'] == m['depth']
            meshes.append(dict(mesh=m['mesh'], **result))
            by_base[(m['node_base_word'], m['triangle_base_word'])] = m['mesh']
        instances = collections.Counter()
        for i in range(int(self.w[0])):
            at = 8 + i*24
            instances[by_base[tuple(map(int, self.w[at+18:at+20]))]] += 1
        top = self.tree(int(self.w[2]), int(self.w[1]), True) if self.w[1] else None
        return dict(frame=self.meta['frame'], view=self.meta['view'],
                    generation=self.meta['generation'],
                    written_instances=int(self.w[0]),
                    reserved_instances=self.meta['reserved_instances'],
                    mesh_instances=dict(instances), meshes=meshes, tlas=top)

    @staticmethod
    def box(o, inv, lo, hi, tmin, tmax):
        a, b = (lo-o)*inv, (hi-o)*inv
        near = max(np.minimum(a, b).max(), tmin)
        far = min(np.maximum(a, b).min(), tmax)
        return near if near <= far else F(np.finfo('f4').max)

    def trace(self, origin, direction, tmin, tmax, any_hit=False):
        o, d = np.array(origin, dtype='f4'), np.array(direction, dtype='f4')
        if not np.isfinite(o).all() or not np.isfinite(d).all():
            raise ValueError('non-finite input ray')
        tmin, best_t = F(tmin), F(tmax)
        best_i, record = 0xffffffff, [0, 0, 0, 0]
        counts = collections.Counter({k: 0 for k in COUNTERS})
        world_inv = inverse(d)

        def limit(i):
            return next_up(best_t) if record[0] and i < best_i else best_t

        def children(at, origin, inv, bound, counter):
            counts[counter] += 2
            a = self.box(origin, inv, self.f[at:at+3], self.f[at+3:at+6], tmin, bound)
            b = self.box(origin, inv, self.f[at+6:at+9], self.f[at+9:at+12], tmin, bound)
            pairs = [(a, int(self.w[at+12])), (b, int(self.w[at+13]))]
            # Stable equal-distance order: child 0 first, as in WGSL.
            return [child for t, child in sorted(pairs, key=lambda p:p[0]) if t <= bound]

        def instance(i):
            nonlocal best_t, best_i, record
            counts['tlas_instance_entries'] += 1
            r = 8+i*24
            if any_hit and self.w[r+22]:
                return False
            counts['instance_box_tests'] += 1
            if self.box(o, world_inv, self.f[r+12:r+15], self.f[r+15:r+18], tmin, limit(i)) > limit(i):
                return False
            counts['instance_box_entries'] += 1
            counts['blas_visits'] += 1
            rows = self.f[r:r+12].reshape(3, 4)
            local_o = np.array([np.dot(row, np.append(o, F(1))) for row in rows], dtype='f4')
            local_d = np.array([np.dot(row[:3], d) for row in rows], dtype='f4')
            inv = inverse(local_d)
            nodes, triangles, root, slot = map(int, self.w[r+18:r+22])
            stack = [root]
            while stack:
                child = stack.pop()
                if child & LEAF:
                    start = (child & ~LEAF) >> 3
                    for k in range(child & 7):
                        counts['triangle_tests'] += 1
                        at = triangles+(start+k)*12
                        a, e1, e2 = (self.f[at+j:at+j+3] for j in (0, 3, 6))
                        p = np.cross(local_d, e2)
                        det = np.dot(e1, p)
                        if abs(det) < F(1e-30): continue
                        inv_det = F(1)/det
                        s = local_o-a
                        u = np.dot(s, p)*inv_det
                        if u < -F(4e-6) or u > F(1)+F(4e-6): continue
                        q = np.cross(s, e1)
                        v = np.dot(local_d, q)*inv_det
                        if v < -F(4e-6) or u+v > F(1)+F(4e-6): continue
                        t = np.dot(e2, q)*inv_det
                        if t > tmin and t < limit(i):
                            best_t, best_i = t, i
                            record = [slot+1, int(self.w[at+9]), int(u.view('u4')), int(v.view('u4'))]
                            if any_hit: return True
                else:
                    counts['blas_nodes'] += 1
                    near_first = children(nodes+child*16, local_o, inv, limit(i), 'blas_box_tests')
                    stack.extend(reversed(near_first))
                    if len(stack) > 32:
                        raise ValueError('BLAS pending stack exceeds shader bound; never silently drop')
            return False

        if self.w[1]:
            stack = [int(self.w[2])]
            while stack:
                child = stack.pop()
                if child & LEAF:
                    if instance(child & ~LEAF): break
                else:
                    counts['tlas_nodes'] += 1
                    bound = next_up(best_t) if record[0] else best_t
                    stack.extend(reversed(children(int(self.w[1])+child*16, o, world_inv, bound, 'tlas_box_tests')))
                    if len(stack) > 32:
                        raise ValueError('TLAS pending stack exceeds shader bound; never silently drop')
        else:
            for i in range(int(self.w[0])):
                if instance(i): break
        counts.setdefault('tlas_box_tests', 0)
        return dict(t=float(best_t), record=record, **counts)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('geometry', type=Path)
    parser.add_argument('--rays', type=Path, help='JSONL exact shader rays: frame,view,origin,direction,tmin,tmax; optional id,slot,any_hit,gpu_record')
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    geometry = Geometry(args.geometry)
    args.out.mkdir(parents=True, exist_ok=True)
    inventory = geometry.inventory()
    inventory['metadata_sha256'] = hashlib.sha256(args.geometry.read_bytes()).hexdigest()
    (args.out/'inventory.json').write_text(json.dumps(inventory, indent=2)+'\n')
    if args.rays:
        totals = collections.Counter()
        rays = matches = mismatches = checked = 0
        with args.rays.open() as source, (args.out/'rays.csv').open('w', newline='') as dest:
            writer = None
            for n, line in enumerate(source):
                ray = json.loads(line)
                if (ray['frame'], ray['view']) != (geometry.meta['frame'], geometry.meta['view']):
                    raise ValueError('geometry and ray frame/view mismatch')
                result = geometry.trace(ray['origin'], ray['direction'], ray['tmin'], ray['tmax'], ray.get('any_hit', False))
                rays += 1
                totals.update({key: result[key] for key in COUNTERS})
                if 'gpu_record' in ray:
                    checked += 1
                    # Primitive identity first; barycentric float bits may differ on CPU.
                    same = result['record'][:2] == ray['gpu_record'][:2]
                    matches += int(same)
                    mismatches += int(not same)
                row = dict(id=ray.get('id', n), slot=ray.get('slot'), **result)
                if writer is None:
                    writer = csv.DictWriter(dest, fieldnames=row)
                    writer.writeheader()
                writer.writerow(row)
        summary = dict(rays=rays, total_counts=dict(totals),
                       mean_counts={key:totals[key]/rays for key in COUNTERS} if rays else {},
                       gpu_identity_checked=checked, gpu_identity_matches=matches,
                       gpu_identity_mismatches=mismatches,
                       caution='CPU float32 traversal model; no shader FMA/tie bit-identity claim')
        (args.out/'replay-summary.json').write_text(json.dumps(summary, indent=2)+'\n')
    print(json.dumps(dict(frame=inventory['frame'], instances=inventory['written_instances'], meshes=len(inventory['meshes']))))


if __name__ == '__main__':
    main()
