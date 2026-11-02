"""Independent binary/page inspection and deterministic changed-source evidence. Standard library only."""
import hashlib
import json
from pathlib import Path
import struct
import sys


class Reader:
    def __init__(self, data):
        self.data, self.offset = data, 0

    def take(self, size):
        result = self.data[self.offset:self.offset + size]
        assert len(result) == size
        self.offset += size
        return result

    def u32(self):
        return struct.unpack('<I', self.take(4))[0]


def change_source(source, output):
    data = bytearray(source.read_bytes())
    r = Reader(data)
    assert r.take(4) == b'RLA\0' and r.u32() == 1
    meshes, placements, materials, _ = [r.u32() for _ in range(4)]
    for _ in range(meshes):
        for _ in range(r.u32()):
            _, vertices, indices = [r.u32() for _ in range(3)]
            r.take(vertices * 48 + indices * 4)
    r.take(placements * 68)
    assert materials > 0
    struct.pack_into('<f', data, r.offset + 20, 0.375)
    output.write_bytes(data)
    print(json.dumps(dict(source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),
                         changed_sha256=hashlib.sha256(data).hexdigest(),
                         change='first material roughness -> 0.375; source reference preserved')))


def inspect(cache, manifest):
    m = manifest.read_bytes()
    assert len(m) == 80 and m[:8] == b'RVM\0\x01\0\0\0'
    package_hash, size = m[40:72].hex(), struct.unpack('<Q', m[72:])[0]
    data = (cache / 'virtual-v1/packages' / package_hash).read_bytes()
    assert len(data) == size and hashlib.sha256(data).hexdigest() == package_hash
    r = Reader(data)
    assert r.take(4) == b'RLV\0' and r.u32() == 1
    meshes, placements, page_count, resources = [r.u32() for _ in range(4)]
    resource_data = r.take(resources)
    assert resource_data[:8] == b'RLA\0\x01\0\0\0'
    assert struct.unpack('<II', resource_data[8:16]) == (0, 0)
    all_clusters, terminal_groups, groups_total, coarse_clusters = [], [], 0, 0
    root_pages = set()
    scalar_cuts = []
    for mesh in range(meshes):
        ng, nc = r.u32(), r.u32()
        groups = []
        for g in range(ng):
            depth = r.u32()
            bounds = struct.unpack('<5f', r.take(20))
            start, count = r.u32(), r.u32()
            groups.append(dict(depth=depth, error=bounds[4], start=start, count=count))
        clusters = []
        for c in range(nc):
            owner, refined = r.u32(), r.u32()
            r.take(20)
            material, page, offset, count = [r.u32() for _ in range(4)]
            assert owner < ng and (refined == 0xffffffff or refined < owner)
            assert page < page_count and count > 0 and count % 3 == 0
            assert groups[owner]['start'] <= c < groups[owner]['start'] + groups[owner]['count']
            clusters.append(dict(owner=owner, refined=refined, page=page, offset=offset, count=count))
        incoming = {c['refined'] for c in clusters if c['refined'] != 0xffffffff}
        terminals = {g for g in range(ng) if g not in incoming}
        assert terminals == {g for g in range(ng) if groups[g]['error'] == struct.unpack('<f', bytes.fromhex('ffff7f7f'))[0]}
        for c in clusters:
            if c['owner'] in terminals:
                root_pages.add(c['page'])
                coarse_clusters += 1
        for threshold in [0, 0.0001, 0.001, 0.01, 0.1, 1, 10]:
            cut = [i for i, c in enumerate(clusters) if groups[c['owner']]['error'] > threshold and
                   (c['refined'] == 0xffffffff or groups[c['refined']]['error'] <= threshold)]
            scalar_cuts.append(dict(mesh=mesh, threshold=threshold, selected_clusters=len(cut)))
        all_clusters.extend(clusters)
        terminal_groups.extend((mesh, g) for g in terminals)
        groups_total += ng
    r.take(placements * 68)
    payload_bytes = 0
    intervals = [[] for _ in range(page_count)]
    for c in all_clusters:
        intervals[c['page']].append((c['offset'], c['offset'] + c['count']))
    for i in range(page_count):
        vertices, indices, payload = [r.u32() for _ in range(3)]
        digest = r.take(32).hex()
        b = (cache / 'virtual-v1/pages' / digest).read_bytes()
        assert len(b) == 65536 and hashlib.sha256(b).hexdigest() == digest
        assert payload == vertices * 48 + indices * 4 <= 65536 and not any(b[payload:])
        values = struct.unpack_from(f'<{indices}I', b, vertices * 48)
        assert max(values) < vertices
        previous = 0
        for start, end in sorted(intervals[i]):
            assert start == previous
            previous = end
        assert previous == indices
        payload_bytes += payload
    assert r.offset == len(data)
    return dict(package_sha256=package_hash, metadata_file_bytes=len(data), resources_sha256=hashlib.sha256(resource_data).hexdigest(),
                meshes=meshes, groups=groups_total, clusters=len(all_clusters), pages=page_count,
                payload_bytes=payload_bytes, disk_page_bytes=page_count*65536,
                minimum_cover_clusters=coarse_clusters, pinned_root_pages=sorted(root_pages),
                terminal_groups=terminal_groups, independent_upstream_scalar_cut_counts=scalar_cuts)


if __name__ == '__main__':
    if sys.argv[1] == 'change-source':
        change_source(*map(Path, sys.argv[2:]))
    else:
        print(json.dumps(inspect(*map(Path, sys.argv[1:])), indent=2))
