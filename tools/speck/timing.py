"""U-SPECK timing of the game scene's flight (one lease): `near_light_rays` 0 against 16.

    py -3 -B tools/speck/timing.py run GAME_EXE OUT     # inside a lease
    py -3 -B tools/speck/timing.py report OUT           # CPU

`run` opens with the reference run (N-VSM2's: bin/n3's game scene on non-RT `Disk`, 96 frames
at 1080p, whose sun lookup stage reads about 1.21 ms; more than 10 % off and the lease is not
a measurement), then flies the flight (180 frames, cut at 24, no frame written) on each path
with the rays off and on, twice, alternating, with RENDERLAB_PASS_LOG's per-pass GPU timeline.
`report` prints, per path and setting, the median GPU span and GI pass times over the moving
frames (8-23 and 40-179) and over the 16 frames from the cut.
"""
import json
import os
import statistics as st
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
WORK = Path("D:/Coding/renderlab-work/unit-speck")
FOX = "D:/Coding/renderlab-work/m7-sk1/Fox.glb"
REFERENCE = "D:/Coding/renderlab-work/n-vsm1/bin/n3/game_scene.exe"
EXPECTED_LOOKUP_MS = 1.21
RUNS = [(path, rays, visit) for visit in (1, 2) for path in ("nonrt", "rt") for rays in (0, 16)]


def rows(log):
    return [json.loads(line) for line in Path(log).read_text().splitlines() if line.strip()]


def fly(game, out, path, extra, frames, env):
    out.mkdir(parents=True, exist_ok=True)
    log = out / "passes.jsonl"
    if log.exists():
        log.unlink()
    r = subprocess.run([game, "--path", path, "--fox", FOX, "--size", "1920x1080", "--frames", str(frames), "--every", "1000",
                        *extra, "--out", str(out), "--pipeline-cache", str(WORK / "cache")], cwd=REPO,
                       env=env | {"RENDERLAB_PASS_LOG": str(log), "RENDERLAB_PASS_LOG_DETAIL": "1"}, capture_output=True, text=True, timeout=600)
    print(out.name, "exit", r.returncode, flush=True)
    return r.returncode


def run(game, out):
    env = {k: v for k, v in os.environ.items() if k not in ("VK_INSTANCE_LAYERS", "VK_LAYER_PATH", "RENDERLAB_ABLATE")}
    env |= {"RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK": "I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK", "TMP": str(WORK / "tmp"), "TEMP": str(WORK / "tmp")}
    code = fly(REFERENCE, out / "reference", "nonrt", ["--diag", "vsm-lookup=disk"], 96, env)
    lookup = st.median(r["passes"].get("sun_shadow.lookup", 0.0) for r in rows(out / "reference/passes.jsonl")[8:])
    ok = 0.9 * EXPECTED_LOOKUP_MS <= lookup <= 1.1 * EXPECTED_LOOKUP_MS
    (out / "reference.json").write_text(json.dumps(dict(sun_lookup_ms=lookup, expected_ms=EXPECTED_LOOKUP_MS, within_10_percent=ok)))
    print(f"reference sun lookup {lookup:.3f} ms (expected {EXPECTED_LOOKUP_MS}): {'ok' if ok else 'MORE THAN 10% OFF'}", flush=True)
    for path, rays, visit in RUNS:
        code = max(code, fly(game, out / f"{path}-rays{rays}-{visit}", path, ["--frames", "180", "--diag", "cut=1", "--diag", f"gi-light-rays={rays}"], 180, env))
    return code


def report(out):
    print(json.loads((out / "reference.json").read_text()))
    for path in ("nonrt", "rt"):
        for rays in (0, 16):
            for visit in (1, 2):
                frames = rows(out / f"{path}-rays{rays}-{visit}/passes.jsonl")
                moving = frames[8:24] + frames[40:]
                cut = frames[24:40]
                gi = lambda rs: {n: st.median(r["passes"].get(n, 0.0) for r in rs) for n in sorted({k for r in rs for k in r["passes"]})
                                 if n.split(".")[0] in ("gi", "indirect", "gi_cache", "radiance_cache")}
                names = gi(moving)
                print(f"{path} rays {rays:2d} visit {visit}: moving span p50 {st.median(r['span'] for r in moving):6.2f} ms | cut window p50 "
                      f"{st.median(r['span'] for r in cut):6.2f} ms, max {max(r['span'] for r in cut):6.2f} | GI moving "
                      + " ".join(f"{n} {v:.3f}" for n, v in names.items()) + " | GI cut " + " ".join(f"{n} {v:.3f}" for n, v in gi(cut).items()))
    names = sorted({k for r in rows(out / "rt-rays16-1/passes.jsonl") for k in r["passes"]})
    print("passes:", " ".join(names))


if __name__ == "__main__":
    if sys.argv[1] == "run":
        raise SystemExit(run(sys.argv[2], Path(sys.argv[3])))
    report(Path(sys.argv[2]))
