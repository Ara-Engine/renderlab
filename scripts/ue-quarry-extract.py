"""Decode Unreal's own EXRs and retain raw direct ratios for later Level-B scoring.

Uses the existing calibration's isolated OpenEXR dependency directory. No gain,
alignment, resampling, clipping or reference synthesis is performed.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import sys

import numpy as np
from PIL import Image

ROOT = Path("D:/Coding/renderlab-work/ue-m5/Quarry")
DEPENDENCIES = Path("D:/Coding/projects/desktop/renderlab/.refs/m5-m6/ue-runtime/python")
SOURCE = Path("D:/Coding/renderlab-work/m6-compute-cost/runs/quarry-c3r")


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def display(rgb, stops):
    linear = np.maximum(rgb, 0) * 2. ** stops
    mapped = linear / (1 + linear)
    srgb = np.where(mapped <= .0031308, 12.92 * mapped, 1.055 * mapped ** (1 / 2.4) - .055)
    return np.clip(np.rint(255 * srgb), 0, 255).astype("uint8")


def extract(run, output, dependencies):
    sys.path.insert(0, str(dependencies))
    import OpenEXR
    spec = importlib.util.spec_from_file_location("quarry_score", Path(__file__).with_name("ue-quarry-score.py"))
    score = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(score)
    capture = json.loads((run / "capture.json").read_text())
    output.mkdir(parents=True, exist_ok=False)
    cache = {}
    def exr(record):
        path = run / record["path"]
        if digest(path) != record["sha256"]:
            raise ValueError("EXR bytes changed")
        if path not in cache:
            with OpenEXR.File(str(path), separate_channels=True) as image:
                c = image.channels()
                rgb = np.stack([c[k].pixels for k in "RGB"], -1)
                if rgb.shape != (1080, 1920, 3) or rgb.dtype != np.float32 or not np.isfinite(rgb).all():
                    raise ValueError("expected finite float32 1080p linear RGB")
                alpha = c["A"].pixels.copy()
                if alpha.shape != (1080, 1920) or not np.isfinite(alpha).all():
                    raise ValueError("invalid inverse-opacity channel")
            cache[path] = rgb, alpha
        return cache[path]
    results = []
    for record in capture["captures"]:
        if not record["casts_shadows"]:
            continue
        pose = record["pose"]
        unshadowed = next(r for r in capture["captures"] if r["pose"] == pose and not r["casts_shadows"])
        a, alpha_a = exr(record)
        b, alpha_b = exr(unshadowed)
        depth_path = SOURCE / f"pose-{pose}-depth.f32"
        depth = np.fromfile(depth_path, dtype="<f4").reshape(1080, 1920)
        source_cover = depth > 0
        ue_cover = (alpha_a < .5) & (alpha_b < .5)
        # Source-only pixels stay undefined rather than borrowing a UE neighbour.
        direct_a, direct_b = a.copy(), b.copy()
        direct_a[~ue_cover] = np.nan
        visibility, valid, extraction = score.visibility(direct_a, direct_b, source_cover)
        label = f"pose-{pose}-{record['state']}"
        np.savez(output / f"{label}-direct.npz", shadowed=a, unshadowed=b, visibility=visibility,
                 valid=valid, source_cover=source_cover, ue_cover=ue_cover, depth=depth)
        gray = np.repeat(np.clip(np.nan_to_num(visibility), 0, 1)[..., None], 3, -1)
        image = np.rint(255 * gray).astype("uint8")
        image[source_cover & ~valid] = [255, 0, 255]
        Image.fromarray(image).save(output / f"{label}-visibility.png")
        for name, data in (("shadowed", a), ("unshadowed", b)):
            Image.fromarray(display(data, -14)).save(output / f"{label}-{name}-display.png")
        results.append(dict(pose=pose, state=record["state"], extraction=extraction,
                            source_cover=int(source_cover.sum()), ue_cover=int(ue_cover.sum()),
                            source_only=int((source_cover & ~ue_cover).sum()), ue_only=int((ue_cover & ~source_cover).sum()),
                            source_depth_sha256=digest(depth_path), shadowed_sha256=record["sha256"],
                            unshadowed_sha256=unshadowed["sha256"], raw_npz_sha256=digest(output / f"{label}-direct.npz")))
    manifest = dict(status="RAW_VISIBILITY_EXTRACTED_NOT_SCORED", capture=str(run / "capture.json"),
                    capture_sha256=digest(run / "capture.json"), mode=capture["shadow_mode"], pairs=results,
                    reference="Bind separately to the immutable 16,384-ray reference; never score against one shipping draw",
                    scale="unexposed linear RGB as exported; paired ratio cancels a common scale; no fitted gain",
                    display="-14 stops, per-channel Reinhard then sRGB; magenta denotes undefined source receiver",
                    limits="Requires independent projection/coverage, material and pre-exposure audit; no Level B verdict")
    (output / "extraction.json").write_text(json.dumps(manifest, indent=2, allow_nan=False) + "\n")
    print(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run")
    parser.add_argument("--dependencies", type=Path, default=DEPENDENCIES)
    args = parser.parse_args()
    if not args.run or any(c not in "abcdefghijklmnopqrstuvwxyz0123456789-" for c in args.run):
        parser.error("expected a private run label")
    extract(ROOT / "runs" / args.run, ROOT / "extracted" / args.run, args.dependencies)
