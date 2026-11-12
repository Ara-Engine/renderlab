"""U-RTVIS timing of the RT visibility pass (docs/rtvis.md): one lease, one game_scene binary.

    py -3 -B tools/rtvis/timing.py run GAME_EXE OUT [PLAN]   # inside a lease
    py -3 -B tools/rtvis/timing.py report OUT                # CPU

Each lease opens with the reference run: the default game scene on RT at 1080p (120 frames);
its median frame span is recorded and compared with REFERENCE_MS once that is set (more than
10 % off: the lease is not a measurement). PLAN `repro` (step 1) then walks `--diag forest=1`
(and the field walk) with a camera cut at frame 24, interleaving the default shadow budget
with an 8-ray one, the sun off, a high sun and 720p. Every run logs RENDERLAB_PASS_LOG with
its `visibility.*` parts. PLAN `cold` (step 3) times the forest walk's cut with the cold probe
stop on and off (RENDERLAB_COLD_PROBES), twice, interleaved. PLAN `pictures` writes every
frame of short single-cut clips with the stop on and off: the forest held still (cut at 24,
settled before and after), the forest walk, and the game scene's orbit jump on both paths.
`report` prints per run the median frame span, `visibility` and its parts over the moving
frames (8-22 and 40-119), and the frames from 22 to 30 around the cut; `hashes` compares the
picture runs' per-frame hashes, on against off.
"""
import json
import os
import statistics as st
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
WORK = Path("D:/Coding/renderlab-work/unit-rtvis")
# The default game scene on RT at 1080p, frame span median (ms): leases of 2026-11-12 measured
# 26.67, 26.54 and 26.84.
REFERENCE_MS = 26.7
FOX = "D:/Coding/renderlab-work/m7-sk1/Fox.glb"
# Held-budget runs (step 5): main's cold probes on, the held stop on (1) or off (0).
HELD = lambda v: {"RENDERLAB_COLD_PROBES": "1", "RENDERLAB_HELD_PROBES": v}
LOW = ["--hour", "17.651"]
CUT = ["--diag", "cut=1"]
FOREST = ["--diag", "forest=1", *CUT]
EIGHT = ["--diag", "shadow-budget=256,64,8,8,16"]
PLANS = {
    "repro": [
        ("forest-low-1", FOREST + LOW, {}),
        ("forest-low-b8-1", FOREST + LOW + EIGHT, {}),
        ("forest-low-nosun", FOREST + LOW + ["--diag", "sun=0"], {}),
        ("forest-noon", FOREST + ["--time", "noon"], {}),
        ("forest-low-720", FOREST + LOW + ["--size", "1280x720"], {}),
        ("field-low", ["--diag", "field=1", *CUT, *LOW], {}),
        ("field-low-opaque", ["--diag", "field=1", *CUT, *LOW], {"RENDERLAB_DIAG_ALL_OPAQUE": "1"}),
        ("forest-low-2", FOREST + LOW, {}),
        ("forest-low-b8-2", FOREST + LOW + EIGHT, {}),
    ],
    "cold": [
        (f"forest-low-cold{v}-{k}", FOREST + LOW, {"RENDERLAB_COLD_PROBES": v})
        for k in (1, 2) for v in ("1", "0")
    ],
    "pictures": [
        (f"{name}-cold{v}", extra, {"RENDERLAB_COLD_PROBES": v})
        for name, extra in [
            ("p-forest-still", FOREST + LOW + ["--still", "--frames", "60", "--every", "1"]),
            ("p-forest-walk", FOREST + LOW + ["--frames", "40", "--every", "1"]),
            ("p-game-rt", CUT + ["--frames", "40", "--every", "1"]),
            ("p-game-nonrt", CUT + ["--path", "nonrt", "--frames", "40", "--every", "1"]),
        ]
        for v in ("1", "0")
    ],
    # The forest walk and the game flight (the Fox walking, the crate spinning), moving frames.
    "held": [
        (f"{name}-held{v}-{k}", extra, HELD(v))
        for k in (1, 2)
        for name, extra in [
            ("forest", ["--diag", "forest=1", *LOW]),
            ("game", ["--fox", FOX]),
        ]
        for v in ("1", "0")
    ],
    # Every frame of: the forest walk; the game flight; the game scene with the camera held at
    # its first pose while the Fox and the crate move (their shadows sweep ground whose history
    # has saturated); and the game flight on non-RT (byte identity).
    "heldpics": [
        (f"{name}-held{v}", extra, HELD(v))
        for name, extra in [
            ("p-forest-walk", ["--diag", "forest=1", *LOW, "--frames", "60", "--every", "1"]),
            ("p-game-flight", ["--fox", FOX, "--frames", "60", "--every", "1"]),
            ("p-game-hold", ["--fox", FOX, "--diag", "hold=1", "--frames", "60", "--every", "1"]),
            ("p-game-nonrt", ["--fox", FOX, "--path", "nonrt", "--frames", "60", "--every", "1"]),
        ]
        for v in ("1", "0")
    ],
}
MATCHED_FRAMES = "16,32,47"


