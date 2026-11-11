"""Runs one named batch of U-BC lease steps (inside scripts/n8-run.py's lease; no cargo here).

    py -3 -B run_steps.py BATCH

Each step's output goes to out/<batch>/<step>.log and its exit code to out/<batch>/steps.json.
Steps keep running after a failure; the batch exits nonzero if any step failed.
"""
import hashlib
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

WORK = Path("D:/Coding/renderlab-work/unit-bc")
REPO = Path(__file__).resolve().parents[2]
ROOT_CHECK = Path("D:/Coding/renderlab-work/root-check")
FOX = "D:/Coding/renderlab-work/m7-sk1/Fox.glb"
ACK = {"RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK": "I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK"}


def test_binaries():
    """Paths of the test executables the last test build's log names."""
    log = (WORK / "build-check1.log").read_text(errors="replace")
    return re.findall(r"Executable .*\((.+\.exe)\)", log)


def lib():
    """The renderlab crate's own test binary (the asset and cook crates' start renderlab_)."""
    return next(path for path in test_binaries() if Path(path).name.startswith("renderlab-"))


def gpu_test(name):
    return [lib(), f"texture_bc_tests::{name}", "--ignored", "--nocapture", "--test-threads=1"]


def plain(env):
    """Without the Khronos validation layer, as root's identity gates run the game scene."""
    return {k: v for k, v in env.items() if k not in ("VK_INSTANCE_LAYERS", "VK_LAYER_PATH")}


def batches(out):
    game = str(WORK / "bin/game_scene.exe")
    return {
        # GPU steps first, the lib suite last (it survives a late guard trip least well).
        "a": [
            ("decode", gpu_test("gpu_bc_decode_matches_cpu"), {}),
            ("memory", gpu_test("gpu_bc_memory_4096"), {}),
            ("fixture", gpu_test("gpu_bc_fixture_matches_decoded_rgba8"), {}),
        ],
        "b": [
            ("residency", gpu_test("gpu_bc_residency_camera_cut"), {}),
            ("engine", gpu_test("gpu_bc_engine_fixture"), {}),
        ],
        "lib": [
            ("lib", [lib(), "--test-threads=2"], {}),
        ] + [
            (Path(path).stem, [path, "--test-threads=2"], {})
            for path in sorted(test_binaries()) if not Path(path).name.startswith("renderlab-")
        ],
        "game": [
            (f"game-{p}", [game, "--path", p, "--fox", FOX, "--frames", "40", "--every", "1", "--out", str(out / p),
                           "--pipeline-cache", str(out / "cache")], {"plain": "1"})
            for p in ("rt", "nonrt")
        ],
    }


def main():
    batch = sys.argv[1]
    out = WORK / "out" / batch
    out.mkdir(parents=True, exist_ok=True)
    base = dict(os.environ, **ACK, RENDERLAB_BC_OUT=str(WORK / "out"), TMP=str(WORK / "tmp"), TEMP=str(WORK / "tmp"))
    codes = {}
    for name, command, extra in batches(out)[batch]:
        env = dict(base)
        if extra.pop("plain", None):
            env = plain(env)
        env.update(extra)
        start = time.monotonic()
        with open(out / f"{name}.log", "w") as log:
            try:
                code = subprocess.run(command, cwd=REPO, stdout=log, stderr=subprocess.STDOUT, env=env, timeout=840).returncode
            except subprocess.TimeoutExpired:
                code = 124
        codes[name] = dict(exit=code, seconds=round(time.monotonic() - start, 1))
        print(batch, name, codes[name], flush=True)
        (out / "steps.json").write_text(json.dumps(codes, indent=1))
    if batch == "game":
        # Against main's saved frames (root's P-SPLIT gate: main with P-SPLIT merged, 40 of 40).
        same = {}
        for p in ("rt", "nonrt"):
            a = sorted((ROOT_CHECK / "fix-game" / f"fix-{p}").glob("frame-*.png"))
            b = sorted((out / p).glob("frame-*.png"))
            equal = sum(hashlib.sha256(x.read_bytes()).digest() == hashlib.sha256(y.read_bytes()).digest() for x, y in zip(a, b))
            same[p] = [equal, len(a), len(b)]
            codes[f"identity-{p}"] = dict(exit=0 if a and equal == len(a) == len(b) else 1)
        (out / "identity.json").write_text(json.dumps(same, indent=1))
        (out / "steps.json").write_text(json.dumps(codes, indent=1))
        print("identical frames against main:", same, flush=True)
    sys.exit(max(abs(c["exit"]) for c in codes.values()))


if __name__ == "__main__":
    main()
