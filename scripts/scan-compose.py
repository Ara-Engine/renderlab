"""Merge independently exported scans without changing geometry or embedded texture bytes."""
from __future__ import annotations

import argparse
from dataclasses import dataclass, asdict
import hashlib
import json
import math
from pathlib import Path
import shutil
import struct


@dataclass
class ScenePlacement:
    scan_id: str
    translation: list[float]
    rotation_y_radians: float = 0.0
    uniform_scale: float = 1.0


def glb_json(path):
    with path.open("rb") as source:
        magic, version, total = struct.unpack("<III", source.read(12))
        assert magic == 0x46546C67 and version == 2 and total == path.stat().st_size
        length, kind = struct.unpack("<II", source.read(8))
        assert kind == 0x4E4F534A
        document = json.loads(source.read(length))
        length, kind = struct.unpack("<II", source.read(8))
        assert kind == 0x004E4942
        return document, source.tell(), length


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("placements", type=Path, help="JSON array of ScenePlacement")
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    placements = [ScenePlacement(**x) for x in json.loads(args.placements.read_text())]
    result = {"asset": {"version": "2.0", "generator": "Renderlab private scan composition"},
              "scene": 0, "scenes": [{"nodes": []}], "buffers": [{"byteLength": 0}]}
    arrays = ["bufferViews", "accessors", "images", "samplers", "textures", "materials", "meshes", "nodes"]
    result.update({key: [] for key in arrays})
    chunks, entries, scene_bounds = [], [], []
    binary_size = 0
    for placement in placements:
        folder = args.root / "assets" / placement.scan_id
        path = folder / "scan.glb"
        doc, offset, size = glb_json(path)
        if doc.get("extensionsUsed") or doc.get("extensionsRequired"):
            raise ValueError("export contains unsupported extensions")
        shifts = {key: len(result[key]) for key in arrays}
        for view in doc.get("bufferViews", []):
            view["byteOffset"] = view.get("byteOffset", 0) + binary_size
        for accessor in doc.get("accessors", []):
            accessor["bufferView"] += shifts["bufferViews"]
        for image in doc.get("images", []):
            image["bufferView"] += shifts["bufferViews"]
        for texture in doc.get("textures", []):
            texture["source"] += shifts["images"]
            if "sampler" in texture:
                texture["sampler"] += shifts["samplers"]
        for material in doc.get("materials", []):
            for key in ("baseColorTexture", "metallicRoughnessTexture"):
                texture = material.get("pbrMetallicRoughness", {}).get(key)
                if texture:
                    texture["index"] += shifts["textures"]
        triangles = vertices = 0
        for mesh in doc["meshes"]:
            for primitive in mesh["primitives"]:
                if set(primitive["attributes"]) - {"POSITION", "NORMAL", "TEXCOORD_0", "COLOR_0"}:
                    raise ValueError("unsupported vertex channels")
                vertices += doc["accessors"][primitive["attributes"]["POSITION"]]["count"]
                triangles += doc["accessors"][primitive["indices"]]["count"] // 3
                primitive["indices"] += shifts["accessors"]
                primitive["attributes"] = {k: v + shifts["accessors"] for k, v in primitive["attributes"].items()}
                primitive["material"] += shifts["materials"]
        for node in doc["nodes"]:
            if "mesh" in node:
                node["mesh"] += shifts["meshes"]
            if "children" in node:
                node["children"] = [v + shifts["nodes"] for v in node["children"]]
        children = [n + shifts["nodes"] for n in doc["scenes"][doc.get("scene", 0)]["nodes"]]
        for key in arrays:
            result[key].extend(doc.get(key, []))
        wrapper = len(result["nodes"])
        result["nodes"].append({"name": placement.scan_id, "children": children,
                                "translation": placement.translation,
                                "rotation": [0, math.sin(placement.rotation_y_radians / 2), 0,
                                             math.cos(placement.rotation_y_radians / 2)],
                                "scale": [placement.uniform_scale] * 3})
        result["scenes"][0]["nodes"].append(wrapper)
        inspection = json.loads((folder / "inspection.json").read_text())
        lo, hi = inspection["bounds_blender_world"]
        for x in (lo[0], hi[0]):
            for y in (lo[1], hi[1]):
                for z in (lo[2], hi[2]):
                    c, s = math.cos(placement.rotation_y_radians), math.sin(placement.rotation_y_radians)
                    v = [c*x - s*y, z, -s*x - c*y]
                    scene_bounds.append([v[i] * placement.uniform_scale + placement.translation[i] for i in range(3)])
        with path.open("rb") as source:
            identity = hashlib.file_digest(source, "sha256").hexdigest()
        entries.append({"scan_id": placement.scan_id, "glb_sha256": identity,
                        "triangles": triangles, "vertices": vertices, "placement": asdict(placement)})
        chunks.append((path, offset, size))
        binary_size += size
    result["buffers"][0]["byteLength"] = binary_size
    encoded = json.dumps(result, separators=(",", ":")).encode()
    encoded += b" " * (-len(encoded) % 4)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("wb") as output:
        output.write(struct.pack("<III", 0x46546C67, 2, 28 + len(encoded) + binary_size))
        output.write(struct.pack("<II", len(encoded), 0x4E4F534A))
        output.write(encoded)
        output.write(struct.pack("<II", binary_size, 0x004E4942))
        for path, offset, size in chunks:
            with path.open("rb") as source:
                source.seek(offset)
                shutil.copyfileobj(source, output, 1024 * 1024)
    with args.output.open("rb") as source:
        identity = hashlib.file_digest(source, "sha256").hexdigest()
    report = {"shape": "ScenePlacement", "entries": entries,
              "glb_sha256": identity, "glb_bytes": args.output.stat().st_size,
              "triangles": sum(e["triangles"] for e in entries),
              "bounds_gltf": [[min(p[i] for p in scene_bounds) for i in range(3)],
                              [max(p[i] for p in scene_bounds) for i in range(3)]]}
    args.output.with_suffix(".json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
