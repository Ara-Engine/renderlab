"""Compose SK1's inspected frames and diagnostic chart from its retained JSON/PNGs."""
import json
from pathlib import Path
import statistics
import sys

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from PIL import Image, ImageDraw

out = Path(sys.argv[1] if len(sys.argv) > 1 else "D:/Coding/renderlab-work/m7-sk1/images")
frames = [out / f"{path}-{scene}-{frame:02}.png" for path in ("nonrt", "rt")
          for scene in ("fixture", "quarry") for frame in (0, 8, 16, 24, 31)]
width, height = 480, 270
sheet = Image.new("RGB", (width * 5, (height + 24) * 4), "#14171a")
draw = ImageDraw.Draw(sheet)
for index, path in enumerate(frames):
    x, y = index % 5 * width, index // 5 * (height + 24)
    sheet.paste(Image.open(path).convert("RGB").resize((width, height)), (x, y + 24))
    draw.text((x + 8, y + 5), path.stem, fill="white")
sheet.save(out / "contact-sheet.png")

rows = json.loads((out / "summary.json").read_text())
labels = [f"{row['scene']} / {row['path']}" for row in rows]
figure, axes = plt.subplots(1, 2, figsize=(12, 3.6), layout="constrained")
colours = ["#258987", "#258987", "#9065b0", "#9065b0"]
axes[0].barh(labels, [r["synchronized_frame_wall_mean_ms"] for r in rows], color=colours)
axes[0].set_xlabel("Milliseconds; pose submission through explicit GPU drain")
axes[0].set_title("Synchronized complete-frame wall time")
samples = [[v * 1000 for v in r["skinning_gpu_component_samples_ms"]] for r in rows]
axes[1].barh(labels, [statistics.mean(v) for v in samples],
             xerr=[statistics.stdev(v) for v in samples], color=colours, capsize=3)
axes[1].set_xlabel("Microseconds; mean ± sample SD of 8 warmed dispatches")
axes[1].set_title("Skin compute + cluster copies (no upload or AS)")
for axis in axes:
    axis.invert_yaxis()
    axis.spines[["top", "right"]].set_visible(False)
    axis.grid(axis="x", alpha=0.15)
    axis.set_axisbelow(True)
figure.suptitle("SK1 development-build diagnostics · contended · different shadow methods")
figure.savefig(out / "timings.png", dpi=160)
plt.close(figure)
print(out / "contact-sheet.png")
print(out / "timings.png")
