"""Independent inspection of self-authored GLB and prepared records."""
import hashlib
import json
import math
from pathlib import Path
import struct
import sys


class Reader:
    def __init__(self, data):
        self.data, self.offset = data, 0

    def take(self, count):
        assert 0 <= count <= len(self.data) - self.offset
        result = self.data[self.offset:self.offset + count]
        self.offset += count
        return result

    def u32(self):
        return struct.unpack('<I', self.take(4))[0]


def image_dimensions(data):
    if data.startswith(b'\x89PNG\r\n\x1a\n'):
        assert data[12:16] == b'IHDR'
        return list(struct.unpack('>II', data[16:24]))
    assert data[:2] == b'\xff\xd8'
    pos = 2
    while pos < len(data):
        assert data[pos] == 255
        marker = data[pos + 1]
        size = int.from_bytes(data[pos + 2:pos + 4], 'big')
        if marker in (0xc0, 0xc1, 0xc2):
            height, width = struct.unpack('>HH', data[pos + 5:pos + 9])
            return [width, height]
        assert size >= 2
        pos += 2 + size
    raise AssertionError('missing JPEG dimensions')


def inspect(source, cache, manifest):
    glb = source.read_bytes()
    magic, version, length, json_length, json_type = struct.unpack('<4s4I', glb[:20])
    assert magic == b'glTF' and version == 2 and length == len(glb)
    assert json_type == 0x4e4f534a
    root = json.loads(glb[20:20 + json_length])
    bin_length, bin_type = struct.unpack('<II', glb[20 + json_length:28 + json_length])
    binary = glb[28 + json_length:]
    assert bin_type == 0x004e4942 and len(binary) == bin_length
    source_vertices = source_indices = source_primitives = 0
    for mesh in root['meshes']:
        for primitive in mesh['primitives']:
            source_primitives += 1
            vertices = root['accessors'][primitive['attributes']['POSITION']]['count']
            source_vertices += vertices
            source_indices += root['accessors'][primitive['indices']]['count'] if 'indices' in primitive else vertices
    dimensions = []
    for image in root['images']:
        view = root['bufferViews'][image['bufferView']]
        start = view.get('byteOffset', 0)
        dimensions.append(image_dimensions(binary[start:start + view['byteLength']]))
    manifest_bytes = manifest.read_bytes()
    assert manifest_bytes[:4] == b'RLM\0'
    manifest_version = int.from_bytes(manifest_bytes[4:8], 'little')
    assert manifest_version in (1, 2, 3)
    payload_hash = manifest_bytes[40:72].hex()
    payload = (cache / 'assets' / payload_hash).read_bytes()
    assert hashlib.sha256(payload).hexdigest() == payload_hash
    assert len(payload) == int.from_bytes(manifest_bytes[72:80], 'little')
    geometry_count = int.from_bytes(manifest_bytes[80:84], 'little')
    assert len(manifest_bytes) == 84 + geometry_count * 40
    reader = Reader(payload)
    assert reader.take(4) == b'RLA\0'
    schema = reader.u32()
    assert 1 <= schema <= manifest_version
    meshes, placements, materials, textures = [reader.u32() for _ in range(4)]
    assert meshes == geometry_count
    vertices = indices = primitives = 0
    mesh_hashes = []
    for mesh_index in range(meshes):
        count = reader.u32()
        primitives += count
        mesh_version = 1
        records = []
        for _ in range(count):
            material, nv, ni = [reader.u32() for _ in range(3)]
            assert material < materials and nv > 0 and ni % 3 == 0
            vertices += nv
            indices += ni
            basis, tangents = (reader.u32(), reader.u32()) if schema >= 2 else (0, 0)
            assert basis in (0, 1, 2) and tangents == (nv if basis else 0)
            adjacency = reader.u32() if schema >= 3 else 0
            assert adjacency in (0, ni)
            mesh_version = max(mesh_version, 3 if adjacency else 2 if tangents else 1)
            attributes_start = reader.offset
            for _ in range(nv):
                values = struct.unpack('<12f', reader.take(48))
                assert all(math.isfinite(v) for v in values)
                assert any(v != 0 for v in values[3:6])
            for _ in range(tangents):
                assert all(math.isfinite(v) for v in struct.unpack('<4f', reader.take(16)))
            for _ in range(ni):
                assert reader.u32() < nv
            attributes_end = reader.offset
            for half in range(adjacency):
                neighbour = reader.u32()
                assert neighbour == 0xffffffff or (neighbour < ni // 3 and neighbour != half // 3)
            records.append((material, nv, ni, basis, tangents, adjacency,
                            payload[attributes_start:attributes_end], payload[attributes_end:reader.offset]))
        geometry = b'RLG\0' + struct.pack('<II', mesh_version, count)
        for material, nv, ni, basis, tangents, adjacency, attributes, neighbours in records:
            geometry += struct.pack('<III', material, nv, ni)
            if mesh_version >= 2:
                geometry += struct.pack('<II', basis, tangents)
            if mesh_version >= 3:
                geometry += struct.pack('<I', adjacency)
            geometry += attributes + neighbours
        digest = hashlib.sha256(geometry).hexdigest()
        entry = manifest_bytes[84 + mesh_index * 40:124 + mesh_index * 40]
        assert digest == entry[:32].hex()
        assert (cache / 'geometry' / digest).read_bytes() == geometry
        mesh_hashes.append(digest)
    transforms = []
    for _ in range(placements):
        assert reader.u32() < meshes
        values = struct.unpack('<16f', reader.take(64))
        assert values[3] == values[7] == values[11] == 0 and values[15] == 1
        transforms.append(list(values))
    material_records = []
    for _ in range(materials):
        factors = struct.unpack('<6f', reader.take(24))
        sided = reader.u32()
        refs = []
        for _ in range(3 if schema >= 2 else 2):
            present = reader.u32()
            assert present in (0, 1)
            refs.append([reader.u32() for _ in range(6)] if present else None)
        if schema >= 2:
            assert math.isfinite(struct.unpack('<f', reader.take(4))[0])
        material_records.append(dict(factors=factors, double_sided=sided, references=refs))
    texture_records = []
    texture_bytes = 0
    for _ in range(textures):
        role, levels = reader.u32(), reader.u32()
        mips = []
        for _ in range(levels):
            w, h, size = [reader.u32() for _ in range(3)]
            assert size == w * h * 4
            pixels = reader.take(size)
            texture_bytes += size
            mips.append(dict(dimensions=[w, h], bytes=size, first_pixel=list(pixels[:4])))
        assert mips[-1]['dimensions'] == [1, 1]
        texture_records.append(dict(role=role, mips=mips))
    assert reader.offset == len(payload)
    assert (meshes, primitives, vertices, indices) == (len(root['meshes']), source_primitives, source_vertices, source_indices)
    return dict(source=source.name, source_sha256=hashlib.sha256(glb).hexdigest(), source_bytes=len(glb),
                source_counts=dict(meshes=len(root['meshes']), primitives=source_primitives, nodes=len(root['nodes']),
                                   vertices=source_vertices, indices=source_indices, images=len(root['images']), image_dimensions=dimensions),
                prepared_sha256=payload_hash, prepared_bytes=len(payload), geometry_sha256=mesh_hashes,
                prepared_counts=dict(meshes=meshes, primitives=primitives, placements=placements, materials=materials,
                                     textures=textures, vertices=vertices, indices=indices, texture_bytes=texture_bytes),
                transforms_column_major=transforms, materials=material_records, textures=texture_records)


if __name__ == '__main__':
    source, cache, manifest = map(Path, sys.argv[1:])
    print(json.dumps(inspect(source, cache, manifest), indent=2))
