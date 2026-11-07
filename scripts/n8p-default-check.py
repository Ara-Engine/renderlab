"""N8p default/Exact identity gate. Run the precompiled lib test binary under n8-run.py.

One no-RT process loads the quarry once, checks both poses against N8o's full SHA-256
counts hashes (final-t/quarry-no-rt), and repeats Surface's determinism/caster gates.
No Cargo or windows are launched. Outputs belong under E:/renderlab-work.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

QUARRY = Path("D:/Coding/renderlab-work/m6-adaptive/gpu-phase-quarry/quarry-manifest.json")
DETAIL = Path("D:/Coding/projects/desktop/renderlab/measurements/quarry-quality/detail")
# Per pose: after 16 sun turns, 48 camera frames, then 48 caster frames. N8o's Exact
# hashes are main's Exact output; Surface's hashes include its deterministic reuse.
EXPECTED = {
    "exact": [
        ["05e50a1d8ad174266cfe359a8185c7df5f47d87bd6ec3145b4caa3acd87d9d8f",
         "c58c0822a7bc83d4f493697d7c99f23eb3e8733f2d454ef426da8c0d6bce947a",
         "371171617a237358233544fed0496925942dc2c60f6a745b6c4cf3be5c4a397d"],
        ["bed48bf7099343bb1ba7d84d7a7ed7ab77220670f34f50f2edf801184fd78e0b",
         "3db7b54ddac3f70666124643f4dc859e063581bb70de9d4fbbd9c6025bc1760d",
         "0222cbae8e8b72a2d1b0e50e152e8807647fd767a38ced1eb7c34c31d5a0baba"],
    ],
    "surface": [
        ["c3d5a6946c4d274f682112564448f2964454356885a55e28d0b019adaf3fa66f",
         "3e0786a539a9343e7fe45f3e22719439b8f833142d1f605a5a669700aeb08cd8",
         "95d7350cc915693e866d87c1fd17de97665c9fd31f1dd5e50833df5ed03285a6"],
        ["e029d2ef86448e21fbffc42536bed84411d0171c45298e98e6dc10b81ba167c3",
         "03fe73d4bafaf2d67790b198175355f991791a398dbf369d5b8f577b77f3e2d5",
         "4c9b1ac32bd669afa99ae96df85e4fc09f02a619c8c3c4a9ee4bcbe963a3fac2"],
    ],
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("root", type=Path)
    args = parser.parse_args()
    args.root.mkdir(parents=True, exist_ok=True)
    manifest = dict(json.loads(QUARRY.read_text()), output=str(args.root / "quarry-no-rt"),
                    no_rt=True, validate=True, timing=True, hash_only=True, reference=False,
                    lod_pixels=[0.35], software_extent=[32], lod_lift=[1.0], poses=[0, 1],
                    stage_repeats=0, turn_frames=16, move_frames=48, approach_frames=0,
                    caster_gates=True, virtual_cache=str(DETAIL / "cache"),
                    virtual_manifest=str(DETAIL / "virtual.manifest"),
                    virtual_variants={"a-default": {}, "b-exact": {"lookup": "exact"},
                                      "c-surface": {"lookup": "surface"}, "d-default": {}})
    path = args.root / "manifest.json"
    path.write_text(json.dumps(manifest, indent=2))
    subprocess.run([str(args.binary), "gpu_visibility_quarry", "--ignored", "--nocapture", "--test-threads=1"],
                   env=dict(os.environ, RENDERLAB_PRIMARY_MANIFEST=str(path)),
                   creationflags=subprocess.CREATE_NO_WINDOW, check=True)
    out = args.root / "quarry-no-rt"
    changed = json.loads((out / "changed-frames.json").read_text())
    checked = []
    for pose in range(2):
        for variant in manifest["virtual_variants"]:
            label = f"p{pose}-lod0.35-sw32-lift1-v{variant}"
            snapshot = json.loads((out / (label + ".hashes.json")).read_text())
            phases = {k: v for row in changed if row["label"] == label for k, v in row.items()}
            hashes = [snapshot["counts_sha256"], phases["move_counts_sha256"], phases["caster_counts_sha256"]]
            lookup = "exact" if variant == "b-exact" else "surface"
            assert hashes == EXPECTED[lookup][pose], (label, hashes, EXPECTED[lookup][pose])
            gates = phases["caster_gates"]
            for key in ("static_differs_from_fresh", "static_differs_from_twin",
                        "history_unexplained", "history_repeat_differs"):
                assert gates[key] == 0, (label, key, gates[key])
            checked.append(dict(label=label, lookup=lookup, hashes=hashes, caster_gates=gates))
    summary = dict(passed=True, binary_sha256=hashlib.file_digest(args.binary.open("rb"), "sha256").hexdigest(),
                   checks=checked)
    (args.root / "identity-summary.json").write_text(json.dumps(summary, indent=2))
    print("N8p: all 24 full hashes match; default equals recorded Surface; explicit Exact equals main; caster gates pass", flush=True)


if __name__ == "__main__":
    main()
