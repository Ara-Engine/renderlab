"""Score the pre-registered U1 short screen; retain per-frame data and inspected PNGs."""
import argparse
import json
import math
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw


def read(path, size):
    values = np.fromfile(path, dtype="<f4").reshape(size[1], size[0], 4)[..., :3].astype(np.float64)
    if not np.isfinite(values).all():
        raise ValueError(f"Nonfinite output: {path}")
    return values


def patch_mask(size, patch, track, frame):
    w, h = size
    half_y = 4 * math.tan(0.4)
    half_x = half_y * w / h
    x0 = -1.65 + (patch % 3) * 1.1
    y0 = 0.65 - (patch // 3) * 0.65
    camera_x = frame * 0.5 * 4 * math.tan(0.4) * 2 / h if track == "camera" else 0
    x = (np.arange(w) + 0.5 - w / 2) * 2 * half_x / w + camera_x
    y = (h / 2 - np.arange(h) - 0.5) * 2 * half_y / h
    margin = 2 * 2 * half_y / h
    return ((x > x0 + margin) & (x < x0 + 1.05 - margin))[None, :] & ((y > y0 + margin) & (y < y0 + 0.6 - margin))[:, None]


def display(values):
    mapped = np.maximum(values, 0) / (1 + np.maximum(values, 0))
    return (np.where(mapped <= 0.0031308, 12.92 * mapped, 1.055 * mapped ** (1 / 2.4) - 0.055) * 255).round().clip(0, 255).astype(np.uint8)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--root", type=Path, default=Path("E:/renderlab-work/m7-u1"))
    a = p.parse_args()
    manifest = json.loads((a.root / "reference/manifest.json").read_text())
    size = manifest["size"]
    names = [f"normal-r{r}" for r in (0.08, 0.15, 0.3)] + [f"geometry-r{r}" for r in (0.08, 0.15, 0.3)] + ["flat-control"]
    rows, summary, sheets = [], [], []
    for path in ("software", "hardware"):
        if not (a.root / path / "manifest.json").exists():
            continue
        for track in ("static", "camera", "light"):
            previous = None
            track_rows = []
            for frame in range(1 if track == "static" else 12):
                ref = read(a.root / "reference" / f"{track}-{frame}.f32", size)
                images = [read(a.root / path / f"{track}-{frame}-{v}.f32", size) for v in range(2)]
                for patch, name in enumerate(names + ["full-frame"]):
                    mask = patch_mask(size, patch, track, frame) if patch < 7 else np.ones((size[1], size[0]), dtype=bool)
                    if not mask.any():
                        raise ValueError(f"Empty population {name}")
                    for variant, image in enumerate(images):
                        error = image[mask] - ref[mask]
                        row = dict(path=path, track=track, frame=frame, population=name, variant=variant,
                                   pixels=int(mask.sum()), rms=float(np.sqrt(np.mean(error ** 2))),
                                   mean=float(error.mean()), relative_energy_bias=float(error.sum() / max(ref[mask].sum(), 1e-12)),
                                   display_rms=float(np.sqrt(np.mean((display(image)[mask].astype(float) - display(ref)[mask].astype(float)) ** 2))) / 255,
                                   peak=float(image[mask].max()), reference_peak=float(ref[mask].max()))
                        if previous is not None:
                            prev_ref, prev_images = previous
                            stable = mask & (patch_mask(size, patch, track, frame - 1) if patch < 7 else mask)
                            residual = ((image - prev_images[variant]) - (ref - prev_ref))[stable]
                            row["flicker_rms"] = float(np.sqrt(np.mean(residual ** 2)))
                        rows.append(row)
                        track_rows.append(row)
                if frame in (0, 6, 11):
                    sheet = Image.new("RGB", (size[0] * 3, size[1] + 24), "#171717")
                    draw = ImageDraw.Draw(sheet)
                    for i, (label, image) in enumerate(zip(("Before", "Candidate", "256-subframe reference"), images + [ref])):
                        sheet.paste(Image.fromarray(display(image)), (size[0] * i, 24))
                        draw.text((size[0] * i + 4, 4), f"{path} {track} {frame}: {label}", fill="white")
                    sheet.save(a.root / f"{path}-{track}-{frame}.png")
                    if frame == 0:
                        sheets.append(sheet)
                previous = (ref, images)
            for name in names + ["full-frame"]:
                for variant in range(2):
                    selected = [r for r in track_rows if r["population"] == name and r["variant"] == variant]
                    row = dict(path=path, track=track, population=name, variant=variant,
                               rms=float(np.sqrt(np.mean([r["rms"] ** 2 for r in selected]))),
                               max_abs_energy_bias=max(abs(r["relative_energy_bias"]) for r in selected))
                    flicker = [r["flicker_rms"] for r in selected if "flicker_rms" in r]
                    if flicker:
                        row["flicker_rms"] = float(np.sqrt(np.mean(np.square(flicker))))
                    summary.append(row)
    (a.root / "per-frame.json").write_text(json.dumps(rows, indent=2))
    (a.root / "summary.json").write_text(json.dumps(summary, indent=2))
    if sheets:
        combined = Image.new("RGB", (sheets[0].width, sum(s.height for s in sheets)), "black")
        y = 0
        for sheet in sheets:
            combined.paste(sheet, (0, y)); y += sheet.height
        combined.save(a.root / "contact-sheet.png")
    for path in ("software", "hardware"):
        for track in ("static", "camera", "light"):
            selected = [r for r in summary if r["path"] == path and r["track"] == track and r["population"] == "full-frame"]
            if selected:
                print(path, track, json.dumps(selected))


if __name__ == "__main__":
    main()
