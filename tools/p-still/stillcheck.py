"""P-STILL still-idle image check (CPU): base (gi-jittered) against candidate on the long-still
capture manifest, per captured frame and path. Error is P-YARD2's (score.py: F0 codes at -14 EV,
all covered pixels, against the frame's path-traced reference); the A/B difference counts pixels
over 8 codes and writes x8 difference crops for inspection.

    py -3 -B tools/p-still/stillcheck.py
"""
import importlib.util
import json
from pathlib import Path

import numpy as np
from PIL import Image

HERE = Path(__file__).resolve().parent
WORK = Path("D:/Coding/renderlab-work/p-still")
spec = importlib.util.spec_from_file_location("pyard_score", HERE.parent / "p-yard/score.py")
score = importlib.util.module_from_spec(spec)
spec.loader.exec_module(score)
frames = json.loads((WORK / "manifest-still-capture.json").read_text())["scenes"]["quarry"]["frames"]
stops = score.MANIFEST["scenes"]["quarry"]["exposure_stops"]
sheets = WORK / "sheets"
sheets.mkdir(exist_ok=True)


def load(run, i):
    info = json.loads((WORK / "runs" / run / "engine.json").read_text())["hdr"]
    w, h, pre = info["width"], info["height"], info["pre_exposure"]
    raw = np.fromfile(WORK / "runs" / run / f"hdr-{i:04d}.bin", "<f2")
    return raw.reshape(h, raw.size // h)[:, :w * 4].reshape(h, w, 4)[..., :3].astype(np.float32) / np.float32(pre)


rows = []
for path in ("rt", "nonrt"):
    for i, f in enumerate(frames):
        if not f["capture"]:
            continue
        a, b = (score.codes(load(f"sc-{v}-{path}", i), stops) for v in ("base", "cand"))
        if f["reference"]:
            ref = score.reference(f["reference"])
            mean, mask = ref["display"][0][0], ref["pops"]["all-covered"]
            err = lambda x: float(np.sqrt(((x - mean) ** 2).mean(-1)[mask].mean()))
        else:  # no reference for this walk frame: A/B difference only
            err = lambda x: None
        d = np.abs(a - b).max(-1)
        row = dict(path=path, frame=i, state=f["state"], err_base=err(a), err_cand=err(b),
                   over8=int((d > 8).sum()), over2=int((d > 2).sum()), max=float(d.max()), mean=float(d.mean()))
        rows.append(row)
        print(row, flush=True)
        if f["state"] in ("still", "walk") or i == 609:
            to8 = lambda x: np.clip(x, 0, 255).astype(np.uint8)
            Image.fromarray(np.concatenate([to8(a), to8(b), to8(np.repeat(d[..., None] * 8, 3, -1))], 1)).save(
                sheets / f"still-{path}-{i:04d}.png")
(WORK / "stillcheck.json").write_text(json.dumps(rows, indent=1))
