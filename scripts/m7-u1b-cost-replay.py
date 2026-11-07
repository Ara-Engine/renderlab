"""Reject spatial cost ideas cheaply against saved U1 frames; GPU remains the gate."""
import importlib.util
import json
import math
from pathlib import Path

import numpy as np

spec = importlib.util.spec_from_file_location("replay", Path(__file__).with_name("m7-u1b-replay.py"))
replay = importlib.util.module_from_spec(spec)
spec.loader.exec_module(replay)
ROOT = Path("E:/renderlab-work/m7-u1b/cost-iteration-1")
OLD = Path("E:/renderlab-work/m7-u1")


def main():
    maps = replay.textures()
    rows = []
    size = [320, 180]
    pitch = 8 * math.tan(.4) / size[1]
    lod = math.log2(512 * pitch / .6)
    for track in ("static", "camera", "light"):
        previous = {}
        for frame in range(1 if track == "static" else 12):
            ref = replay.score.read(OLD / "reference" / f"{track}-{frame}.f32", size)
            before = replay.score.read(OLD / "software" / f"{track}-{frame}-0.f32", size)
            eye_x = frame * .5 * pitch if track == "camera" else 0
            eye = np.array([eye_x, 0, 4.])
            light = replay.unit(np.array([frame * .005 if track == "light" else 0, .05, 1.]))
            for patch, rough in enumerate((.08, .15, .3)):
                mask = replay.score.patch_mask(size, patch, track, frame)
                ys, xs = np.nonzero(mask)
                world = np.column_stack(((xs + .5 - 160) * pitch + eye_x, (90 - ys - .5) * pitch, np.zeros(len(xs))))
                uv = (world[:, :2] - [-1.65 + patch * 1.1, .65]) / [1.05, .6]
                value = replay.sample(maps[0], uv, lod)
                spread = value[:, 3] ** 2
                # CPU uses the same fixed, centred grids as the GPU experiments.
                images = {"before": before.copy()}
                cache = {}
                for side in (2, 4, 8, 16):
                    image = np.zeros((len(xs), 3))
                    for y in range(side):
                        for x in range(side):
                            offset = (np.array([x, y]) + .5) / side - .5
                            q = world + [offset[0] * pitch, -offset[1] * pitch, 0]
                            u = uv + offset * [pitch / 1.05, -pitch / .6]
                            n = replay.unit(replay.sample(maps[0], u, max(lod - 4, 0))[:, :3] * 2 - 1)
                            image += replay.brdf(n, replay.unit(eye - q), light, rough) / (side * side)
                    cache[side] = image.astype(np.float16).astype(float)
                    full = np.zeros_like(ref); full[mask] = cache[side]
                    images[f"spatial-{side * side}-v3"] = full
                for factor in (4, 8):
                    desired = np.clip(factor * np.sqrt(spread / max(rough ** 4, 1e-8)), 1, 16)
                    side = 2 ** np.clip(np.ceil(np.log2(desired)), 0, 4).astype(int)
                    full = before.copy()
                    for n in (2, 4, 8, 16):
                        p = side == n
                        full[ys[p], xs[p]] = cache[n][p]
                    images[f"adaptive-{factor}-v3"] = full
                for name, image in images.items():
                    error = image[mask] - ref[mask]
                    row = dict(track=track, frame=frame, population=patch, variant=name,
                               rms=float(np.sqrt(np.mean(error ** 2))),
                               bias=float(error.sum() / ref[mask].sum()))
                    key = (patch, name)
                    if key in previous:
                        pi, pr, pm = previous[key]
                        stable = mask & pm
                        row["flicker"] = float(np.sqrt(np.mean(((image - pi) - (ref - pr))[stable] ** 2)))
                    previous[key] = image, ref, mask
                    rows.append(row)
    failures = {}
    for r in rows:
        if r["variant"] == "before":
            continue
        b = next(b for b in rows if b["variant"] == "before" and all(b[k] == r[k] for k in ("track", "frame", "population")))
        for k in ("rms", "bias", "flicker"):
            if k in r and abs(r[k]) > abs(b[k]) + max(1e-7, abs(b[k]) * 1e-6):
                failures.setdefault(r["variant"], []).append(dict(**r, metric=k, before=b[k]))
    (ROOT / "cpu-cost-replay.json").write_text(json.dumps(dict(rows=rows, failures=failures), indent=2))
    print({k:len(v) for k,v in failures.items()})


if __name__ == "__main__":
    main()