def matched(exe, out, spp="4096"):
    """reflection_matched's flight: the RT matched frames with the held stop on and off, then
    the path tracer at the same poses (the measurement scene's moving crate and hero)."""
    out = Path(out)
    env = environment()
    code = 0
    for name, extra, more in [
        ("m-held1", ["--path", "rt", "--matched"], HELD("1")),
        ("m-held0", ["--path", "rt", "--matched"], HELD("0")),
        ("m-reference", ["--path", "rt", "--reference", spp], {}),
    ]:
        d = out / name
        d.mkdir(parents=True, exist_ok=True)
        r = subprocess.run([exe, *extra, "--write-frames", MATCHED_FRAMES, "--out", str(d)], cwd=REPO,
                           capture_output=True, text=True, timeout=900, env=env | more)
        (d / "stderr.txt").write_text(r.stderr[-20000:])
        print(name, "exit", r.returncode, flush=True)
        code = max(code, r.returncode)
    return code


def rows(log):
    return [json.loads(line) for line in Path(log).read_text().splitlines() if line.strip()]


def environment():
    drop = ("VK_INSTANCE_LAYERS", "VK_LAYER_PATH", "RENDERLAB_GI", "RENDERLAB_DIAG_ALL_OPAQUE")
    env = {k: v for k, v in os.environ.items() if k not in drop}
    # The pre-step-3 cold budget and no held stop unless a run sets them itself.
    return env | {"RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK": "I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
                  "TMP": str(WORK / "tmp"), "TEMP": str(WORK / "tmp"), "RENDERLAB_COLD_PROBES": "0",
                  "RENDERLAB_HELD_PROBES": "0"}


def fly(game, out, extra, env, frames=120):
    out.mkdir(parents=True, exist_ok=True)
    log = out / "passes.jsonl"
    log.unlink(missing_ok=True)
    # Later flags win in game_scene's parser: `extra` may override the path, size and frames.
    args = [game, "--path", "rt", "--size", "1920x1080", "--frames", str(frames), "--every", "1000",
            *extra, "--out", str(out), "--pipeline-cache", str(WORK / "cache")]
    r = subprocess.run(args, cwd=REPO, capture_output=True, text=True, timeout=600,
                       env=env | {"RENDERLAB_PASS_LOG": str(log), "RENDERLAB_PASS_LOG_DETAIL": "1"})
    (out / "stderr.txt").write_text(r.stderr[-20000:])
    print(out.name, "exit", r.returncode, flush=True)
    return r.returncode


def run(game, out, plan="repro"):
    out = Path(out)
    env = environment()
    code = fly(game, out / "reference", [], env)
    frame = st.median(r["span"] for r in rows(out / "reference/passes.jsonl")[8:])
    ok = REFERENCE_MS is None or 0.9 * REFERENCE_MS <= frame <= 1.1 * REFERENCE_MS
    (out / "reference.json").write_text(json.dumps(dict(frame_ms=frame, expected_ms=REFERENCE_MS, within_10_percent=ok)))
    print(f"reference frame {frame:.2f} ms (expected {REFERENCE_MS}): {'ok' if ok else 'MORE THAN 10% OFF'}", flush=True)
    for name, extra, more in PLANS[plan]:
        code = max(code, fly(game, out / name, extra, env | more))
    return code


def hashes(out, switch="cold"):
    """Per picture clip: the frames whose hashes differ between the stop on and off."""
    out = Path(out)
    for on in sorted(out.glob(f"p-*-{switch}1")):
        off = on.with_name(on.name[:-1] + "0")
        a, b = (json.loads((d / "run.json").read_text())["frames"] for d in (on, off))
        differ = [x["frame"] for x, y in zip(a, b) if x["rgba8_sha256"] != y["rgba8_sha256"]]
        print(f"{on.name[:-6]:16} frames {len(a)}  differing {len(differ)}: {differ[:12]}{' ...' if len(differ) > 12 else ''}")


def report(out):
    out = Path(out)
    print(json.loads((out / "reference.json").read_text()))
    for d in sorted(p for p in out.iterdir() if (p / "passes.jsonl").exists()):
        r = rows(d / "passes.jsonl")
        moving = [x for x in r if 8 <= x["frame"] <= 22 or x["frame"] >= 40]
        if not moving:
            print(f"{d.name:22} n={len(r):3} (no moving frames; see stderr.txt)")
            continue
        parts = sorted({k for x in r for k in x["passes"] if k.startswith("visibility")})
        med = lambda key, xs: st.median(x["passes"].get(key, 0.0) for x in xs)
        line = f"{d.name:22} n={len(r):3} frame {st.median(x['span'] for x in moving):6.2f}"
        line += "".join(f"  {k} {med(k, moving):6.2f}" for k in parts)
        print(line)
        around = [x for x in r if 22 <= x["frame"] <= 30]
        print(" " * 22, "cut frames 22-30 visibility:", " ".join(f"{sum(x['passes'].get(k, 0) for k in parts):.1f}" for x in around),
              "| span:", " ".join(f"{x['span']:.1f}" for x in around))


if __name__ == "__main__":
    if sys.argv[1] == "run":
        raise SystemExit(run(*sys.argv[2:5]))
    if sys.argv[1] == "report":
        report(sys.argv[2])
    if sys.argv[1] == "hashes":
        hashes(*sys.argv[2:4])
    if sys.argv[1] == "matched":
        raise SystemExit(matched(*sys.argv[2:5]))
