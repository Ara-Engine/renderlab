"""U-SPECK: gather hit cells sized to the ray's cone (IndirectSettings::hit_cone_cells), judged
against the path tracer on the field walk.

    py -3 -B tools/speck/cone.py run MATCHED_EXE OUT [--spp 16384]     # inside a lease
    py -3 -B tools/speck/cone.py run-flight MATCHED_EXE OUT [--spp 4096]   # the 48-frame flight
    py -3 -B tools/speck/cone.py report OUT                            # CPU

`run` path-traces the matched field walk (examples/reflection_matched.rs `--field`: uniform
sky, sun, 300 boxes and a slab, the 4 m walk over 120 frames, 1280x720) at frames 60 and 119,
then renders it on both paths with hit_cone_cells 0 (every hit at the pixels' level), 16, 8
and 4, writing frames 0, 30, 60, 90 and 119 and the cache's counters (gi.jsonl). Finished
runs are skipped.

`report` prints, per path and setting: the render against the reference at frames 60 and 119
(mean and RMS of the difference of 80 px region means, in 8-bit display codes, and the largest
region), the same against the setting-0 render, the cell mottle on the open ground (band-pass
RMS), and the cells created, evicted under pressure and the pixels' failed inserts.
"""
import json
import os
import subprocess
import sys
import time
from pathlib import Path

import numpy as np
from PIL import Image
from scipy import ndimage as ndi

REPO = Path(__file__).resolve().parents[2]
WORK = Path("D:/Coding/renderlab-work/unit-speck")
CELLS = (0, 16, 8, 4)
FRAMES = (60, 119)
FLIGHT_FRAMES = (16, 32, 47)
LUM = np.array([0.2126, 0.7152, 0.0722])


def run(exe, out, spp, flight=False):
    (WORK / "tmp").mkdir(parents=True, exist_ok=True)
    env = {k: v for k, v in os.environ.items() if k not in ("VK_INSTANCE_LAYERS", "VK_LAYER_PATH", "RENDERLAB_ABLATE", "RENDERLAB_GI")}
    env |= {"RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK": "I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK", "TMP": str(WORK / "tmp"), "TEMP": str(WORK / "tmp")}
    # The field walk, or (`run-flight`) the example's own 48-frame flight past the spinning crate.
    scene = ["--frames", "48"] if flight else ["--field", "--frames", "120"]
    frames = FLIGHT_FRAMES if flight else FRAMES
    written = ",".join(map(str, frames)) if flight else "0,30,60,90,119"
    common = [*scene, "--size", "1280x720", "--pipeline-cache", str(WORK / "cache")]
    steps = [("reference", ["--path", "rt", "--reference", str(spp), "--write-frames", ",".join(map(str, frames))], {})]
    for path in ("rt", "nonrt"):
        for cells in CELLS:
            steps.append((f"{path}-cone{cells}", ["--path", path, "--matched", "--write-frames", written],
                          {"RENDERLAB_GI": f"hit_cone_cells={cells}"}))
    worst = 0
    for name, args, extra in steps:
        folder = out / name
        if (folder / "done.json").exists():
            continue
        folder.mkdir(parents=True, exist_ok=True)
        start = time.monotonic()
        r = subprocess.run([exe, *args, *common, "--out", str(folder)], cwd=REPO, capture_output=True, text=True, timeout=900,
                           env=env | extra | {"RENDERLAB_GI_LOG": str(folder / "gi.jsonl")})
        print(f"{name}: exit {r.returncode}, {time.monotonic() - start:.0f} s", flush=True)
        if r.returncode == 0:
            (folder / "done.json").write_text(json.dumps(dict(exe=exe, tail=(r.stdout + r.stderr)[-400:])))
        else:
            print((r.stdout + r.stderr)[-1500:], flush=True)
        worst = max(worst, r.returncode)
    return worst


def image(folder, frame):
    return np.asarray(Image.open(folder / f"frame-{frame:03d}.png").convert("RGB"), np.float64) @ LUM


def regions(a, tile=80):
    rows, cols = a.shape[0] // tile, a.shape[1] // tile
    return a[:rows * tile, :cols * tile].reshape(rows, tile, cols, tile).mean(axis=(1, 3))


def changed(out, frames):
    """Where a setting moves the picture: for the 80 px regions a setting changes by more than
    0.75 codes against setting 0, both renders' mean distance from the reference there."""
    for path in ("rt", "nonrt"):
        for cells in CELLS[1:]:
            for frame in frames:
                ref, base, ours = (regions(image(out / n, frame)) for n in ("reference", f"{path}-cone0", f"{path}-cone{cells}"))
                moved = np.abs(ours - base) > 0.75
                if moved.any():
                    print(f"{path} cells {cells:2d} f{frame}: {int(moved.sum())} regions moved, by {np.abs(ours - base)[moved].mean():.2f} codes on average; "
                          f"distance from the reference there: cells 0 {np.abs(base - ref)[moved].mean():.2f}, cells {cells} {np.abs(ours - ref)[moved].mean():.2f}")
                else:
                    print(f"{path} cells {cells:2d} f{frame}: no region moved by more than 0.75 codes")


def report(out):
    if (out / "reference" / f"frame-{FLIGHT_FRAMES[0]:03d}.png").exists():
        return changed(out, FLIGHT_FRAMES)
    for path in ("rt", "nonrt"):
        for cells in CELLS:
            folder = out / f"{path}-cone{cells}"
            rows = [json.loads(line) for line in (folder / "gi.jsonl").read_text().splitlines() if line.strip()]
            total = lambda key: sum(r.get(key, 0) for r in rows)
            line = f"{path} cells {cells:2d}: created {total('created'):8d}, evicted {total('evicted'):7d}, pixels' failed {total('full_pixel'):7d}, gather queue at the end {rows[-1]['bounce_queued']:7d}"
            for frame in FRAMES:
                ours, ref, base = image(folder, frame), image(out / "reference", frame), image(out / f"{path}-cone0", frame)
                d = regions(ours) - regions(ref)
                b = regions(ours) - regions(base)
                # The open ground at the bottom left of the frame (no box, no slab).
                crop = (ndi.uniform_filter(ours, 3) - ndi.uniform_filter(ours, 31))[560:700, 40:420]
                line += (f" | f{frame} vs reference mean {d.mean():+.2f} RMS {np.sqrt((d ** 2).mean()):.2f} max {np.abs(d).max():.2f};"
                         f" vs cells 0 RMS {np.sqrt((b ** 2).mean()):.2f} max {np.abs(b).max():.2f}; mottle {np.sqrt((crop ** 2).mean()):.3f}")
            print(line)
    for frame in FRAMES:
        a, b = (np.asarray(Image.open(out / "reference" / f"frame-{frame:03d}-{h}.png").convert("RGB"), np.float64) @ LUM for h in "ab")
        d = (regions(a) - regions(b)) / 2
        print(f"reference f{frame}: its own noise by region RMS {np.sqrt((d ** 2).mean()):.2f} codes")


if __name__ == "__main__":
    if sys.argv[1] in ("run", "run-flight"):
        spp = int(sys.argv[sys.argv.index("--spp") + 1]) if "--spp" in sys.argv else 16384
        raise SystemExit(run(sys.argv[2], Path(sys.argv[3]), spp, sys.argv[1] == "run-flight"))
    report(Path(sys.argv[2]))
