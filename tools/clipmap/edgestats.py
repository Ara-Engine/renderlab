"""Wobble and 90-10 width of a straight shadow edge in a crop of per-pixel sun visibility (lit on
the left of each row). Feed it a replay's .npy (replay.py) or a RENDERLAB_VD_DUMP frame
(vis-<frame>-<w>x<h>.bin: one u32 a pixel, 256 lit; divide by 256).

stats(crop) -> (wobble: rms px of each row's 50% crossing about a quadratic fit,
                median 90-10 width px, dominant period of the wobble px, rows measured)
"""
import numpy as np


def stats(c):
    px = []
    widths = []
    for r in c:
        if np.isnan(r).any():
            continue
        idx = np.where((r[:-1] >= 0.5) & (r[1:] < 0.5))[0]
        if not len(idx):
            continue
        i = idx[0]
        px.append(i + (r[i] - 0.5) / (r[i] - r[i + 1]))
        lo = np.where(r < 0.1)[0]
        a = np.where(r > 0.9)[0]
        if len(lo):
            a = a[a < lo[0]]
            if len(a):
                widths.append(lo[0] - a[-1])
    px = np.array(px)
    yy = np.arange(len(px))
    res = px - np.polyval(np.polyfit(yy, px, 2), yy)
    sp = np.abs(np.fft.rfft(res - res.mean()))
    fr = np.fft.rfftfreq(len(res))
    k = np.argmax(sp[3:]) + 3
    return res.std(), np.median(widths), 1 / fr[k], len(px)
