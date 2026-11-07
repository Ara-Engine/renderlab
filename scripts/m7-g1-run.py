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
    staged = BINARY.with_name(args.name + ".exe")
    staged.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(exe, staged)
    print("binary", staged)


def window_open(u):
    """Another owner's reservation without a later release line, before its end; or another
    owner's request or queue line newer than the last release. A free lock is not a release."""
    import datetime as dt
    import re
    lines = u.SCHEDULE.read_text(encoding="utf-8").splitlines()
    now = u.utc()
    last_release = max((i for i, l in enumerate(lines) if "window released" in l and not l.startswith("[")), default=-1)
    stamp = r"window released (\d{4}-\d\d-\d\dT[\d:.]+(?:\+00:00|Z)?)"
    for i, line in enumerate(lines):
        # Any reservation (plain or quiet, any case) names its owner; one with an end time
        # blocks until that end or a later release line from its owner.
        m = re.match(r"\[(\S+) [^\]]*reservation\] owner=(\S+)", line, re.IGNORECASE)
        if m and m.group(2) != u.OWNER:
            released = any(l.startswith(m.group(2) + " ") and "window released" in l for l in lines[i + 1:])
            end = re.search(r"\bend=(\S+?)[;,]?(?:\s|$)", line)
            if end:
                if not released and dt.datetime.fromisoformat(end.group(1).replace("Z", "+00:00")) > now:
                    return line
            elif not released and "quiet" in line.lower():
                # A queued quiet run gets the lock first: for 3 minutes after every
                # release line, short screens leave it free so the next quiet owner can
                # start; once started, its own reservation line blocks above.
                stamps = re.findall(stamp, "\n".join(lines[i + 1:]))
                last = max((dt.datetime.fromisoformat(s.replace("Z", "+00:00")) for s in stamps), default=None)
                if last is None or (now - last).total_seconds() < 180:
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
    env["RENDERLAB_M7_G1_VISITS"] = str(args.visits)
    if args.configs:
        env["RENDERLAB_M7_G1_CONFIGS"] = args.configs
    if args.sections:
        env["RENDERLAB_M7_G1_SECTIONS"] = args.sections
    # Each binary is one process writing its own subdirectory, in order, inside one lease
    # (a paired A/B of staged builds); a single binary writes the label directory itself.
    binaries = [Path(b) for b in args.binary] or [BINARY]
    with u.lease("g1-" + args.label, args.minutes) as owner:
        out.mkdir(parents=True)
        try:
            for binary in binaries:
                target = out / binary.stem if len(binaries) > 1 else out
                target.mkdir(exist_ok=True)
                env["RENDERLAB_M7_G1_OUT"] = str(target)
                u.guarded([binary, TEST, "--ignored", "--exact", "--nocapture", "--test-threads=1"], target / "run",
                          timeout=max(60, int(u.remaining_seconds(owner)) - 30), cwd=REPO, env=env)
        finally:
            (out / "contention.json").write_text(json.dumps(dict(
                game_running=contended_by_game, hold=hold,
                timings="contended: not valid for speed claims" if contended_by_game or hold == "relaxed" else "unlabelled"), indent=2))
    print(json.dumps(sorted(p.name for p in out.iterdir()), indent=1))


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("build").add_argument("--as", dest="name", default=BINARY.stem,
                                         help="staged name under bin/ (variants for a paired A/B)")
    p = sub.add_parser("gpu")
    p.add_argument("label")
    p.add_argument("--minutes", type=int, default=10)
    p.add_argument("--spp", type=int, default=4096)
    p.add_argument("--quick", action="store_true")
    p.add_argument("--validate", action="store_true", help="keep Vulkan validation layers on")
    p.add_argument("--force-schedule", action="store_true", help="ignore pending requests (root-assigned slot)")
    p.add_argument("--allow-game", action="store_true", help="under [GAME HOLD RELAXED] only: run headless beside the game")
    p.add_argument("--paths", default="rt,rtsoftware,nonrt", help="comma list of rt, rtsoftware, nonrt")
    p.add_argument("--configs", default="", help="comma list of configurations to score (default all)")
    p.add_argument("--sections", default="", help="comma list of quality, gates, motion, caster, timing (default all)")
    p.add_argument("--visits", type=int, default=1, help="interleaved timing visits per configuration")
    p.add_argument("--binary", action="append", default=[], help="staged test binary; repeat for a paired A/B")
    args = parser.parse_args()
    dict(build=build, gpu=gpu)[args.command](args)


if __name__ == "__main__":
    main()
