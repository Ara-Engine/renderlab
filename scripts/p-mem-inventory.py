"""Summarize ARA1_MEMORY inventories without opening a device or reading image dumps.

py -3 -B scripts/p-mem-inventory.py BEFORE_DIR AFTER_DIR OUTPUT_DIR
Each input contains <scene>-<rt|nonrt>/memory.json and memory-upload.json.
All sizes come from the allocator, including backend padding, not nominal texels.
"""
import csv
import json
from pathlib import Path
import sys

TEXTURES = {
    "AT1 camera volume", "AT1 multiple-scattering LUT", "AT1 sky-view LUT",
    "AT1 transmittance LUT (+ fog ambient row)", "LC1 light cluster lists",
    "LC1 local shadow maps", "N1c conservative min depth", "N1c hardware-cluster depth",
    "N1c unused HZB base input", "N8 receiver levels", "N8c hardware raster area",
    "TP2 coloured transmission", "TR1 frame samples", "TR1 history anchors",
    "TR1 history colour", "TR1 silhouette slots", "environment lobe table",
    "exposed linear HDR", "fresh exact world position and depth",
    "fresh geometric normal and coverage", "fresh visible ray counts",
    "immutable environment cube", "immutable material texture", "post DOF",
    "post bloom down", "post bloom up", "post depth velocity", "post identity LUT",
    "post motion", "post velocity max", "prepared DFG RG16F",
    "primary hard receiver epsilon", "primary hit records", "primary tile coverage",
    "redrawn hard shadow depth array", "renderlab engine output",
}
HISTORY = {"TR1 history anchors", "TR1 history colour", "TR1 silhouette slots"}


def category(name):
    if name.startswith("renderlab hardware"):
        return "BLAS/TLAS"
    if "staging" in name.lower() or "scratch" in name.lower():
        return "Upload/build staging"
    if name in HISTORY:
        return "TR1 history textures"
    if name.startswith("indirect cache"):
        return "Software GI geometry buffer" if name.endswith("software geometry") else "GI cache buffers"
    if name.startswith("N8"):
        return "Sun-shadow textures" if name in TEXTURES else "Sun-shadow buffers"
    if name.startswith("post "):
        return "Post textures" if name in TEXTURES else "Post uniforms"
    if name in {"immutable material texture", "immutable environment cube", "environment lobe table", "prepared DFG RG16F"} or name.startswith("AT1") and name in TEXTURES:
        return "Material/environment/LUT textures"
    if name in {"LC1 local shadow maps", "LC1 local shadow view matrices"}:
        return "Local shadow atlas"  # textures and its view-matrix uniform buffer
    if name in TEXTURES:
        return "Other frame targets/textures"
    if name.startswith(("mesh ", "instance ", "normal transforms", "visible instance", "N1c cluster", "N1c LOD")):
        return "Geometry buffers"
    return "Other buffers"


def read(root, case, file="memory.json"):
    path = root / case / file
    return json.loads(path.read_text()) if path.exists() else None


def totals(report):
    groups = {}
    for item in report["by_label"]:
        group = category(item["name"])
        groups[group] = groups.get(group, 0) + item["bytes"]
    assert sum(groups.values()) == report["allocated_bytes"]
    return dict(groups=groups, allocated_bytes=report["allocated_bytes"],
                reserved_bytes=report["reserved_bytes"],
                slack_bytes=report["reserved_bytes"] - report["allocated_bytes"],
                dxgi_local_bytes=max((r["local_used_bytes"] for r in report["dxgi"]), default=None),
                dxgi_nonlocal_bytes=max((r["nonlocal_used_bytes"] or 0 for r in report["dxgi"]), default=None))


def main():
    before, after, out = map(Path, sys.argv[1:])
    out.mkdir(parents=True, exist_ok=True)
    cases = sorted({p.parent.name for root in [before, after] for p in root.glob("*/memory.json")})
    rows, summary = [], {}
    for case in cases:
        a, b = read(before, case), read(after, case)
        summary[case] = {"before": totals(a) if a else None, "after": totals(b) if b else None}
        for side, root in [("before", before), ("after", after)]:
            upload = read(root, case, "memory-upload.json")
            summary[case][side + "_upload"] = totals(upload) if upload else None
            engine = read(root, case, "engine.json")
            if summary[case][side] and engine:
                snapshots = [frame["dxgi"] for frame in engine["frames"] if frame.get("dxgi")]
                snapshots += [r["dxgi"] for r in [upload, a if side == "before" else b] if r]
                summary[case][side]["dxgi_captured_peak_bytes"] = max(
                    adapter["local_used_bytes"] for snapshot in snapshots for adapter in snapshot)
        for phase, reports in [("warm", [a, b]), ("upload", [read(root, case, "memory-upload.json")
                                                          for root in [before, after]])]:
            labels = [{r["name"]: r for r in report["by_label"]} if report else {} for report in reports]
            for name in sorted(labels[0].keys() | labels[1].keys()):
                values = [label.get(name, {"bytes": 0, "count": 0}) for label in labels]
                rows.append(dict(case=case, phase=phase, name=name,
                    kind="acceleration structure" if name.startswith("renderlab hardware") else "texture" if name in TEXTURES else "buffer",
                    category=category(name), before_bytes=values[0]["bytes"], after_bytes=values[1]["bytes"] if reports[1] else "",
                    saved_bytes=values[0]["bytes"]-values[1]["bytes"] if reports[1] else "",
                    before_count=values[0]["count"], after_count=values[1]["count"] if reports[1] else ""))
    with (out / "resources.csv").open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0])); w.writeheader(); w.writerows(rows)
    (out / "memory-summary.json").write_text(json.dumps(summary, indent=2))
    for case, s in summary.items():
        print(case, {k: (v["allocated_bytes"]/2**20, v["reserved_bytes"]/2**20, v["dxgi_local_bytes"]/2**20) if v else None for k,v in s.items() if k in ["before", "after"]})


if __name__ == "__main__":
    main()
