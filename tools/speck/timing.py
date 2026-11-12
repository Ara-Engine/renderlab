"""U-SPECK timing of the game scene's flight (one lease): `near_light_rays` 0 against 16.

    py -3 -B tools/speck/timing.py run GAME_EXE OUT     # inside a lease
    py -3 -B tools/speck/timing.py cuts GAME_EXE OUT [--gi "slots_log2=17"]   # inside a lease: the field walk, a cut every frame
    py -3 -B tools/speck/timing.py report OUT           # CPU

Each lease opens with the reference run (this unit's s8 binary on the non-RT field walk, whose
frame root timed at 28.8 ms; more than 10 % off and the lease is not a measurement). `run` then flies the flight (180 frames, cut at 24, no frame written) on each path
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
# The reference run: this unit's s8 binary (dfc6fab3) on the non-RT field walk, whose frame root
# timed at 28.8 ms (2026-11-12, interleaved twice). N-VSM2's bin/n3, the earlier reference, was
# deleted in that night's disk cleanup.
REFERENCE = str(WORK / "bin/s8/game_scene.exe")
EXPECTED_FRAME_MS = 28.8
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


def environment():
    env = {k: v for k, v in os.environ.items() if k not in ("VK_INSTANCE_LAYERS", "VK_LAYER_PATH", "RENDERLAB_ABLATE", "RENDERLAB_GI", "RENDERLAB_GI_LOG")}
    return env | {"RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK": "I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK", "TMP": str(WORK / "tmp"), "TEMP": str(WORK / "tmp")}


def reference(out, env):
    """The lease's opening reference run; returns its exit code."""
    code = fly(REFERENCE, out / "reference", "nonrt", ["--diag", "field=1"], 120, env)
    frame = st.median(r["span"] for r in rows(out / "reference/passes.jsonl")[8:])
    ok = 0.9 * EXPECTED_FRAME_MS <= frame <= 1.1 * EXPECTED_FRAME_MS
    (out / "reference.json").write_text(json.dumps(dict(frame_ms=frame, expected_ms=EXPECTED_FRAME_MS, within_10_percent=ok)))
    print(f"reference frame {frame:.2f} ms (expected {EXPECTED_FRAME_MS}): {'ok' if ok else 'MORE THAN 10% OFF'}", flush=True)
    return code


def run(game, out):
    env = environment()
    code = reference(out, env)
    for path, rays, visit in RUNS:
        code = max(code, fly(game, out / f"{path}-rays{rays}-{visit}", path, ["--frames", "180", "--diag", "cut=1", "--diag", f"gi-light-rays={rays}"], 180, env))
    return code


# `cuts`: the field walk with a camera cut every frame (game_scene `--diag field=1 --diag
# cut=every`, 120 frames), the eviction under pressure off against on, twice, alternating.
CUT_RUNS = [(path, name, visit) for visit in (1, 2) for path in ("nonrt", "rt") for name in ("off", "on")]
CUT_SETTING = {"off": "pressure_eviction=0", "on": "pressure_eviction=1"}


def cuts(game, out, extra=""):
    """`extra`: more RENDERLAB_GI pairs for every run (`slots_log2=17;` forces a full table)."""
    env = environment()
    code = reference(out, env)
    for path, name, visit in CUT_RUNS:
        code = max(code, fly(game, out / f"{path}-{name}-{visit}", path, ["--frames", "120", "--diag", "field=1", "--diag", "cut=every"], 120,
                             env | {"RENDERLAB_GI": extra + CUT_SETTING[name]}))
    # Not timed: the same four runs once more with the cache's counters logged.
    for path in ("nonrt", "rt"):
        for name in ("off", "on"):
            folder = out / f"{path}-{name}-log"
            code = max(code, fly(game, folder, path, ["--frames", "120", "--diag", "field=1", "--diag", "cut=every"], 120,
                                 env | {"RENDERLAB_GI": extra + CUT_SETTING[name], "RENDERLAB_GI_LOG": str(folder / "gi.jsonl")}))
    return code


def cuts_report(out):
    print(json.loads((out / "reference.json").read_text()))
    for path in ("nonrt", "rt"):
        for name in ("off", "on"):
            for visit in (1, 2):
                frames = rows(out / f"{path}-{name}-{visit}/passes.jsonl")[8:]
                gi = {n: st.median(r["passes"].get(n, 0.0) for r in frames) for n in sorted({k for r in frames for k in r["passes"]}) if n.startswith("gi")}
                top = {n: v for n, v in gi.items() if "." not in n}
                print(f"{path} eviction {name:3} visit {visit}: frame span p50 {st.median(r['span'] for r in frames):6.2f} ms | GI stages sum {sum(top.values()):6.2f} ms | "
                      + " ".join(f"{n} {v:.2f}" for n, v in top.items()))
            log = [json.loads(line) for line in (out / f"{path}-{name}-log/gi.jsonl").read_text().splitlines() if line.strip()]
            total = lambda key: sum(r.get(key, 0) for r in log)
            print(f"   counters over {len(log)} frames: created {total('created')}, evicted {total('evicted')} (seen within 8 frames {total('evicted_recent')}), "
                  f"pixels' failed inserts {total('full_pixel')}, gathers {total('gathers')}, gather rays {total('gather_rays')}, traces {total('traces')}")


def report(out):
    print(json.loads((out / "reference.json").read_text()))
    for path in ("nonrt", "rt"):
        for rays in (0, 16):
            for visit in (1, 2):
                frames = rows(out / f"{path}-rays{rays}-{visit}/passes.jsonl")
                moving = frames[8:24] + frames[40:]
                cut = frames[24:40]
                gi = lambda rs: {n: st.median(r["passes"].get(n, 0.0) for r in rs) for n in sorted({k for r in rs for k in r["passes"]})
                                 if n.startswith("gi") and "." not in n}
                names = gi(moving)
                print(f"{path} rays {rays:2d} visit {visit}: moving span p50 {st.median(r['span'] for r in moving):6.2f} ms | cut window p50 "
                      f"{st.median(r['span'] for r in cut):6.2f} ms, max {max(r['span'] for r in cut):6.2f} | GI moving "
                      + " ".join(f"{n} {v:.3f}" for n, v in names.items()) + " | GI cut " + " ".join(f"{n} {v:.3f}" for n, v in gi(cut).items()))


if __name__ == "__main__":
    mode, target = sys.argv[1], Path(sys.argv[-1])
    if mode == "run":
        raise SystemExit(run(sys.argv[2], target))
    if mode == "cuts":
        extra = sys.argv[sys.argv.index("--gi") + 1] + ";" if "--gi" in sys.argv else ""
        raise SystemExit(cuts(sys.argv[2], Path(sys.argv[3]), extra))
    (cuts_report if (target / "rt-on-1").exists() else report)(target)
