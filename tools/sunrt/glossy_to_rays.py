"""Adapt bfeb48ef's glossy dump to replay_geometry.py JSONL. CPU float32 model."""
import argparse
import hashlib
import json
from pathlib import Path
import numpy as np

F = np.float32
GOLDEN = F(.6180339887)


def normalize(v):
    return v / F(np.sqrt(np.dot(v, v)))


def oct_decode(word):
    raw = np.array([word & 65535, word >> 16], dtype='<u2').view('<i2')
    e = np.maximum(F(-1), raw.astype('f4')/F(32767))
    z = F(1)-abs(e[0])-abs(e[1])
    xy = (F(1)-np.abs(e[::-1]))*np.where(e >= 0, F(1), F(-1)) if z < 0 else e
    return normalize(np.array([xy[0], xy[1], z], dtype='f4'))


def hash32(x):
    x = ((x ^ (x >> 16))*0x7feb352d) & 0xffffffff
    x = ((x ^ (x >> 15))*0x846ca68b) & 0xffffffff
    return (x ^ (x >> 16)) & 0xffffffff


def basis(n):
    x, y, z = n
    s = F(1) if z >= 0 else F(-1)
    a = F(-1)/(s+z)
    b = x*y*a
    return np.column_stack((np.array([F(1)+s*x*x*a, s*b, -s*x], dtype='f4'),
                            np.array([b, s+y*y*a, -y], dtype='f4'), n))


def vndf(v, alpha, u):
    vh = normalize(np.array([alpha*v[0], alpha*v[1], v[2]], dtype='f4'))
    lensq = vh[0]*vh[0]+vh[1]*vh[1]
    t1 = np.array([-vh[1], vh[0], 0], dtype='f4')/F(np.sqrt(max(lensq, F(1e-30)))) if lensq > F(1e-12) else np.array([1, 0, 0], dtype='f4')
    t2 = np.cross(vh, t1)
    r, phi = F(np.sqrt(u[0])), F(6.283185307179586)*u[1]
    p1, s = r*F(np.cos(phi)), F(.5)*(F(1)+vh[2])
    p2 = (F(1)-s)*F(np.sqrt(max(F(1)-p1*p1, F(0))))+s*r*F(np.sin(phi))
    nh = p1*t1+p2*t2+F(np.sqrt(max(F(1)-p1*p1-p2*p2, F(0))))*vh
    return normalize(np.array([alpha*nh[0], alpha*nh[1], max(nh[2], F(1e-6))], dtype='f4'))


def directions(entry, frame):
    n, v, side = [oct_decode(int(entry[i])) for i in (4, 5, 6)]
    alpha = F(np.array([int(entry[7]) & 65535], dtype='<u2').view('<f2')[0])
    count = (int(entry[7]) >> 16) & 31
    pixel = int(entry[8])
    x, y = pixel & 65535, pixel >> 16
    seed = hash32(((x*0x9e3779b1) & 0xffffffff) ^ hash32(y ^ ((frame*0x85ebca6b) & 0xffffffff)))
    rotation = F(seed >> 8)*F(1/16777216)
    f = F(frame)*GOLDEN
    radial = F(f-F(np.floor(f))) if frame else F(.5)
    b = basis(n)
    local_v = np.array([np.dot(v, b[:,i]) for i in range(3)], dtype='f4')
    for k in range(count):
        angle = rotation+F(k)*GOLDEN
        u = np.array([(F(k)+radial)/F(count), angle-F(np.floor(angle))], dtype='f4')
        normal = b @ vndf(local_v, max(alpha, F(1e-6)), u)
        d = -v-F(2)*np.dot(normal, -v)*normal
        yield k, d, bool(np.dot(d, side) > 0)


def convert(geometry_path, list_path, output):
    g, m = json.loads(geometry_path.read_text()), json.loads(list_path.read_text())
    for key in ('frame', 'view', 'generation'):
        if g[key] != m[key]: raise ValueError('geometry/list '+key+' mismatch')
    if m['version'] != 1 or m['path'] != 'nonrt' or (m['entry_bytes'], m['header_bytes']) != (48, 16):
        raise ValueError('expected version-1 non-RT glossy layout')
    stem = str(list_path).removesuffix('.glossy.json')
    if geometry_path.with_suffix('').resolve() != Path(stem).resolve():
        raise ValueError('geometry/list renderer stem mismatch')
    list_file, epsilon_file = Path(stem+'.glossy.bin'), Path(stem+'.glossy-epsilon.bin')
    w = np.fromfile(list_file, dtype='<u4')
    eps = np.fromfile(epsilon_file, dtype='<f4')
    if len(w) != 4+12*m['entries'] or len(eps) != m['entries'] or int(w[0]) != m['appended']:
        raise ValueError('incomplete list/epsilon payload')
    if m['entries'] != min(m['appended'], m['capacity']): raise ValueError('invalid list capacity')
    entries = w[4:].reshape(-1,12)
    stats = dict(entries=m['entries'], appended=m['appended'], capacity=m['capacity'],
                 overflow_entries_not_listed=m['appended']-m['entries'],
                 slots=0, rays=0, horizon_rejected=0, invalid_entries=0, one_stratum_entries=0)
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open('w') as dest:
        for i, entry in enumerate(entries):
            count, first = (int(entry[7]) >> 16) & 31, int(entry[9])
            if not count or first+count > m['room']:
                stats['invalid_entries'] += 1
                continue
            stats['slots'] += count
            stats['one_stratum_entries'] += int(count == 1)
            origin = entry[:3].view('<f4')
            for k, direction, accepted in directions(entry, m['primary_frame_sided_y']):
                if not accepted:
                    stats['horizon_rejected'] += 1
                    continue
                stats['rays'] += 1
                ray = dict(id=f'{i}:{k}', slot=first+k, entry=i, frame=m['frame'], view=m['view'],
                           origin=origin.tolist(), direction=direction.tolist(), tmin=float(eps[i]), tmax=1e6)
                dest.write(json.dumps(ray, separators=(',',':'))+'\n')
    stats['trace_share'] = stats['rays']/stats['slots'] if stats['slots'] else None
    stats['ray_generation'] = 'CPU float32 port; shader FMA/trig rounding may differ; not GPU direction bit certification'
    stats['sources'] = {str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in (geometry_path, list_path, list_file, epsilon_file)}
    output.with_suffix('.summary.json').write_text(json.dumps(stats, indent=2)+'\n')
    return stats


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('geometry', type=Path)
    parser.add_argument('glossy', type=Path)
    parser.add_argument('--out', required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps(convert(args.geometry, args.glossy, args.out), indent=2))
