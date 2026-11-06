"""N8c: where the lookup's walks go, from a VSM_DEBUG trace (gpu_virtual_quarry_diagnose).

Classifies covered receivers by N8c's result (lit, partial, umbra) and sums shared and
per-direction walk visits and candidates per class.

    py -3 -I scripts/n8c-classify.py <diag dir>/quarry pose-0
"""
import sys
from pathlib import Path

import numpy as np

d, label = Path(sys.argv[1]), sys.argv[2]
hit = np.fromfile(d / f"{label}-hit.u32", np.uint32).reshape(-1, 4)
n8 = np.fromfile(d / f"{label}-n8.u32", np.uint32)
rt = np.fromfile(d / f"{label}-interactive.u32", np.uint32)
trace = np.fromfile(d / f"{label}-trace.u32x4", np.uint32).reshape(-1, 8)
covered = hit[:, 0] != 0
x, y, z, w = trace[:, 0], trace[:, 1], trace[:, 2], trace[:, 3]
ring_blocked = x & 0xff
map_blocked = (x >> 8) & 0xff
shared_visits, dir_visits = y & 0xffff, y >> 16
shared_cand, dir_cand = z & 0xffff, z >> 16
ring_tris = w & 0xffff
classes = {"lit": covered & (n8 == 256), "partial": covered & (n8 > 0) & (n8 < 256), "umbra": covered & (n8 == 0)}
total = {k: 0 for k in ("shared_visits", "dir_visits", "shared_cand", "dir_cand", "ring_tris")}
for name, m in classes.items():
    row = dict(pixels=int(m.sum()), shared_visits=int(shared_visits[m].sum()), dir_visits=int(dir_visits[m].sum()),
               shared_cand=int(shared_cand[m].sum()), dir_cand=int(dir_cand[m].sum()), ring_tris=int(ring_tris[m].sum()),
               ring_walked=int((ring_tris[m] > 1).sum()), ring_blocked_dirs=int(np.unpackbits(ring_blocked[m].astype(np.uint8)).sum()),
               map_blocked_dirs=int(np.unpackbits(map_blocked[m].astype(np.uint8)).sum()))
    for k in total:
        total[k] += row[k]
    print(name, row)
print("total", total, "covered", int(covered.sum()))
# Interactive's classes for reference.
for name, m in {"rt lit": covered & (rt == 256), "rt partial": covered & (rt > 0) & (rt < 256), "rt umbra": covered & (rt == 0)}.items():
    print(name, int(m.sum()))
