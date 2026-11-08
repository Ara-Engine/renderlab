"""PP1 CPU-only contact sheets and timing medians from the effect sweep."""
import json
from pathlib import Path
import statistics
import sys
from PIL import Image, ImageDraw

root = Path(sys.argv[1])
summary = {}
for path in ("rt", "nonrt", "raster"):
    for scene in ("quarry", "bright-sky"):
        directory = root / path / scene
        if not directory.exists():
            continue
        data = json.loads((directory / "timing.json").read_text())
        values = {name: statistics.median(f["post_tone_ms"] for f in row["frames"]
                    if f["post_tone_ms"] is not None) for name, row in data.items()}
        summary[f"{path}/{scene}"] = {name: {"post_tone_median_ms": v,
            "delta_to_neutral_ms": v-values["neutral"],"frames":len(data[name]["frames"])} for name,v in values.items()}
        for state in ("still", "moving"):
            names = ["off", "default", "bloom", "motion", "dof", "dof-near", "filmic", "grade", "lut", "vignette", "grain", "aberration", "neutral"]
            names = [n for n in names if (directory / f"{n}-{state}.png").exists()]
            sheet = Image.new("RGB", (960, ((len(names)+1)//2)*296), "#121519")
            draw = ImageDraw.Draw(sheet)
            for i, name in enumerate(names):
                im = Image.open(directory / f"{name}-{state}.png").convert("RGB")
                im.thumbnail((480,270))
                x,y = (i%2)*480,(i//2)*296
                sheet.paste(im,(x,y+24))
                draw.text((x+8,y+5),f"{path} {scene} {name} {state}",fill="white")
            sheet.save(directory / f"inspect-{state}.png")
(root / "summary.json").write_text(json.dumps(summary,indent=2))
print(json.dumps(summary,indent=2))
