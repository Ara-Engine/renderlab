"""U-SPECK flights (one lease, resumable): the game scene's flight with its cut at frame 24,
180 frames at 1080p, on both paths, with one switch changed at a time. A finished run leaves
`done.json` and is skipped on a rerun.

    py -3 -B tools/speck/ablate.py GAME_EXE OUT [--every N] [--only name,name]
                                   [--flight] [--stills] [--field] [--field-still] [--early] [--field-early]
                                   [--lib-test LIB_EXE NAME[,NAME]]

`--stills` also renders each switch from a frozen camera with a steady fire (`--still`, 96
frames, the last written) into OUT/still-PATH-NAME: the settled frame, for the energy check.
`--field` flies the field walk (game_scene `--diag field=1`), `--field-still` holds its first
pose; with any of these the flight runs only if `--flight` is given too. `--lib-test` first runs the lib test
binary's tests whose names contain NAME (ignored GPU tests included) and stops the lease unless
every NAME reports ok.
"""
import json
import os
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
WORK = Path("D:/Coding/renderlab-work/unit-speck")
FOX = "D:/Coding/renderlab-work/m7-sk1/Fox.glb"

# name -> (extra arguments, extra environment), most telling first. `--every 7` writes frames
# 14, 21 and 98 (the frames Islam marked) among 27.
SWITCHES = {
    "base": ([], {}),
    "gi-off": ([], {"RENDERLAB_ABLATE": "gi"}),
    "flicker-off": (["--diag", "flicker=0"], {}),
    # Inside GI and lighting (support/estimator_diag.rs): no local lights, sky-only cache.
    "lights-off": (["--diag", "lights=0"], {}),
    "bounce-off": (["--diag", "gi-bounce=0"], {}),
    "tr1-off": ([], {"RENDERLAB_ABLATE": "tr1"}),
    "reflections-off": ([], {"RENDERLAB_ABLATE": "reflections"}),
    "particles-off": (["--no-particles"], {}),
    # Each local light alone (bit 0 arch point, 1 crate spot, 2 fire point), no sun, unrotated
    # GI direction sets.
    "arch-only": (["--diag", "lights=1"], {}),
    "spot-only": (["--diag", "lights=2"], {}),
    "fire-only": (["--diag", "lights=4"], {}),
    "sun-off": (["--diag", "sun=0"], {}),
    "rotate-off": (["--diag", "gi-rotate=0"], {}),
    # Which lamp-lit surface: no bulb spheres, or bulbs that glow but reflect nothing.
    "bulb-off": (["--diag", "bulb=off"], {}),
    "bulb-dark": (["--diag", "bulb=dark"], {}),
    # The fix (IndirectSettings::near_light_rays): rays aimed around each local light.
    "rays0": (["--diag", "gi-light-rays=0"], {}),
    "rays8": (["--diag", "gi-light-rays=8"], {}),
    "rays16": (["--diag", "gi-light-rays=16"], {}),
    "rays32": (["--diag", "gi-light-rays=32"], {}),
    "rays0-dark": (["--diag", "gi-light-rays=0", "--diag", "bulb=dark"], {}),
    "rays16-dark": (["--diag", "gi-light-rays=16", "--diag", "bulb=dark"], {}),
    # The cache's own structure (the field fixture): lookup, filter, direction count.
    "nearest": (["--diag", "gi-lookup=nearest"], {}),
    "filter-off": (["--diag", "gi-filter=0"], {}),
    "dirs128": (["--diag", "gi-directions=128"], {}),
    # A table too small for the view (RENDERLAB_GI, src/engine/settings.rs): what full buckets look like.
    "slots17": ([], {"RENDERLAB_GI": "slots_log2=17"}),
    "slots18": ([], {"RENDERLAB_GI": "slots_log2=18"}),
    "slots19": ([], {"RENDERLAB_GI": "slots_log2=19"}),
    # The same binary without the eviction under pressure (IndirectSettings::pressure_eviction).
    "pressure-off": ([], {"RENDERLAB_GI": "pressure_eviction=0"}),
    "slots17-pressure-off": ([], {"RENDERLAB_GI": "slots_log2=17;pressure_eviction=0"}),
    # Gather hit cells sized to the ray's cone (IndirectSettings::hit_cone_cells): cells across it.
    "cone16": ([], {"RENDERLAB_GI": "hit_cone_cells=16"}),
    "cone8": ([], {"RENDERLAB_GI": "hit_cone_cells=8"}),
    "cone4": ([], {"RENDERLAB_GI": "hit_cone_cells=4"}),
    # The gather budget: every queued cell alike (as before), or the cells pixels read first
    # (IndirectSettings::gather_pixels_first); and the bound on any schedule, no budget at all.
    "rotation": ([], {"RENDERLAB_GI": "gather_pixels_first=0"}),
    "pixels-first": ([], {"RENDERLAB_GI": "gather_pixels_first=1"}),
    "uncapped": ([], {"RENDERLAB_GI": "gather_cells=4294967295"}),
    # The coarser bounce fallback for moving objects (IndirectSettings::mover_fill).
    "mover-off": ([], {"RENDERLAB_GI": "mover_fill=0"}),
    "mover-on": ([], {"RENDERLAB_GI": "mover_fill=1"}),
}
FLIGHT = ["--frames", "180", "--diag", "cut=1"]
STILL = ["--still", "--frames", "96", "--every", "95", "--diag", "flicker=0"]
# `--field`: the field walk (game_scene `--diag field=1`): 120 frames, every tenth written (the
# integration clip's frames 0, 10, 30, 60, 90 and 119 among them), into OUT/field-PATH-NAME.
FIELD = ["--frames", "120", "--every", "10", "--diag", "field=1"]


