"""Summarize imported scan bounds and make an unlit contact sheet of available previews."""
import argparse
import json
from pathlib import Path

from PIL import Image, ImageDraw


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    args = parser.parse_args()
    inventory = json.loads((args.root / "archive-inventory.json").read_text())
    sources = {entry["id"]: entry for entry in inventory["scans"]}
    hashes = {record["entry"]: record for line in (args.root / "source-hashes.jsonl").read_text().splitlines()
              if (record := json.loads(line))}
    entries = []
    for folder in sorted((args.root / "assets").iterdir()):
        path = folder / "inventory.json"
        if not path.exists():
            path = folder / "inspection.json"
        if not path.exists():
            continue
        report = json.loads(path.read_text())
        lo, hi = report["bounds_blender_world"]
        entry = {"shape": "ScanEntry", **sources[folder.name],
                 "source_identity": hashes[sources[folder.name]["source_entry"]], "inspection": report,
                 "id": folder.name, "triangles": sum(o["triangles"] for o in report["objects"]),
                 "extent_xyz": [round(b-a, 4) for a, b in zip(lo, hi)], "bounds_blender": [lo, hi],
                 "materials": [o["material_slots"] for o in report["objects"]],
                 "uv_layers": [o["uv_layers"] for o in report["objects"]]}
        entries.append(entry)
        print(f"{entry['id']:25s} {entry['triangles']:10,d} {entry['extent_xyz']}")
    (args.root / "catalog.json").write_text(json.dumps(entries, indent=2) + "\n")
    previews = sorted((args.root / "assets").glob("*/preview.png"))
    if previews:
        sheet = Image.new("RGB", (4 * 320, ((len(previews) + 3) // 4) * 264), (28, 31, 36))
        draw = ImageDraw.Draw(sheet)
        for i, path in enumerate(previews):
            with Image.open(path) as source:
                source.thumbnail((320, 240))
                x, y = i % 4 * 320, i // 4 * 264
                sheet.paste(source, (x, y))
                draw.text((x+8, y+243), path.parent.name, fill="white")
        sheet.save(args.root / "contact-sheet.png")


if __name__ == "__main__":
    main()
