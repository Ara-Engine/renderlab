"""Crack-flash attribution (CPU) from TR1 mask captures (RENDERLAB_P_STILL=tr1-mask|tr1-raw|tr1-window,
src/temporal_reconstruction.rs mask_resolve) of the clip manifest (every walk frame 160-223).

    py -3 -B tools/p-flicker/mask.py OUT MASK_RUN [RAW_RUN [WINDOW_RUN]]

A flash is a one-frame spike, needing no reference: at frame t the displayed code departs from the
mean of t-1 and t+1 by more than 20 codes while t-1 and t+1 agree within 10. For flash pixels and for
all pixels it reports TR1's continuation class at t (alpha of the mask run), the history weight and
window before the sample (window run), and how the pixel's fresh sample compares with its film and
its 3x3 neighbours' samples (raw run). The mask run's colour is production's film.
"""
import json
from pathlib import Path
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent))
import split  # noqa: E402  (WORK, score.codes, STOPS)

CLASSES = {0: "background", 1: "restart", 2: "still", 3: "catmull-rom", 4: "bilinear 4", 5: "bilinear 1-3",
           6: "spatial start", 7: "rejected"}


def frames(run):
    case = split.WORK / "runs" / run
    info = json.loads((case / "engine.json").read_text())["hdr"]
    w, h, pre = info["width"], info["height"], info["pre_exposure"]
    def load(path):
        raw = np.fromfile(path, "<f2")
        x = raw.reshape(h, raw.size // h)[:, :w * 4].reshape(h, w, 4).astype(np.float32)
        return x[..., :3] / np.float32(pre), x[..., 3]
    return {int(p.stem[4:]): (lambda p=p: load(p)) for p in sorted(case.glob("hdr-*.bin"))}


def label(a):
    a = np.rint(a).astype(np.int32)
    return np.where(a >= 32, 99, a % 16), (a >= 16) & (a < 32)


def histogram(values, mask):
    n = int(mask.sum())
    return {CLASSES.get(int(k), "third surface" if k == 99 else str(k)): round(int(c) / max(n, 1), 4)
            for k, c in zip(*np.unique(values[mask], return_counts=True))}


def neighbour_range(y):
    """Min and max of each pixel's 8 neighbours (edge pixels: their own value repeated)."""
    p = np.pad(y, 1, mode="edge")
    stack = [p[1 + dy:1 + dy + y.shape[0], 1 + dx:1 + dx + y.shape[1]] for dy in (-1, 0, 1) for dx in (-1, 0, 1)
             if dy or dx]
    return np.min(stack, 0), np.max(stack, 0)


def main(out, mask_run, raw_run=None, window_run=None):
    out.mkdir(parents=True, exist_ok=True)
    film = frames(mask_run)
    raw = frames(raw_run) if raw_run else {}
    win = frames(window_run) if window_run else {}
    keys = sorted(film)
    disp = {}
    def codes(i):
        if i not in disp:
            c, a = film[i]()
            disp[i] = (split.score.codes(c, split.STOPS).mean(-1), a)
        return disp[i]
    totals = dict(flash=0, bright=0, pixels=0)
    cls_flash, cls_all, sil_flash, sil_all = [], [], [], []
    prev_cls_flash, next_cls_flash = [], []
    stats = []
    example = None
    for t in keys[1:-1]:
        if t - 1 not in film or t + 1 not in film:
            continue
        a, _ = codes(t - 1)
        b, alpha = codes(t)
        c, _ = codes(t + 1)
        spike = b - (a + c) / 2
        flash = (np.abs(spike) > 20) & (np.abs(c - a) < 10)
        totals["flash"] += int(flash.sum())
        totals["bright"] += int((flash & (spike > 0)).sum())
        totals["pixels"] += flash.size
        k, sil = label(alpha)
        cls_flash.append(k[flash]); sil_flash.append(sil[flash])
        rng = np.random.default_rng(t).random(k.shape) < 0.01
        cls_all.append(k[rng]); sil_all.append(sil[rng])
        prev_cls_flash.append(label(codes(t - 1)[1])[0][flash])
        next_cls_flash.append(label(codes(t + 1)[1])[0][flash])
        if example is None and flash.sum() > 0:
            example = (t, flash, spike)
        row = dict(t=t, flash=int(flash.sum()), bright=int((flash & (spike > 0)).sum()))
        if t in raw:
            s, _ = raw[t]()
            sc = split.score.codes(s, split.STOPS).mean(-1)
            lo, hi = neighbour_range(sc)
            outlier = np.maximum(sc - hi, lo - sc)  # > 0: the sample lies outside its 8 neighbours' range
            row["sample_minus_film_flash"] = float(np.median((sc - b)[flash])) if flash.any() else None
            row["sample_outlier_flash_p50"] = float(np.median(outlier[flash])) if flash.any() else None
            row["sample_outlier_all_p50"] = float(np.median(outlier[rng]))
            row["flash_sample_outlier_gt10"] = float((outlier[flash] > 10).mean()) if flash.any() else None
            row["all_sample_outlier_gt10"] = float((outlier > 10).mean())
            row["flash_same_sign"] = float((np.sign(sc - (a + c) / 2)[flash] == np.sign(spike[flash])).mean()) if flash.any() else None
        if t in win:
            x, _ = win[t]()
            row["weight_flash_p50"] = float(np.median(x[..., 0][flash])) if flash.any() else None
            row["window_flash_p50"] = float(np.median(x[..., 1][flash])) if flash.any() else None
            row["weight_all_p50"] = float(np.median(x[..., 0][rng]))
            row["window_all_p50"] = float(np.median(x[..., 1][rng]))
        stats.append(row)
        print(row, flush=True)
    cat = lambda xs: np.concatenate(xs) if xs else np.zeros(0)
    result = dict(
        run=mask_run, raw=raw_run, window=window_run, frames=len(stats),
        flash_per_frame=totals["flash"] / max(len(stats), 1), bright_share=totals["bright"] / max(totals["flash"], 1),
        class_at_flash=histogram(cat(cls_flash), np.ones(cat(cls_flash).shape, bool)),
        class_all=histogram(cat(cls_all), np.ones(cat(cls_all).shape, bool)),
        class_before_flash=histogram(cat(prev_cls_flash), np.ones(cat(prev_cls_flash).shape, bool)),
        class_after_flash=histogram(cat(next_cls_flash), np.ones(cat(next_cls_flash).shape, bool)),
        silhouette_at_flash=float(cat(sil_flash).mean()) if cat(sil_flash).size else None,
        silhouette_all=float(cat(sil_all).mean()) if cat(sil_all).size else None,
        per_frame=stats)
    (out / f"mask-{mask_run}.json").write_text(json.dumps(result, indent=1) + "\n")
    print(json.dumps({k: v for k, v in result.items() if k != "per_frame"}, indent=1))
    if example:
        t, flash, spike = example
        _, alpha = codes(t)
        k, sil = label(alpha)
        palette = np.array([[0, 0, 0], [255, 0, 255], [40, 40, 40], [0, 90, 0], [0, 0, 200], [230, 160, 0],
                            [255, 0, 0], [0, 230, 230]], np.uint8)
        img = palette[np.clip(np.where(k == 99, 0, k), 0, 7)]
        img[sil] = img[sil] // 2 + 60
        img[flash] = [255, 255, 255]
        Image.fromarray(img).save(out / f"mask-{mask_run}-{t}.png")


if __name__ == "__main__":
    main(Path(sys.argv[1]), *sys.argv[2:])
