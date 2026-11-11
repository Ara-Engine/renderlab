"""Bright-blob detector for frame sequences: run it before any video or sheet leaves.

Reports, per frame, small bright blobs (a few pixels to a few dozen pixels, soft-edged, like a
world-space cache cell showing an unconverged or unclamped value) and edge crawl, and prints
the worst frames. A single-pixel test does not see these: they are soft and several pixels wide.

    py -3 tools/blobs.py FRAMES_DIR [FRAMES_DIR ...] [--baseline N] [--json OUT]

FRAMES_DIR holds frame-NNN.png (game_scene `--every 1`). For each frame:
- blobs: luminance difference of blurs, a 3x3 mean against a 31x31 mean, above +TH codes (a
  bright spot standing out of its neighbourhood); connected regions of 3 to 400 pixels whose
  peak is at least 1.5x the threshold count as blobs. Large features (a lamp, a highlight band)
  exceed the area bound and are ignored; texture rarely forms isolated bright regions.
- blob_px: their total area; the per-frame number to compare.
- crawl: mean absolute change of the edge map (Sobel magnitude) between consecutive frames, a
  rough proxy for edges that shimmer; meaningful only on a still camera or against a reference.
The baseline frame (default 0) is the clean reference; the summary prints each sequence's
median and worst blob_px against it.
"""
import argparse
import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image

TH = 6.0


def box(a, r):
    """Mean over a (2r+1)^2 window, edges clamped, via integral images."""
    p = np.pad(a, r + 1, mode="edge").astype(np.float64)
    c = p.cumsum(0).cumsum(1)
    k = 2 * r + 1
    s = c[k:, k:] - c[:-k, k:] - c[k:, :-k] + c[:-k, :-k]
    return (s / (k * k))[: a.shape[0], : a.shape[1]].astype(np.float32)


def label(mask):
    """Connected regions (4-neighbour) of a boolean mask: (labels, count). Small and dependency-free."""
    h, w = mask.shape
    lab = np.zeros((h, w), np.int32)
    n = 0
    ys, xs = np.nonzero(mask)
    for y0, x0 in zip(ys, xs):
        if lab[y0, x0]:
            continue
        n += 1
        stack = [(y0, x0)]
        lab[y0, x0] = n
        while stack:
            y, x = stack.pop()
            for yy, xx in ((y - 1, x), (y + 1, x), (y, x - 1), (y, x + 1)):
                if 0 <= yy < h and 0 <= xx < w and mask[yy, xx] and not lab[yy, xx]:
                    lab[yy, xx] = n
                    stack.append((yy, xx))
    return lab, n


def analyse(frame):
    a = np.asarray(Image.open(frame).convert("RGB"), np.float32)
    lum = 0.2126 * a[..., 0] + 0.7152 * a[..., 1] + 0.0722 * a[..., 2]
    dog = box(lum, 1) - box(lum, 15)
    mask = dog > TH
    lab, n = label(mask)
    if n == 0:
        return dict(blobs=0, blob_px=0), lum
    area = np.bincount(lab.ravel(), minlength=n + 1)
    peak = np.zeros(n + 1, np.float32)
    np.maximum.at(peak, lab.ravel(), dog.ravel())
    keep = (area >= 3) & (area <= 400) & (peak >= 1.5 * TH)
    keep[0] = False
    return dict(blobs=int(keep.sum()), blob_px=int(area[keep].sum())), lum


def edges(lum):
    gx = np.zeros_like(lum); gy = np.zeros_like(lum)
    gx[:, 1:-1] = lum[:, 2:] - lum[:, :-2]
    gy[1:-1, :] = lum[2:, :] - lum[:-2, :]
    return np.hypot(gx, gy)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("dirs", nargs="+")
    ap.add_argument("--baseline", type=int, default=0)
    ap.add_argument("--json")
    a = ap.parse_args()
    report = {}
    for d in a.dirs:
        frames = sorted(Path(d).glob("frame-*.png"))
        rows, prev = [], None
        for f in frames:
            r, lum = analyse(f)
            e = edges(lum)
            r["crawl"] = round(float(np.abs(e - prev).mean()), 3) if prev is not None else 0.0
            prev = e
            r["frame"] = int(f.stem.split("-")[1])
            rows.append(r)
        px = np.array([r["blob_px"] for r in rows])
        base = next((r["blob_px"] for r in rows if r["frame"] == a.baseline), px[0] if len(px) else 0)
        worst = sorted(rows, key=lambda r: -r["blob_px"])[:5]
        summary = dict(frames=len(rows), baseline_frame=a.baseline, baseline_blob_px=int(base),
                       median_blob_px=int(np.median(px)) if len(px) else 0, max_blob_px=int(px.max()) if len(px) else 0,
                       worst=[(r["frame"], r["blob_px"], r["blobs"]) for r in worst])
        report[d] = dict(summary=summary, frames=rows)
        print(f"{d}: baseline f{a.baseline} {base} px | median {summary['median_blob_px']} px | max {summary['max_blob_px']} px | "
              f"worst frames (frame, px, blobs) {summary['worst']}", flush=True)
    if a.json:
        Path(a.json).write_text(json.dumps(report, indent=1))


if __name__ == "__main__":
    sys.exit(main())
