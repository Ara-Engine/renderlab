"""M7-G1: build and run the world radiance cache GPU gates (sky visibility on SV).

  build    cargo test --no-run (own target on E:, -j 2); copies the test binary. Never inside a lease.
  gpu      one shared GPU lease (m7-u0-ue.py lease/guard): the SV gates on both paths.

Leases use the shared lock and E:/renderlab-work/gpu-schedule.md reservation/release lines; a
launch needs >= 4 GiB available, and the guard stops the run below 2 GiB or when a game or a
foreign renderer appears. No window opens. Evidence goes to E:/renderlab-work/m7-g1/runs/<label>.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess

REPO = Path(__file__).resolve().parent.parent
OUT = Path("E:/renderlab-work/m7-g1")
TARGET = OUT / "target"
BINARY = OUT / "bin/g1-tests.exe"
BENCH = Path("E:/renderlab-work/m7-u0/scenes/SV/scene.json")
TEST = "renderer::gi_cache::gpu_tests::gpu_gi_sky_visibility_sv"


def ue():
    spec = importlib.util.spec_from_file_location("m7_u0_ue", REPO / "scripts/m7-u0-ue.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.OWNER = "m7-g1"
    module.OUT = OUT
    return module


def build(args):
    command = ["cargo", "test", "-p", "renderlab", "--lib", "--no-run", "-j", "2", "--target-dir", str(TARGET),
               "--message-format=json"]
    proc = subprocess.run(command, cwd=REPO, capture_output=True, text=True)
    (OUT / "build.jsonl").write_text(proc.stdout)
    if proc.returncode:
        raise SystemExit(proc.stderr[-4000:])
    artifacts = [json.loads(line) for line in proc.stdout.splitlines() if line.startswith("{")]
    exe = next(a["executable"] for a in reversed(artifacts) if a.get("reason") == "compiler-artifact" and a.get("executable")
               and a["target"]["name"] == "renderlab")
    BINARY.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(exe, BINARY)
    print("binary", BINARY)


def window_open(u):
    """Another owner's reservation without a later release line, before its end; or another
    owner's request or queue line newer than the last release. A free lock is not a release."""
    import datetime as dt
    import re
    lines = u.SCHEDULE.read_text(encoding="utf-8").splitlines()
    now = u.utc()
    last_release = max((i for i, l in enumerate(lines) if "window released" in l and not l.startswith("[")), default=-1)
    for i, line in enumerate(lines):
        m = re.match(r"\[(\S+) reservation\] owner=(\S+) end=(\S+)", line)
        if m and m.group(2) != u.OWNER:
            released = any(l.startswith(m.group(2) + " ") and "window released" in l for l in lines[i + 1:])
            if not released and dt.datetime.fromisoformat(m.group(3)) > now:
                return line
        m = re.match(r"\[(\S+) [^\]]*(request|queue)\]", line)
        if m and m.group(1) != u.OWNER and i > last_release:
            return line
        # Our own request names its start time; never take the lock earlier.
        m = re.match(r"\[(\S+) request\] .* Starts at (\d\d:\d\d:\d\d)Z", line)
        if m and m.group(1) == u.OWNER:
            start = dt.datetime.combine(now.date(), dt.time.fromisoformat(m.group(2)), tzinfo=dt.timezone.utc)
            if start > now:
                return line
    return None


def hold_state(u):
    """The latest game-hold line: 'hold', 'relaxed', 'released' or None."""
    state = None
    for line in u.SCHEDULE.read_text(encoding="utf-8").splitlines():
        if line.startswith("[GAME HOLD RELEASED]"):
            state = "released"
        elif line.startswith("[GAME HOLD RELAXED]"):
            state = "relaxed"
        elif line.startswith("[GAME HOLD]"):
            state = "hold"
    return state


def gpu(args):
    u = ue()
    blocking = window_open(u)
    if blocking and not args.force_schedule:
        print("DEFERRED: schedule:", blocking)
        raise SystemExit(3)
    hold = hold_state(u)
    if hold == "hold":
        print("DEFERRED: game hold")
        raise SystemExit(3)
    contended_by_game = []
    if args.allow_game:
        # Only under a relaxed hold (headless runs allowed while Islam plays): the game is
        # recorded as contention, never a reason to stop. Memory and foreign-renderer
        # guards stay.
        if hold != "relaxed":
            raise SystemExit("--allow-game needs [GAME HOLD RELAXED] as the latest hold line")
        state = u.E2E.state

        def tolerant():
            record = state()
            contended_by_game.extend(p["Name"] for p in record["games"] if p["Name"] not in contended_by_game)
            record["games"] = []
            return record

        u.E2E.state = tolerant
    out = OUT / "runs" / args.label
    if out.exists():
        raise SystemExit(f"label {args.label} already used")
    env = dict(os.environ, RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
               RENDERLAB_M7_G1_BENCH=str(BENCH), RENDERLAB_M7_G1_OUT=str(out), RENDERLAB_M7_G1_SPP=str(args.spp),
               RENDERLAB_M7_G1_QUICK="1" if args.quick else "0")
    if not args.validate:
        env["VK_LOADER_LAYERS_DISABLE"] = "VK_LAYER_KHRONOS_validation"
        env.pop("VK_INSTANCE_LAYERS", None)
    env["RENDERLAB_M7_G1_PATHS"] = args.paths
    with u.lease("g1-" + args.label, args.minutes) as owner:
        out.mkdir(parents=True)
        try:
            u.guarded([BINARY, TEST, "--ignored", "--exact", "--nocapture", "--test-threads=1"], out / "run",
                      timeout=max(60, int(u.remaining_seconds(owner)) - 30), cwd=REPO, env=env)
        finally:
            (out / "contention.json").write_text(json.dumps(dict(
                game_running=contended_by_game, hold=hold,
                timings="contended: not valid for speed claims" if contended_by_game or hold == "relaxed" else "unlabelled"), indent=2))
    print(json.dumps(sorted(p.name for p in out.iterdir()), indent=1))


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("build")
    p = sub.add_parser("gpu")
    p.add_argument("label")
    p.add_argument("--minutes", type=int, default=10)
    p.add_argument("--spp", type=int, default=4096)
    p.add_argument("--quick", action="store_true")
    p.add_argument("--validate", action="store_true", help="keep Vulkan validation layers on")
    p.add_argument("--force-schedule", action="store_true", help="ignore pending requests (root-assigned slot)")
    p.add_argument("--allow-game", action="store_true", help="under [GAME HOLD RELAXED] only: run headless beside the game")
    p.add_argument("--paths", default="rt,rtsoftware,nonrt", help="comma list of rt, rtsoftware, nonrt")
    args = parser.parse_args()
    dict(build=build, gpu=gpu)[args.command](args)


if __name__ == "__main__":
    main()
