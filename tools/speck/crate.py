"""U-SPECK a2: the spinning crate of the matched flight against the path tracer, per GI setting.

    py -3 -B tools/speck/crate.py run MATCHED_EXE OUT NAME=GI [NAME=GI ...]   # inside a lease
    py -3 -B tools/speck/crate.py report OUT [SHEET.png]                      # CPU

`run` path-traces examples/reflection_matched.rs's 48-frame flight at frames 16, 32 and 47
(8,192 samples, 1280x720) once, then renders it on both paths for each NAME with
RENDERLAB_GI=GI (`default=` is the engine's own settings). Finished runs are skipped.

`report` finds the crate as the 80 px regions where the reference changes most between the
three frames' neighbours... no: it takes the regions the `default` render is farthest from
the reference in (the worst four a frame, which are the crate's), and prints for every NAME
the mean distance from the reference there and the RMS over the whole frame, in 8-bit codes.
With SHEET it writes the crate's crop at frame 47: reference, then each NAME on RT.
"""
import json
import os
import subprocess
import sys
import time
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

REPO = Path(__file__).resolve().parents[2]
WORK = Path("D:/Coding/renderlab-work/unit-speck")
FRAMES = (16, 32, 47)
LUM = np.array([0.2126, 0.7152, 0.0722])
TILE = 80


def run(exe, out, variants):
    (WORK / "tmp").mkdir(parents=True, exist_ok=True)
    env = {k: v for k, v in os.environ.items() if k not in ("VK_INSTANCE_LAYERS", "VK_LAYER_PATH", "RENDERLAB_ABLATE", "RENDERLAB_GI")}
    env |= {"RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK": "I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK", "TMP": str(WORK / "tmp"), "TEMP": str(WORK / "tmp")}
    written = ",".join(map(str, FRAMES))
    common = ["--frames", "48", "--size", "1280x720", "--write-frames", written, "--pipeline-cache", str(WORK / "cache")]
    steps = [("reference", ["--path", "rt", "--reference", "8192"], {})]
    for name, gi in variants:
        for path in ("rt", "nonrt"):
            # A NAME beginning `freeze` holds the scene and camera at frame 47's time (`--freeze 47`):
            # the same picture with nothing moving.
            held = ["--freeze", "47"] if name.startswith("freeze") else []
            steps.append((f"{path}-{name}", ["--path", path, "--matched", *held], {"RENDERLAB_GI": gi} if gi else {}))
    worst = 0
    for name, args, extra in steps:
        folder = out / name
        if (folder / "done.json").exists():
            continue
        folder.mkdir(parents=True, exist_ok=True)
        start = time.monotonic()
        r = subprocess.run([exe, *args, *common, "--out", str(folder)], cwd=REPO, capture_output=True, text=True, timeout=900, env=env | extra)
        print(f"{name}: exit {r.returncode}, {time.monotonic() - start:.0f} s", flush=True)
        if r.returncode == 0:
            (folder / "done.json").write_text(json.dumps(dict(exe=exe, gi=extra.get("RENDERLAB_GI", ""))))
        else:
            print((r.stdout + r.stderr)[-1500:], flush=True)
        worst = max(worst, r.returncode)
    return worst


def regions(folder, frame):
    a = np.asarray(Image.open(folder / f"frame-{frame:03d}.png").convert("RGB"), np.float64) @ LUM
    rows, cols = a.shape[0] // TILE, a.shape[1] // TILE
    return a[:rows * TILE, :cols * TILE].reshape(rows, TILE, cols, TILE).mean(axis=(1, 3))


def report(out, sheet=None):
    names = sorted({p.name.split("-", 1)[1] for p in out.iterdir() if p.name.startswith("rt-") and (p / "done.json").exists()},
                   key=lambda n: (n != "default", n))
    crate = {}
    for path in ("rt", "nonrt"):
        for frame in FRAMES:
            d = np.abs(regions(out / f"{path}-default", frame) - regions(out / "reference", frame))
            crate[path, frame] = d >= np.sort(d.ravel())[-4]
    for name in names:
        for path in ("rt", "nonrt"):
            parts = []
            for frame in FRAMES:
                d = regions(out / f"{path}-{name}", frame) - regions(out / "reference", frame)
                m = crate[path, frame]
                parts.append(f"f{frame} crate {np.abs(d[m]).mean():5.2f} (signed {d[m].mean():+5.2f}), frame RMS {np.sqrt((d ** 2).mean()):.2f}")
            print(f"{path:5} {name:14}: " + " | ".join(parts))
    if sheet:
        m = crate["rt", 47]
        rows, cols = np.nonzero(m)
        box = (max(cols.min() * TILE - 40, 0), max(rows.min() * TILE - 40, 0), (cols.max() + 1) * TILE + 40, (rows.max() + 1) * TILE + 40)
        panels = []
        for label, folder in [("reference", out / "reference")] + [(n, out / f"rt-{n}") for n in names]:
            im = Image.open(folder / "frame-047.png").convert("RGB").crop(box)
            im = im.resize((im.width * 2, im.height * 2), Image.NEAREST)
            ImageDraw.Draw(im).text((4, 2), label, fill=(255, 255, 0))
            panels.append(im)
        image = Image.new("RGB", (sum(p.width for p in panels) + 4 * (len(panels) - 1), panels[0].height))
        x = 0
        for p in panels:
            image.paste(p, (x, 0))
            x += p.width + 4
        image.save(sheet)
        print(sheet, box)


if __name__ == "__main__":
    if sys.argv[1] == "run":
        raise SystemExit(run(sys.argv[2], Path(sys.argv[3]), [tuple(v.split("=", 1)) for v in sys.argv[4:]]))
    report(Path(sys.argv[2]), sys.argv[3] if len(sys.argv) > 3 else None)
