"""Stage/score high-sample classification; all frozen gate bounds stay unchanged."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
OUT = Path("E:/renderlab-work/receiver-adoption/followup")
REFERENCE = OUT.parent / "primary"
VERIFY = OUT / "solid-verification"
sys.path.insert(0, str(ROOT / "support"))
import fresh_gate


def stage():
    VERIFY.mkdir(exist_ok=True)
    # tan(.04) = .04002135. This slightly wider conservative band also covers
    # f32 normal/basis rounding and verifies both lit and occluded solid pixels.
    light = np.array([.4, .8, .3]); light /= np.linalg.norm(light)
    rows = []
    for pose in range(3):
        source = REFERENCE / f"pose-{pose}-reference.npz"
        reference = np.load(source)
        normals = np.fromfile(REFERENCE / f"pose-{pose}-normal.f32", "<f4").reshape(1080, 1920, 4)
        solid = (reference["ref"] == 0) | (reference["ref"] == 1)
        near = abs(normals[..., :3] @ light) <= .041
        selected = np.flatnonzero(solid & reference["cover"] & near).astype("<u4")
        selected.tofile(VERIFY / f"pose-{pose}-pixels.u32")
        rows.append(dict(pose=pose, selected=len(selected), lit=int((reference["ref"].ravel()[selected] == 1).sum()),
                         reference_sha256=hashlib.sha256(source.read_bytes()).hexdigest()))
    (VERIFY / "selection.json").write_text(json.dumps(dict(band_abs_n_dot_l=.041, replicas=4096,
        rays_per_replica=256, seeds="1..4096; seed zero excluded", poses=rows), indent=2))
    manifest = json.loads((OUT.parent / "static.json").read_text())
    manifest["output"] = str(VERIFY)
    (OUT / "solid-verification.json").write_text(json.dumps(manifest, indent=2))


def score():
    summary = []
    assert (VERIFY / "pose-0-pixels.u32").read_bytes() == (VERIFY / "pose-2-pixels.u32").read_bytes()
    assert (VERIFY / "pose-0-stats.u32").read_bytes() == (VERIFY / "pose-2-stats.u32").read_bytes()
    for pose in range(3):
        pixels = np.fromfile(VERIFY / f"pose-{pose}-pixels.u32", "<u4")
        stats = np.fromfile(VERIFY / f"pose-{pose}-stats.u32", "<u4").reshape(-1, 4).astype(float)
        assert np.all(stats[:, 3] == 4096)
        ref = 1-stats[:, 0]/1048576
        sigma = np.sqrt(np.maximum(0, (stats[:, 1]-stats[:, 0]**2/4096)/4095))/256
        partial = (ref > 0) & (ref < 1)
        old = np.load(REFERENCE / f"pose-{pose}-reference.npz")["ref"].ravel()[pixels]
        np.savez_compressed(VERIFY / f"pose-{pose}-verified.npz", pixels=pixels, ref=ref, sigma=sigma, original=old)
        rows = [dict(x=int(p%1920), y=int(p//1920), old=float(o), new=float(v), sigma=float(s),
                     blocked=int(n)) for p, o, v, s, n in zip(pixels[partial], old[partial], ref[partial], sigma[partial], stats[partial, 0])]
        summary.append(dict(pose=pose, verified=len(pixels), newly_partial=int(partial.sum()),
                            formerly_lit=int((old[partial] == 1).sum()), formerly_occluded=int((old[partial] == 0).sum()), receivers=rows))
    (VERIFY / "classification.json").write_text(json.dumps(summary, indent=2))
    gate = OUT / "level-a"
    gate.mkdir(exist_ok=True)
    patterns = ["pose-*-world.f32", "pose-*-normal.f32", "pose-*-seed-*-production.u32",
                "pose-*-seed-*-adaptive.u32", "pose-*-seed-*-prefix.u32", "pose-*-seed-*-hits.u32"]
    for pattern in patterns:
        for source in REFERENCE.glob(pattern):
            target = gate / source.name
            if not target.exists(): os.link(source, target)
    (gate / "diagnostics.json").write_bytes((REFERENCE / "diagnostics.json").read_bytes())
    result = fresh_gate.prepare(gate, "quarry", solid_verification=VERIFY)
    print(json.dumps(dict(poses=[{k: v for k, v in row.items() if k != "receivers"} for row in summary],
                         numeric_pass=result["numeric_pass"]), indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(); parser.add_argument("action", choices=("stage", "score"))
    args = parser.parse_args()
    (stage if args.action == "stage" else score)()
