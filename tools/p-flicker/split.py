"""P-FLICKER flicker split (CPU): P-YARD2's flicker residual on consecutive moving frames,
pooled per reference population and per pixel, for Renderlab runs and Unreal captures.

    py -3 -B tools/p-flicker/split.py OUT RUN...
      RUN: a runs/ directory (NAME-PATH) of a capture, or ue:CONFIG for p-flicker/ue/quarry/capture/CONFIG

The headline number is exactly tools/p-yard/score.py's `flicker@L0` (all-covered pixels both
frames share, same reference placement). The split pools the same squared residual inside each
reference population (edge, silhouette, occluded, contact, lit, solid) and in `interior`
(covered, not edge or silhouette); the shares are of the headline's squared sum. `energy.npy`
is the mean squared residual per pixel, for heat maps against Unreal.
"""
import importlib.util
import json
from pathlib import Path
import sys

import numpy as np

HERE = Path(__file__).resolve().parent
WORK = Path("D:/Coding/renderlab-work/p-flicker")
spec = importlib.util.spec_from_file_location("pyard_score", HERE.parent / "p-yard/score.py")
score = importlib.util.module_from_spec(spec)
spec.loader.exec_module(score)
FRAMES = score.MANIFEST["scenes"]["quarry"]["frames"]
STOPS = score.MANIFEST["scenes"]["quarry"]["exposure_stops"]
POPS = ("edge", "silhouette", "occluded", "contact", "lit", "solid")
CONTRAST = ((0, 2), (2, 5), (5, 10), (10, 20), (20, 1000))


def loaders(run):
    """{frame index: () -> HDR} for one run."""
    if run.startswith("ue:"):
        case = WORK / "ue/quarry/capture" / run[3:]
        native = json.loads((case / "native.json").read_text())["frames"]
        first = score.MANIFEST["scenes"]["quarry"]["ue_dump_first"]
        def load(row):
            def f():
                with np.load(row["archive"]) as c:
                    return c["hdr"][..., :3].astype(np.float32) / np.float32(c["pre_exposure"])
            return f
        return {i: load(row) for i, row in zip(range(first, len(FRAMES)), native) if Path(row["archive"]).exists()}
    case = WORK / "runs" / run
    info = json.loads((case / "engine.json").read_text())["hdr"]
    w, h, pre = info["width"], info["height"], info["pre_exposure"]
    def load(path):
        def f():
            raw = np.fromfile(path, "<f2")
            return raw.reshape(h, raw.size // h)[:, :w * 4].reshape(h, w, 4)[..., :3].astype(np.float32) / np.float32(pre)
        return f
    return {int(p.stem[4:]): load(p) for p in case.glob("hdr-*.bin")}


def split(run):
    found = loaders(run)
    sums = {p: [0.0, 0] for p in ("all",) + POPS + ("interior",) + tuple(f"contrast-{lo}-{hi}" for lo, hi in CONTRAST)}
    energy, pairs, per_pair, previous = None, 0, [], None
    for i in sorted(found):
        f = FRAMES[i]
        if not f["reference"]:
            previous = None
            continue
        ref = score.reference(f["reference"])
        est = score.codes(found[i](), STOPS)
        # As score.py: a moving frame pairs with the frame before it, whatever that frame's state.
        if previous and previous[0] == i - 1 and f["state"] in ("walk", "return"):
            pref = score.reference(previous[1])
            common = ref["pops"]["all-covered"] & pref["pops"]["all-covered"] & (ref["hit"] == pref["hit"])
            residual = est - previous[2] - (ref["display"][0][0] - pref["display"][0][0])
            e2 = (residual ** 2).mean(-1)
            masks = {"all": common, "interior": common & ~ref["pops"]["edge"] & ~ref["pops"]["silhouette"]}
            masks |= {p: common & ref["pops"][p] for p in POPS}
            # Local texture contrast of the reference (codes, 4-neighbour gradient): fixed bins,
            # so a resampling residual shows as energy rising with contrast.
            y = ref["display"][0][0].mean(-1)
            g = np.zeros_like(y)
            g[1:-1, 1:-1] = (np.abs(y[1:-1, 2:] - y[1:-1, :-2]) + np.abs(y[2:, 1:-1] - y[:-2, 1:-1])) / 2
            for lo, hi in CONTRAST:
                masks[f"contrast-{lo}-{hi}"] = common & (g >= lo) & (g < hi)
            for p, m in masks.items():
                sums[p][0] += float(e2[m].sum())
                sums[p][1] += int(m.sum())
            per_pair.append(dict(index=i, rms=float(e2[common].mean() ** .5)))
            e2[~common] = 0
            energy = e2 if energy is None else energy + e2
            pairs += 1
        previous = (i, f["reference"], est)
    total = sums["all"][0]
    pops = {p: dict(rms=(s / n) ** .5 if n else None, pixels_per_pair=n / max(pairs, 1), share=s / total) for p, (s, n) in sums.items()}
    return dict(run=run, pairs=pairs, flicker=pops["all"]["rms"], pops=pops, per_pair=per_pair), energy / max(pairs, 1)


if __name__ == "__main__":
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    results = {}
    for run in sys.argv[2:]:
        result, energy = split(run)
        results[run] = result
        np.save(out / f"energy-{run.replace(':', '-')}.npy", energy.astype(np.float32))
        print(f"{run}: flicker {result['flicker']:.3f} ({result['pairs']} pairs) | " + " ".join(
            f"{p} {v['rms']:.2f}/{100 * v['share']:.0f}%" for p, v in result["pops"].items() if p != "all" and v["rms"]), flush=True)
    path = out / "split.json"
    old = json.loads(path.read_text()) if path.exists() else {}
    path.write_text(json.dumps(old | results, indent=1) + "\n")
