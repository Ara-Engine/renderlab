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
    p.add_argument("--root", type=Path, default=Path("E:/renderlab-work/m7-u1b"))
    p.add_argument("--reference",default="reference")
    p.add_argument("--suffix",default="")
    p.add_argument("--variants",default="0,1,4,5,6,7,8,9,10,11")
    p.add_argument("--contact-variants",default="0,1,4,6,10")
    a = p.parse_args()
    manifest = json.loads((a.root / a.reference / "manifest.json").read_text())
    size = manifest["size"]
    variants=[int(v) for v in a.variants.split(',')]
    labels=json.loads(next(a.root.glob('software/manifest.json')).read_text())["variants"]
    names = [f"normal-r{r}" for r in (0.08, 0.15, 0.3)] + [f"geometry-r{r}" for r in (0.08, 0.15, 0.3)] + ["flat-control"]
    rows, summary = [], []
    for path in ("software", "hardware"):
        if not (a.root / path / "manifest.json").exists():
            continue
        for track in ("static", "camera", "light"):
            previous = None
            track_rows = []
            for frame in range(1 if track == "static" else 12):
                ref = read(a.root / a.reference / f"{track}-{frame}.f32", size)
                images = [read(a.root / path / f"{track}-{frame}-{v}.f32", size) for v in variants]
                mapped_images=[display(image) for image in images]; mapped_ref=display(ref)
                for patch, name in enumerate(names + ["full-frame"]):
                    mask = patch_mask(size, patch, track, frame) if patch < 7 else np.ones((size[1], size[0]), dtype=bool)
                    if not mask.any():
                        raise ValueError(f"Empty population {name}")
                    for ordinal, image in enumerate(images):
                        variant=variants[ordinal]
                        error = image[mask] - ref[mask]
                        row = dict(path=path, track=track, frame=frame, population=name, variant=variant,
                                   pixels=int(mask.sum()), rms=float(np.sqrt(np.mean(error ** 2))),
                                   mean=float(error.mean()), relative_energy_bias=float(error.sum() / max(ref[mask].sum(), 1e-12)),
                                   display_rms=float(np.sqrt(np.mean((mapped_images[ordinal][mask].astype(float) - mapped_ref[mask].astype(float)) ** 2))) / 255,
                                   peak=float(image[mask].max()), reference_peak=float(ref[mask].max()))
                        if previous is not None:
                            prev_ref, prev_images = previous
                            stable = mask & (patch_mask(size, patch, track, frame - 1) if patch < 7 else mask)
                            residual = ((image - prev_images[ordinal]) - (ref - prev_ref))[stable]
                            row["flicker_rms"] = float(np.sqrt(np.mean(residual ** 2)))
                        rows.append(row)
                        track_rows.append(row)
                previous = (ref, images)
            for name in names + ["full-frame"]:
                for variant in variants:
                    selected = [r for r in track_rows if r["population"] == name and r["variant"] == variant]
                    row = dict(path=path, track=track, population=name, variant=variant,
                               rms=float(np.sqrt(np.mean([r["rms"] ** 2 for r in selected]))),
                               max_abs_energy_bias=max(abs(r["relative_energy_bias"]) for r in selected))
                    flicker = [r["flicker_rms"] for r in selected if "flicker_rms" in r]
                    if flicker:
                        row["flicker_rms"] = float(np.sqrt(np.mean(np.square(flicker))))
                    summary.append(row)
    gates=[]
    for path in ("software","hardware"):
        for variant in variants[1:]:
            failures=[]
            for row in (r for r in rows if r['path']==path and r['variant']==variant):
                baseline=next(r for r in rows if r['path']==path and r['variant']==0 and all(r[k]==row[k] for k in ['track','frame','population']))
                for metric in ['rms','display_rms','relative_energy_bias','flicker_rms']:
                    if metric not in row: continue
                    value=abs(row[metric]) if metric=='relative_energy_bias' else row[metric]
                    control=abs(baseline[metric]) if metric=='relative_energy_bias' else baseline[metric]
                    # Numerical equality only; no quality threshold is relaxed.
                    if value > control + max(1e-7,abs(control)*1e-6):
                        failures.append(dict(track=row['track'],frame=row['frame'],population=row['population'],metric=metric,before=control,candidate=value))
            gates.append(dict(path=path,variant=variant,name=labels[variant],passed=not failures,failures=failures))
    (a.root/f'gates{a.suffix}.json').write_text(json.dumps(gates,indent=2))
    print('GATES',[(g['path'],g['name'],len(g['failures'])) for g in gates])
    (a.root / f"per-frame{a.suffix}.json").write_text(json.dumps(rows, indent=2))
    (a.root / f"summary{a.suffix}.json").write_text(json.dumps(summary, indent=2))
    # Complete images, including silhouettes, and the two specific failure cases.
    columns=[int(v) for v in a.contact_variants.split(',') if int(v) in variants]+[None]
    cell_w,cell_h=size[0],size[1]+24
    combined=Image.new("RGB",(cell_w*len(columns),cell_h*6),(18,18,18))
    draw=ImageDraw.Draw(combined)
    for row,(path,track,frame) in enumerate((p,t,f) for p in ("software","hardware") for t,f in [("static",0),("camera",2),("light",11)]):
        for col,variant in enumerate(columns):
            src=a.root/a.reference/f"{track}-{frame}.f32" if variant is None else a.root/path/f"{track}-{frame}-{variant}.f32"
            combined.paste(Image.fromarray(display(read(src,size))),(col*cell_w,row*cell_h+24))
            draw.text((col*cell_w+3,row*cell_h+4),f"{path} {track}:{frame} / {labels[variant] if variant is not None else a.reference}",fill="white")
    combined.save(a.root/f"contact-sheet{a.suffix}.png")
    for path in ("software", "hardware"):
        for track in ("static", "camera", "light"):
            selected = [r for r in summary if r["path"] == path and r["track"] == track and r["population"] == "full-frame"]
            if selected:
                print(path, track, [(labels[r["variant"]],r["rms"],r.get("flicker_rms")) for r in selected])


if __name__ == "__main__":
    main()
