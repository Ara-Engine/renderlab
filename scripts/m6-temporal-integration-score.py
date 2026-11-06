"""Reuse frozen T1 film scores only after checkpoint SHA-256 identity is established.

The original raw film references were removed under the evidence retention policy.
This does not regenerate or rescore them, and carries no new visibility-solid claim.
"""
import argparse
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--batch", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--scores", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    current = json.loads((args.batch / "temporal.json").read_text())
    baseline = json.loads(args.baseline.read_text())
    scores = json.loads(args.scores.read_text())
    identity = []
    for program, captures in current["digests"].items():
        expected = {row["n"]: row["sha256"] for row in baseline["digests"][program]}
        for row in captures:
            identity.append(dict(program=program, frame=row["n"], identical=row["sha256"] == expected[row["n"]]))
    assert len(identity) == 54 and all(row["identical"] for row in identity), identity
    result = dict(method="Frozen film-score reuse after byte-identical HDR checkpoint hashes; raw references were deleted",
                  baseline=str(args.baseline), scores=str(args.scores), checkpoint_identity=identity, poses={})
    for pose, score in scores["poses"].items():
        frames = {}
        for frame in (8, 128):
            curve = score["t1_curve"][str(frame)]
            ratios = {f"{channel}/{population}": rms / score["bars"][channel][population]["bar"]
                      for channel in ("L0", "L4", "D0", "D4")
                      for population, rms in curve[channel].items()
                      if population in score["bars"][channel] and score["bars"][channel][population]["bar"] > 0}
            frames[str(frame)] = dict(radiance_rms={c: curve[c] for c in ("L0", "L4", "D0", "D4")},
                                      worst_bar_ratio=max(ratios.values()), bar_ratios=ratios)
        result["poses"][pose] = frames
    result["receiver_defect"] = "Known concave-edge production receiver defect remains; no receiver-origin changes or solid-gate waiver"
    args.out.write_text(json.dumps(result, indent=2))
    print(json.dumps(dict(checkpoints_identical=len(identity), poses={p: {n: s["worst_bar_ratio"] for n, s in frames.items()}
                                                                    for p, frames in result["poses"].items()})))


if __name__ == "__main__":
    main()
