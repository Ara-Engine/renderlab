"""Stream prepared texture byte counts without decoding scan geometry or images."""
import argparse
import json
from pathlib import Path
import struct


def count(path):
    with path.open("rb") as f:
        def word():
            return struct.unpack("<I", f.read(4))[0]
        assert f.read(4) == b"RLA\0"
        version, meshes, placements, materials, textures = [word() for _ in range(5)]
        assert 1 <= version <= 6
        for _ in range(meshes):
            for _ in range(word()):
                _, vertices, indices = [word() for _ in range(3)]
                tangents = 0
                if version >= 2:
                    _, tangents = word(), word()
                f.seek(vertices * 48 + indices * 4 + tangents * 16, 1)
        f.seek(placements * 68, 1)
        for _ in range(materials):
            f.seek(28, 1)
            for _ in range(3 if version >= 2 else 2):
                if word():
                    f.seek(24, 1)
            if version >= 2:
                f.seek(4, 1)
        records = []
        for _ in range(textures):
            role, levels = word(), word()
            mips = []
            for _ in range(levels):
                width, height, size = word(), word(), word()
                mips.append(dict(width=width, height=height, bytes=size))
                f.seek(size, 1)
            records.append(dict(role=role, bytes=sum(x["bytes"] for x in mips), mips=mips))
        return dict(source=str(path), source_bytes=path.stat().st_size, version=version,
                    meshes=meshes, materials=materials, textures=records,
                    normal_bytes=sum(t["bytes"] for t in records if t["role"] in [2,3,4]))


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("source", type=Path); p.add_argument("output", type=Path)
    a = p.parse_args()
    a.output.write_text(json.dumps(count(a.source), indent=2))
