"""P-RTS final verification, run inside tools/p-rts/lease.py (prebuilt binaries only):
CPU naga check, ARA1 A/B (ab.py), the RTDEF2 S2 grain fixture on baseline and candidate
test binaries, then the candidate's library suite (lib last).

    final.py ROOT BASE_DIR CAND_DIR
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parents[2]
SCENES = Path("D:/Coding/renderlab-work/p-rts/scenes")
FLAGS = getattr(subprocess, "CREATE_NO_WINDOW", 0)
sys.path.insert(0, str(REPO / "tools/ara1"))
from lease import check_order  # noqa: E402


def step(name, command, log, env=None):
    # Every step re-reads the latest root order; a changed order ends the lease here.
    check_order(os.environ["P_RTS_EXPECTED_ORDER"])
    begin = time.perf_counter()
    with log.open("w", encoding="utf-8") as out:
        code = subprocess.run(command, cwd=REPO, env=env or os.environ, stdout=out, stderr=subprocess.STDOUT,
                              creationflags=FLAGS).returncode
    print(json.dumps(dict(step=name, exit_code=code, seconds=round(time.perf_counter() - begin, 2))), flush=True)
    return code


def main():
    root, base, cand = map(Path, sys.argv[1:4])
    skip = set(sys.argv[4].split(",")) if len(sys.argv) > 4 else set()
    root.mkdir(parents=True, exist_ok=True)
    release = {k: v for k, v in os.environ.items() if k not in ("VK_INSTANCE_LAYERS", "VK_LAYER_ENABLES")}
    if "naga" not in skip and step("naga", [str(cand / "renderlab-tests.exe"), "reconstruction_modules_validate_without_ray_queries"],
            root / "naga.log"):
        return 1
    quality = ["game-base-quality", "ara-pbr-oriented-quality", "quarry-daylight-quality", "game-no-gi-quality",
               "game-no-reflections-quality", "game-alpha-quality", "game-lights-quality", "game-fog-quality",
               "game-atmosphere-quality"]
    timing = [str(SCENES / f"{s}-stream.json") for s in ("game-base", "ara-pbr-oriented", "quarry-daylight")]
    # Full-quarry steps need 8 GiB admission; short leases run the rest at 7 GiB.
    if "quarry" in skip:
        quality, timing = [q for q in quality if "quarry" not in q], [t for t in timing if "quarry" not in t]
    if "nonquarry" in skip:
        quality, timing = [q for q in quality if "quarry" in q], [t for t in timing if "quarry" in t]
    failures = 0 if "ara1-ab" in skip else step("ara1-ab", [sys.executable, "-B", str(REPO / "tools/p-rts/ab.py"), str(root / "ab"),
                                "--binary", f"base={base / 'ara1_capture.exe'}", "--binary", f"cand={cand / 'ara1_capture.exe'}",
                                "--reps", "2", "--quality", *quality, "--timing", *timing], root / "ab.log")
    for label, directory in (("base", base), ("cand", cand)):
        if "s2" in skip or f"s2-{label}" in skip:
            continue
        env = release | {"RENDERLAB_PRIMARY_MANIFEST": f"D:/Coding/renderlab-work/p-rts/s2-{label}.json"}
        failures += step(f"s2-{label}", [str(directory / "renderlab-tests.exe"), "gpu_motion_bench_batch", "--ignored",
                                         "--nocapture", "--test-threads=1"], root / f"s2-{label}.log", env)
    if "lib" not in skip:
        failures += step("lib", [str(cand / "renderlab-tests.exe")], root / "lib.log")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
