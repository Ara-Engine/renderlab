"""VF2 contact sheets and linear-HDR profile diagnostics; never runs a renderer."""
import json
from pathlib import Path
import sys
from PIL import Image, ImageDraw

ROOT = Path("D:/Coding/renderlab-work/vf2/images")


def sheet(rows, output, width=480):
    height = int(width * 9 / 16)
    canvas = Image.new("RGB", (width * max(map(len, rows)), (height + 25) * len(rows)))
    draw = ImageDraw.Draw(canvas)
    for y, row in enumerate(rows):
        for x, (label, path) in enumerate(row):
            image = Image.open(path).convert("RGB")
            image.thumbnail((width, height))
            canvas.paste(image, (x * width, y * (height + 25) + 25))
            draw.text((x * width + 5, y * (height + 25) + 5), label, fill="white")
    canvas.save(output)


def report(label, windows=False):
    root = ROOT / label
    prefix="windows-" if windows else ""
    suffix="-windows" if windows else ""
    summary = {}
    for path in ["nonrt", "rt"]:
        folder = root / ("fixture-" + path + suffix)
        if not (folder / "result.json").exists():
            continue
        data = json.loads((folder / "result.json").read_text())
        summary[path] = {"timing_ms": data["timing_ms"], "ablation_timing_ms":data.get("ablation_timing_ms"), "profiles": {}}
        for name, value in data["configs"].items():
            row = [sum(p) / 3 for p in value["midrow_exposed_rgb"]]
            ordered = sorted(row)
            q10, q90 = ordered[len(row) // 10], ordered[len(row) * 9 // 10]
            summary[path]["profiles"][name] = {"p10": q10, "p90": q90, "range": q90 - q10}
        a = data["configs"]["motion-23"]["midrow_exposed_rgb"]
        b = data["configs"]["motion-31"]["midrow_exposed_rgb"]
        summary[path]["held_camera_midrow_mean_abs_change"] = sum(abs(x-y) for p,q in zip(a,b) for x,y in zip(p,q)) / (3*len(a))
    for name, configs in [("fixture",["off","default","dense-g06","dense-g08","dense-nohistory"]),("ablation",["coarse-one","fine-one","coarse-two","fine-two"]),("angle",["angle-default","angle-coarse-one","angle-dense-g06","angle-dense-g08"]),("angle-motion",["angle-motion-00","angle-motion-07","angle-motion-15","angle-motion-23"]),("angle-sun",["angle-sun-00","angle-sun-07","angle-sun-15","angle-sun-23"]),("motion",["motion-00","motion-07","motion-15","motion-16","motion-23","motion-31"]),("sun",["sun-00","sun-07","sun-15","sun-23"])]:
        rows = [[(path+" "+c,root/("fixture-"+path+suffix)/(c+".png")) for c in configs] for path in ["nonrt","rt"]]
        if all(p.exists() for row in rows for _,p in row):sheet(rows,root/(prefix+name+"-sheet.png"),320)
    rows=[[(path+" "+c,root/path/(c+".png")) for c in ["default","low-default","low-dense","motion-15"]] for path in ["nonrt","traced"]]
    if not windows and all(p.exists() for row in rows for _,p in row):sheet(rows,root/"quarry-sheet.png")
    if not windows:
        for name, configs in [("quarry-comparison", ["default", "low-default", "low-dense"]), ("quarry-motion", ["motion-00", "motion-07", "motion-15", "motion-23"])]:
            rows = [[(path+" "+c, root/path/(c+".png")) for c in configs] for path in ["baseline-nonrt", "nonrt", "baseline-traced", "traced"]]
            if all(p.exists() for row in rows for _,p in row):sheet(rows,root/(name+"-sheet.png"))
    (root/(prefix+"summary.json")).write_text(json.dumps(summary,indent=2))
    print(label, prefix, {path: data["timing_ms"] for path, data in summary.items()})


if __name__ == "__main__":
    report(sys.argv[1])
    if (ROOT/sys.argv[1]/"fixture-nonrt-windows").exists():report(sys.argv[1],True)
