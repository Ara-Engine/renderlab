"""Describe the private quarry composition, budgets and fixed cameras as data."""
import argparse
import hashlib
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    args = parser.parse_args()
    root = args.root
    layout = [
        ("xb5ebhf", -1.55, 0.00, 0.45, 0.15, "ground left"),
        ("xb5gfgm", 0.25, 0.00, -0.85, 0.00, "ground rear"),
        ("xbnhajp", 0.35, 0.00, 0.85, -0.10, "ground foreground"),
        ("xb5lch2", 2.05, 0.00, 0.10, 0.35, "ground right"),
        ("xb5gbjo", -0.20, -0.04, 0.05, 0.25, "foreground slate boulder"),
        ("xb5sabi", 0.95, -0.05, 0.10, -0.45, "foreground broken slate"),
        ("xbkgcc1", -1.30, 0.18, -0.25, 0.45, "left bank"),
        ("xbkgcfh", 1.85, -0.05, -0.40, -0.70, "right bank"),
        ("xbkgdbu", -0.95, 0.65, -0.95, 0.20, "upper left bank"),
        ("xbkndhe", 0.55, 0.25, -1.00, 0.05, "rear exposed ledge"),
        ("xcghcfe", -0.70, 0.00, -2.10, 0.00, "large rear quarry face"),
        ("xckjagi", 1.55, 0.00, -1.75, -0.55, "right quarry face"),
    ]
    inventory = json.loads((root / "archive-inventory.json").read_text())
    source_entries = {e["id"]: e for e in inventory["scans"]}
    hashes = {r["entry"]: r for line in (root / "source-hashes.jsonl").read_text().splitlines() if (r := json.loads(line))}
    placements, scans = [], []
    for identifier, x, y, z, angle, role in layout:
        folder = root / "assets" / identifier
        inspection = json.loads((folder / "inspection.json").read_text())
        lo, hi = inspection["bounds_blender_world"]
        # glTF maps Blender world (X,Y,Z) to (X,Z,-Y). Center in XZ and align the original base.
        local_center = [(lo[0]+hi[0])/2, lo[2], -(lo[1]+hi[1])/2]
        import math
        c, s = math.cos(angle), math.sin(angle)
        rotated = [c*local_center[0]+s*local_center[2], local_center[1],
                   -s*local_center[0]+c*local_center[2]]
        placement = dict(scan_id=identifier, translation=[x-rotated[0], y-rotated[1], z-rotated[2]],
                         rotation_y_radians=angle, uniform_scale=1.0)
        placements.append(placement)
        entry = source_entries[identifier]
        scans.append({"shape": "ScanEntry", **entry, "identity": hashes[entry["source_entry"]],
                      "role": role, "inspection": inspection,
                      "texture_maps": [{**m, **hashes.get(m["entry"], {})} for m in entry["texture_maps"]]})
    (root / "placements.json").write_text(json.dumps(placements, indent=2) + "\n")
    manifest = {"archive": inventory["archive"], "content": "authored composition from scan collection, no original scene layout supplied",
                "unit_assumption": "Blender FBX global import scale 1, imported 0.01 object scale retained. Treat glTF units as metres. Physical scale unverified without acquisition metadata.",
                "rights": "License and acquisition provenance absent. Private local evaluation only. No redistribution assertion.",
                "geometry": "All selected source triangles retained. No decimation, welding or geometry normalization. Uniform placement scales are 1.",
                "material": "2048-square derived JPEG base color, opaque double-sided unlit. Constant metallic 0 and roughness 0.85. Other source channels unused. No conversion lighting bake.",
                "scans": scans, "placements": placements,
                "excluded": [e for e in inventory["scans"] if not e["opaque_candidate"]]}
    (root / "scan-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    limits = {"cook": {"source_bytes": 1073741824, "image_decoded_bytes": 1073741824},
              "prepared": {"max_file_bytes": 2147483648, "max_decoded_bytes": 2147483648,
                           "max_vertices": 16000000, "max_indices": 90000000},
              "virtual": {"max_groups": 1000000, "max_clusters": 1000000, "max_pages": 65536,
                          "max_metadata_bytes": 536870912, "max_geometry_bytes": 2147483648},
              "runtime": {"geometry_bytes": 536870912, "metadata_bytes": 536870912,
                          "texture_bytes": 536870912, "bootstrap_bytes": 1073741824,
                          "max_candidates": 16384, "max_clusters": 262144},
              "renderer": {"max_mesh_bytes": 134217728, "max_texture_bytes": 134217728}}
    (root / "limits.json").write_text(json.dumps(limits, indent=2) + "\n")
    frames = []
    views = [([5.2, 3.4, 6.2], [0, 0.6, -0.15], "wide"),
             ([1.5, 1.25, 2.6], [-0.10, 0.55, 0.02], "close"),
             ([2.7, 0.25, 1.8], [-0.6, 0.2, -0.05], "grazing"),
             ([-3.7, 2.3, 2.7], [0, 0.6, -0.6], "cut_reverse")]
    for eye, target, phase in views:
        for n in range(4):
            frames.append(dict(eye=[eye[0]+n*0.025, eye[1], eye[2]-n*0.025], target=target,
                               up=[0, 1, 0], fov_y_radians=0.85, near=0.005, far=100,
                               width=1920, height=1080, frame=len(frames), phase=phase))
    (root / "cameras.json").write_text(json.dumps({"frames": frames}, indent=2) + "\n")
    for path in [root / name for name in ("scan-manifest.json", "placements.json", "cameras.json", "limits.json")]:
        print(path.name, hashlib.sha256(path.read_bytes()).hexdigest())


if __name__ == "__main__":
    main()
