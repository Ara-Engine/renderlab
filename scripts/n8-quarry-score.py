"""N8 quarry scores on the frozen Level B reference (docs/unreal-native-shadow-mask.md).

Uses the unchanged support/fresh_gate_metrics.py populations and statistics, so rows are
comparable with Unreal's native-mask table: partial RMS, signed mean, p99, G1 envelope
violations, G4 along-contour RMS and per-population RMS. Also counts solid flips
(|error| > 0.5 on lit or umbra receivers) and umbra leaks (umbra receivers above 0.1).

    py -3 -B scripts/n8-quarry-score.py --run E:/renderlab-work/n8/quarry-run/quarry
"""
import argparse
import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "support"))
import fresh_gate_metrics as gate  # noqa: E402

REFERENCE = Path("D:/Coding/renderlab-work/m6-adaptive/gpu-phase-quarry/quarry")
NATIVE = Path("D:/Coding/renderlab-work/ue-m5/Quarry/native-shadow")
# Decoded native VSM masks of docs/unreal-native-shadow-mask.md, registered to these poses.
VSM = {0: "native-vsm-baseline-p0-02", 1: "native-vsm-baseline-p1-01"}
SHAPE = (1080, 1920)


def traced_reference(run, pose):
    """The run's own reference on today's traced receivers: mean of Exact seeds 1-64."""
    raw = np.fromfile(run / f"pose-{pose}-traced-reference.bin", dtype="u1")
    n = SHAPE[0] * SHAPE[1]
    values = raw[: n * 12].view("<f4").reshape(n, 3)
    cover = raw[n * 12:].astype(bool).reshape(SHAPE)
    return values[:, 0].reshape(SHAPE).astype(float), values[:, 1].reshape(SHAPE).astype(float), values[:, 2].reshape(SHAPE), cover


def score_row(pose, name, cand, ref, sigma, depth, cover):
    out, err, pops = gate.metrics(cand, ref, sigma, depth, cover)
    solid = pops["lit"] | pops["occluded"]
    # S1-style contact loss: reference umbra receivers within 3 px of a lit or partial
    # receiver (shadow edges and contacts) that the candidate lights by more than half.
    from scipy import ndimage
    edge = pops["occluded"] & ndimage.binary_dilation(pops["lit"] | pops["partial"], np.ones((7, 7), bool))
    return dict(pose=pose, mode=name, partial_rms=out["partial"].get("rms"), partial_mean=out["partial"].get("mean"),
                partial_p99=out["partial"].get("p99"), g1_violations=out["envelope_violations"], g4_rms=out["along_contour"]["rms"],
                solid_flips=int((solid & (np.abs(err) > 0.5)).sum()), umbra_leaks=int((pops["occluded"] & (cand > 0.1)).sum()),
                lit_darkened=int((pops["lit"] & (cand < 0.9)).sum()), edge_umbra_lost=int((edge & (cand > 0.5)).sum()), edge_umbra=int(edge.sum()),
                populations={k: out[k].get("rms") for k in ("partial", "narrow", "faint", "isolated", "silhouette", "lit", "occluded")},
                counts={k: out[k].get("n", 0) for k in ("partial", "lit", "occluded")})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", type=Path, required=True)
    parser.add_argument("--reference", type=Path, default=REFERENCE)
    parser.add_argument("--traced", action="store_true", help="score against the run's traced-receiver reference, with native VSM")
    args = parser.parse_args()
    if args.traced:
        rows = []
        for pose in (0, 1):
            ref, sigma, depth, cover = traced_reference(args.run, pose)
            for name in ("interactive", "n8", "n8-ring0"):
                cand = np.fromfile(args.run / f"pose-{pose}-{name}.u32", dtype="<u4").reshape(SHAPE) / 256.0
                rows.append(score_row(pose, name, cand, ref, sigma, depth, cover))
            with np.load(NATIVE / VSM[pose] / "decoded-v2/native.npz") as a:
                vsm, valid = a["visibility"], a["valid"]
            common = cover & valid
            rows.append(score_row(pose, "UE VSM native", vsm, ref, sigma, depth, common))
            # The Renderlab modes on the same common domain, for a like-for-like comparison.
            for name in ("interactive", "n8"):
                cand = np.fromfile(args.run / f"pose-{pose}-{name}.u32", dtype="<u4").reshape(SHAPE) / 256.0
                rows.append(score_row(pose, name + " (VSM domain)", cand, ref, sigma, depth, common))
        for r in rows:
            print(json.dumps({k: (round(v, 4) if isinstance(v, float) else v) for k, v in r.items() if k not in ("populations", "counts")}))
        (args.run / "scores-traced.json").write_text(json.dumps(rows, indent=2))
        return
    rows = []
    names = sorted({p.name.split("-", 2)[2].removesuffix(".u32") for p in args.run.glob("pose-*-*.u32")})
    for pose in range(3):
        with np.load(args.reference / f"pose-{pose}-reference.npz") as a:
            ref, sigma, depth, cover = a["ref"], a["sigma"], a["depth"], a["cover"]
        for name in names:
            counts = np.fromfile(args.run / f"pose-{pose}-{name}.u32", dtype="<u4").reshape(ref.shape)
            cand = counts / 256.0
            out, err, pops = gate.metrics(cand, ref, sigma, depth, cover)
            solid = pops["lit"] | pops["occluded"]
            row = dict(pose=pose, mode=name,
                       partial_rms=out["partial"]["rms"], partial_mean=out["partial"]["mean"],
                       partial_p99=out["partial"]["p99"], g1_violations=out["envelope_violations"],
                       g4_rms=out["along_contour"]["rms"],
                       solid_flips=int((solid & (np.abs(err) > 0.5)).sum()),
                       umbra_leaks=int((pops["occluded"] & (cand > 0.1)).sum()),
                       lit_darkened=int((pops["lit"] & (cand < 0.9)).sum()),
                       populations={k: out[k].get("rms") for k in ("partial", "narrow", "faint", "isolated", "silhouette", "lit", "occluded")},
                       counts={k: out[k].get("n", 0) for k in ("partial", "lit", "occluded")})
            rows.append(row)
            if pose == 0:
                grey = lambda v: np.repeat((np.clip(v, 0, 1) * 255).astype("u1")[..., None], 3, -1)
                e = np.zeros((*ref.shape, 3), "u1")
                e[..., 0] = np.clip(-err * 8 * 255, 0, 255) * cover
                e[..., 2] = np.clip(err * 8 * 255, 0, 255) * cover
                Image.fromarray(np.concatenate([grey(np.where(cover, ref, 1)), grey(np.where(cover, cand, 1)), e], 1)).save(args.run / f"pose-0-{name}-view.png")
            print(json.dumps({k: v for k, v in row.items() if k not in ("populations", "counts")}))
    (args.run / "scores.json").write_text(json.dumps(rows, indent=2))


if __name__ == "__main__":
    main()
