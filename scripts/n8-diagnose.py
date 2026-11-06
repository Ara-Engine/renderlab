"""N8b root-cause classifier for the VSM_DEBUG decision traces (docs/non-rt-n8.md, N8b).

Interactive traces the same eight directions with exact hardware rays, so on receivers it
lights fully (256) every direction N8 blocks is a false shadow, and on receivers it
shadows fully (0) every direction N8 lights is a leak. Each wrong direction is attributed
to the decision that produced it, from the per-pixel trace (shaders/virtual_shadow.wgsl):

    py -3 -B scripts/n8-diagnose.py E:/renderlab-work/n8b/diag/quarry [--json out.json]
"""
import argparse
import json
import sys
from pathlib import Path

import numpy as np

# vsm_top_hits outcome bits (trace word 5), OR over a pixel's misses.
WALK = ["exhausted", "open_edge", "degenerate", "back_step", "own_ring", "boundary", "ray_above", "near_drift"]
POP = np.array([bin(i).count("1") for i in range(256)], np.int64)


def bits(word, shift):
    return ((word >> shift) & 0xFF).astype(np.int64)


def dir_count(mask):
    return int(POP[mask].sum())


def histogram(values):
    return {int(k): int(v) for k, v in zip(*np.unique(values, return_counts=True))}


def mask_histogram(masks):
    """Pixels per set bit (placement, or sample index)."""
    return {b: int(((masks >> b) & 1).sum()) for b in range(32) if ((masks >> b) & 1).any()}


def classify(prefix):
    hit = np.fromfile(f"{prefix}-hit.u32", "<u4").reshape(-1, 4)
    rt = np.fromfile(f"{prefix}-interactive.u32", "<u4").astype(np.int64)
    n8 = np.fromfile(f"{prefix}-n8.u32", "<u4").astype(np.int64)
    t = np.fromfile(f"{prefix}-trace.u32x4", "<u4").reshape(-1, 8).astype(np.int64)
    covered = hit[:, 0] != 0
    x, y, z, w = t[:, 0], t[:, 1], t[:, 2], t[:, 3]
    seen_cand, under_miss, seen_top, samples = t[:, 4], t[:, 5], t[:, 6], t[:, 7]
    ring, exact, thick, level = bits(x, 0), bits(x, 8), bits(x, 16), bits(x, 24)
    blocked = ring | exact | thick
    traced = covered & (rt <= 256)
    consistent = (32 * (8 - POP[blocked]) == n8) | ~traced
    out = dict(covered=int(covered.sum()), trace_inconsistent=int((~consistent & covered).sum()),
               equal=int((covered & (rt == n8)).sum()), darker=int((covered & (n8 < rt)).sum()),
               lighter=int((covered & (n8 > rt)).sum()))
    # False shadows: Interactive fully lit.
    lit = covered & (rt == 256) & (n8 < 256)
    in_fp, below, first = bits(z, 0), bits(z, 8), bits(z, 16)
    above_top = bits(w, 24)
    th = np.where(lit, thick, 0)
    out["false_shadow"] = dict(
        pixels=int(lit.sum()),
        pixels_ring=int((lit & (ring != 0)).sum()), pixels_exact=int((lit & (exact != 0)).sum()),
        pixels_thick=int((lit & (thick != 0)).sum()),
        pixels_thick_only=int((lit & (thick != 0) & (ring == 0) & (exact == 0)).sum()),
        directions=dict(ring=dir_count(np.where(lit, ring, 0)), exact=dir_count(np.where(lit, exact, 0)), thick=dir_count(th)),
        thick=dict(inside_footprint=dir_count(th & in_fp), inside_below_plane=dir_count(th & in_fp & below),
                   inside_above_plane=dir_count(th & in_fp & ~below & 0xFF), outside_footprint=dir_count(th & ~in_fp & 0xFF),
                   first_sample=dir_count(th & first), ray_above_texel_top=dir_count(th & above_top)),
        levels={int(k): int(v) for k, v in zip(*np.unique(level[lit], return_counts=True))})
    # Leaks: Interactive fully shadowed.
    umbra = covered & (rt == 0) & (n8 > 0)
    free = np.where(umbra, ~blocked & 0xFF, 0)
    cand, skip, outside, nomarch, under = bits(y, 0), bits(y, 8), bits(y, 16), bits(y, 24), bits(z, 24)
    none_level = umbra & (level == 0xFF)
    free_lv = np.where(~none_level, free, 0)
    out["leak"] = dict(
        pixels=int(umbra.sum()), pixels_no_level=int(none_level.sum()),
        directions=dict(total=dir_count(free), no_level=dir_count(np.where(none_level, free, 0)),
                        no_march=dir_count(free_lv & nomarch), outside_window=dir_count(free_lv & outside & ~nomarch & 0xFF),
                        ring_skip=dir_count(free_lv & skip), candidate_exact_miss=dir_count(free_lv & cand),
                        candidate_inside_below=dir_count(free_lv & cand & under),
                        no_candidate=dir_count(free_lv & ~cand & ~skip & ~nomarch & 0xFF)),
        ring_flags=dict(step_cap=int((umbra & ((w >> 16) & 1).astype(bool)).sum()), open_edge=int((umbra & ((w >> 17) & 1).astype(bool)).sum()),
                        non_manifold=int((umbra & ((w >> 18) & 1).astype(bool)).sum())),
        levels={int(k): int(v) for k, v in zip(*np.unique(level[umbra], return_counts=True))},
        receiver_placements=histogram(hit[umbra, 0] - 1),
        candidate_placements=mask_histogram(seen_cand[umbra]),
        walk_outcomes={name: int(((under_miss[umbra] >> b) & 1).sum()) for b, name in enumerate(WALK)},
        sampled_top_placements=mask_histogram(seen_top[umbra]), candidate_samples=mask_histogram(samples[umbra]))
    partial_light = covered & (rt > 0) & (rt < 256) & (n8 > rt)
    partial_dark = covered & (rt > 0) & (rt < 256) & (n8 < rt)
    out["partial"] = dict(lighter=int(partial_light.sum()), darker=int(partial_dark.sum()),
                          lighter_by_ge_2=int((partial_light & (n8 - rt >= 64)).sum()), darker_by_ge_2=int((partial_dark & (rt - n8 >= 64)).sum()))
    ring_n = w & 0xFFFF
    out["ring_triangles"] = dict(mean=float(ring_n[covered].mean()) if covered.any() else 0, p99=float(np.quantile(ring_n[covered], .99)) if covered.any() else 0)
    return out, dict(lit=lit, umbra=umbra, covered=covered)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("root", type=Path)
    p.add_argument("--json", type=Path)
    a = p.parse_args()
    rows = {}
    for trace in sorted(a.root.glob("*-trace.u32x4")):
        label = trace.name.removesuffix("-trace.u32x4")
        rows[label], _ = classify(str(a.root / label))
        print(label, json.dumps(rows[label]))
    if a.json:
        a.json.write_text(json.dumps(rows, indent=2))


if __name__ == "__main__":
    sys.exit(main())