def main():
    game, out = sys.argv[1], Path(sys.argv[2])
    every = sys.argv[sys.argv.index("--every") + 1] if "--every" in sys.argv else "7"
    only = sys.argv[sys.argv.index("--only") + 1].split(",") if "--only" in sys.argv else list(SWITCHES)
    (WORK / "tmp").mkdir(parents=True, exist_ok=True)
    env = {k: v for k, v in os.environ.items() if k not in ("VK_INSTANCE_LAYERS", "VK_LAYER_PATH", "RENDERLAB_ABLATE")}
    env |= {"RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK": "I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
            "TMP": str(WORK / "tmp"), "TEMP": str(WORK / "tmp")}
    if "--lib-test" in sys.argv:
        lib, name = sys.argv[sys.argv.index("--lib-test") + 1:][:2]
        names = name.split(",")
        r = subprocess.run([lib, *names, "--include-ignored", "--test-threads", "1"], cwd=REPO, env=env,
                           capture_output=True, text=True, timeout=600)
        print((r.stdout + r.stderr)[-2500:], flush=True)
        # libtest exits 0 when no test matches a filter: every named test must report ok.
        missing = [n for n in names if not any(line.startswith("test ") and line.endswith("ok") and n in line for line in r.stdout.splitlines())]
        if r.returncode != 0 or missing:
            print("lib tests failed or did not run:", missing, flush=True)
            return r.returncode or 1
    worst = 0
    chosen = [m for m in ("--flight", "--stills", "--field", "--field-still", "--early", "--field-early") if m in sys.argv] or ["--flight"]
    table = {"--flight": ("", FLIGHT + ["--every", every]), "--stills": ("still-", STILL), "--field": ("field-", FIELD),
             "--field-still": ("fieldstill-", FIELD + ["--still"]),
             # A held camera's first 33 frames, every fourth written: how soon cells get their bounce.
             "--early": ("early-", ["--still", "--frames", "33", "--every", "4", "--diag", "flicker=0"]),
             "--field-early": ("fieldearly-", ["--still", "--frames", "33", "--every", "4", "--diag", "field=1"])}
    modes = [table[m] for m in chosen]
    for prefix, mode in modes:
        for name in only:
            extra, extra_env = SWITCHES[name]
            for path in ("nonrt", "rt"):
                run = out / f"{prefix}{path}-{name}"
                if (run / "done.json").exists():
                    continue
                run.mkdir(parents=True, exist_ok=True)
                start = time.monotonic()
                # Every run logs the cache's counters per frame (gi.jsonl; a readback, no picture change).
                log = {"RENDERLAB_GI_LOG": str(run / "gi.jsonl")}
                r = subprocess.run([game, "--path", path, "--fox", FOX, "--size", "1920x1080", *mode, *extra,
                                    "--out", str(run), "--pipeline-cache", str(WORK / "cache")],
                                   cwd=REPO, env=env | extra_env | log, capture_output=True, text=True, timeout=600)
                frames = len(list(run.glob("frame-*.png")))
                print(f"{run.name}: exit {r.returncode}, {frames} frames, {time.monotonic() - start:.0f} s", flush=True)
                if r.returncode == 0:
                    (run / "done.json").write_text(json.dumps(dict(game=game, frames=frames, tail=(r.stdout + r.stderr)[-400:])))
                else:
                    print((r.stdout + r.stderr)[-1500:], flush=True)
                worst = max(worst, r.returncode)
    return worst


if __name__ == "__main__":
    raise SystemExit(main())
