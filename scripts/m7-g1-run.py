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
# The owning unit: m7-g1, or a follow-up such as m7-g1b (scripts/m7-g1b-run.py), with its
# own schedule owner name and evidence root.
UNIT = os.environ.get("M7_G1_UNIT", "m7-g1")
OUT = Path("E:/renderlab-work") / UNIT
TARGET = OUT / "target"
BINARY = OUT / "bin/g1-tests.exe"
BENCH = Path("E:/renderlab-work/m7-u0/scenes/SV/scene.json")
TEST = "renderer::gi_cache::gpu_tests::gpu_gi_sky_visibility_sv"


def ue():
    spec = importlib.util.spec_from_file_location("m7_u0_ue", REPO / "scripts/m7-u0-ue.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.OWNER = UNIT
    module.OUT = OUT
    return module


def reservation_released(line, owner, later):
    """A later release line for this reservation: it starts with the owner, or names the
    reservation's label ("DL4 retained-check-b window released" for owner dl4-env-helpers)."""
    import re
    label = re.search(r"\blabel=(\S+)", line)
    return any("window released" in l and not l.startswith("[")
               and (l.startswith(owner + " ") or (label and f" {label.group(1)} window released" in l))
               for l in later)


def quiet_active(u, any_lease=True):
    """Another owner's window in force now. With `any_lease` (the default, for starting a
    build), any reservation with an end time and no later release line; always, a QUIET
    active or quiet window line that is in force: start <= now < end when it names them,
    otherwise until a later release line names its label or owner. Queue lines
    (position=) are not windows."""
    import datetime as dt
    import re
    lines = u.SCHEDULE.read_text(encoding="utf-8").splitlines()
    now = u.utc()
    for i, line in enumerate(lines):
        m = re.match(r"\[(\S+) [^\]]*reservation\] owner=(\S+) end=(\S+)", line, re.IGNORECASE)
        if any_lease and m and m.group(2) != u.OWNER:
            released = reservation_released(line, m.group(2), lines[i + 1:])
            if not released and dt.datetime.fromisoformat(m.group(3).replace("Z", "+00:00")) > now:
                return line
    for i, line in enumerate(lines):
        m = re.match(r"\[(\S+) [^\]]*quiet[^\]]*\] owner=(\S+)", line, re.IGNORECASE)
        if not m or m.group(2) == u.OWNER or "position=" in line:
            continue
        later = lines[i + 1:]
        released = reservation_released(line, m.group(2), later) or any(
            l.lower().startswith(m.group(1).lower() + " ") and "released" in l for l in later)
        if released:
            continue
        start = re.search(r"\bstart=(\S+?)[;,]?(?:\s|$)", line)
        end = re.search(r"\bend=(\S+?)[;,]?(?:\s|$)", line)
        if start and end:
            t0, t1 = (dt.datetime.fromisoformat(x.group(1).replace("Z", "+00:00")) for x in (start, end))
            if t0 <= now < t1:
                return line
        elif re.search(r"QUIET (active|window)", line, re.IGNORECASE):
            return line
    return None


def build(args):
    """Two Cargo jobs, started only when no other lease is active. A watchdog stops the
    whole build tree if another owner's quiet window starts (no compiler inside quiet
    windows) and clears the crate's incremental cache, which a killed build can leave
    inconsistent (LNK1120); rerun later."""
    import time
    command = ["cargo", "test", "-p", "renderlab", "--lib", "--no-run", "-j", "2", "--target-dir", str(TARGET),
               "--message-format=json"] + (["--release"] if args.release else [])
    u = ue()
    # Never inside a quiet window; with --beside-leases, other owners' functional (non-quiet)
    # leases do not block a two-job build (CPU only, no GPU work).
    if (line := quiet_active(u, any_lease=not args.beside_leases)):
        print(f"DEFERRED: quiet window: {line[:160]}")
        raise SystemExit(3)
    log = (OUT / "build.jsonl").open("w")
    err = (OUT / "build.err").open("w")
    proc = subprocess.Popen(command, cwd=REPO, stdout=log, stderr=err, text=True)
    while proc.poll() is None:
        time.sleep(5)
        if (line := quiet_active(u, any_lease=False)):
            subprocess.run(["taskkill", "/T", "/F", "/PID", str(proc.pid)], capture_output=True)
            proc.wait()
            for stale in (TARGET / "debug" / "incremental").glob("renderlab-*"):
                shutil.rmtree(stale, ignore_errors=True)
            print(f"STOPPED for quiet window: {line[:160]}")
            raise SystemExit(3)
    log.close()
    err.close()
    if proc.returncode:
        raise SystemExit((OUT / "build.err").read_text()[-4000:])
    stdout = (OUT / "build.jsonl").read_text()
    artifacts = [json.loads(line) for line in stdout.splitlines() if line.startswith("{")]
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
            released = reservation_released(line, m.group(2), lines[i + 1:])
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
    with u.lease(UNIT[3:] + "-" + args.label, args.minutes) as owner:
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


def functional_gate(result, full=True):
    """The v4 correctness gates on every path; returns the failures (empty passes). Without
    `full`, only the determinism and non-caller gates (a gates-only step)."""
    failures = []
    for path, v in result["paths"].items():
        d = v.get("determinism", {})
        for k in ["table_repeat_equal", "image_repeat_equal", "serial_table_equal", "serial_image_equal"]:
            if not d.get(k):
                failures.append(f"{path} determinism {k}")
        if not v.get("non_caller", {}).get("toon_image_equal"):
            failures.append(f"{path} non-caller image changed")
        if not full:
            continue
        if not v.get("caster", {}).get("rolled_equals_fresh"):
            failures.append(f"{path} caster rolled != fresh")
        stale = v.get("rotation-staleness") or [{}]
        if stale[-1].get("pixels_differing_from_fresh") != 0:
            failures.append(f"{path} staleness does not settle")
        for pose, bound in [("overview", 0.03), ("gaps", 0.05)]:
            rms = v.get("trilinear-4px", {}).get(pose, {}).get("scores", {}).get("frame", {}).get("rms", 1.0)
            if rms > bound:
                failures.append(f"{path} trilinear-4px {pose} rms {rms:.4f}")
    return failures


def plan(args):
    """One lease running a JSON list of steps in order: {name, binary, sections, configs,
    visits, gate, shaders, env}. `shaders` names a directory of GI shader variants
    (RENDERLAB_GI_SHADERS, read by test builds); `env` adds variables. A step with gate
    "functional" stops the plan (and the lease) when its gates fail. Each step writes
    runs/<label>/<name>. Only `--quiet` plans announce a QUIET window."""
    import datetime as dt
    import re
    import time
    u = ue()
    spec = json.loads(Path(args.plan).read_text())
    # A plan is a list of steps, or {"steps": [...], "released_after": {owner: iso}}: each
    # owner must post a release line stamped after that time (the root's order).
    steps = spec["steps"] if isinstance(spec, dict) else spec
    for owner, since in (spec.get("released_after", {}) if isinstance(spec, dict) else {}).items():
        since = dt.datetime.fromisoformat(since.replace("Z", "+00:00"))
        stamps = [dt.datetime.fromisoformat(s) for s in re.findall(
            rf"(?m)^{re.escape(owner)} .*window released (\d{{4}}-\d\d-\d\dT[\d:.]+\+00:00)",
            u.SCHEDULE.read_text(encoding="utf-8"))]
        if not any(s > since for s in stamps):
            print(f"DEFERRED: waiting for {owner}'s release after {since.isoformat()}")
            raise SystemExit(3)
    # Queued quiet owners ahead of this one: each must post a completion line after our
    # own queue line, starting with its name ("[dl3 QUIET completed]", "DL3 QUIET completed
    # handoff ...", "[m7-u0b done]").
    lines = u.SCHEDULE.read_text(encoding="utf-8").splitlines()
    mine = max((i for i, l in enumerate(lines) if l.startswith(f"[{u.OWNER} quiet reservation]")), default=0)
    done = re.compile(r"QUIET completed|\b(completed?|done|finished)\]", re.IGNORECASE)
    for token in filter(None, args.after.split(",")):
        start = re.compile(rf"\[?{re.escape(token)}\b", re.IGNORECASE)
        if not any(start.match(l) and done.search(l) for l in lines[mine + 1:]):
            print(f"DEFERRED: waiting for {token} completion")
            raise SystemExit(3)
    blocking = window_open(u)
    if blocking and not args.force_schedule:
        print("DEFERRED: schedule:", blocking)
        raise SystemExit(3)
    if hold_state(u) == "hold":
        print("DEFERRED: game hold")
        raise SystemExit(3)
    out = OUT / "runs" / args.label
    if out.exists():
        raise SystemExit(f"label {args.label} already used")
    if args.check_only:
        print("CLEAR: the plan could start now")
        return
    base_env = dict(os.environ, RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK", RENDERLAB_M7_G1_REFS=str(OUT / "refs"),
                    RENDERLAB_M7_G1_BENCH=str(BENCH), RENDERLAB_M7_G1_SPP=str(args.spp), RENDERLAB_M7_G1_QUICK="0",
                    RENDERLAB_M7_G1_PATHS=args.paths, VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation")
    base_env.pop("VK_INSTANCE_LAYERS", None)
    log = []
    with u.lease(UNIT[3:] + "-" + args.label, args.minutes) as owner:
        out.mkdir(parents=True)
        now = u.utc()
        if args.quiet:
            with u.SCHEDULE.open("a", encoding="utf-8") as f:
                f.write(f"\n[{u.OWNER} QUIET active] owner={u.OWNER} label={args.label} start={now.isoformat()} "
                        f"end={(now + dt.timedelta(minutes=args.minutes)).isoformat()}; no other GPU lease or compiler/heavy "
                        "build inside. Headless only: no window, focus, input or capture.\n")
        try:
            for step in steps:
                target = out / step["name"]
                target.mkdir()
                env = dict(base_env, RENDERLAB_M7_G1_OUT=str(target), RENDERLAB_M7_G1_VISITS=str(step.get("visits", 1)))
                for key, var in [("sections", "RENDERLAB_M7_G1_SECTIONS"), ("configs", "RENDERLAB_M7_G1_CONFIGS"),
                                 ("shaders", "RENDERLAB_GI_SHADERS")]:
                    if step.get(key):
                        env[var] = step[key]
                env.update({k: str(v) for k, v in step.get("env", {}).items()})
                start = time.time()
                binary = BINARY.with_name(step["binary"] + ".exe")
                # A "lib" step runs the whole non-ignored lib suite (`cargo test -p renderlab
                # --lib`): renderlab test binaries run only inside a lease holding the lock. A
                # "test" step runs the non-ignored tests matching a filter (e.g. the naga checks
                # of a shader variant). A "command" step runs a list ({binary} replaced), e.g.
                # an Nsight GPU trace of the binary; "kill_after" names an image to stop after it,
                # and "copy_binary" stages the binary as bin/g1b-trace.exe (the traced image).
                if step.get("command"):
                    command = [str(c).replace("{binary}", str(binary)) for c in step["command"]]
                    if step.get("copy_binary"):
                        shutil.copy2(binary, BINARY.with_name("g1b-trace.exe"))
                elif step.get("test"):
                    command = [binary, step["test"], "--test-threads=2"]
                else:
                    command = ([binary, "--test-threads=2"] if step.get("lib") else
                               [binary, step.get("test_name", TEST), "--ignored", "--exact", "--nocapture",
                                "--test-threads=1"])
                try:
                    u.guarded(command, target / "run", timeout=max(60, int(u.remaining_seconds(owner)) - 30),
                              cwd=REPO, env=env)
                finally:
                    if step.get("kill_after"):
                        subprocess.run(["taskkill", "/IM", step["kill_after"], "/F", "/T"], capture_output=True)
                entry = dict(step=step["name"], seconds=round(time.time() - start, 1))
                if step.get("lib"):
                    process = json.loads((target / "run" / "process.json").read_text())
                    if process.get("exit_code") != 0:
                        entry["failures"] = [f"lib suite exit {process.get('exit_code')}"]
                if step.get("gate") in ("functional", "determinism"):
                    entry["failures"] = functional_gate(json.loads((target / "g1-sv.json").read_text()),
                                                        full=step["gate"] == "functional")
                log.append(entry)
                print(json.dumps(entry))
                if entry.get("failures"):
                    break
        finally:
            (out / "plan.json").write_text(json.dumps(dict(steps=steps, log=log), indent=2))
            (out / "contention.json").write_text(json.dumps(dict(hold=hold_state(u)), indent=2))
    failed = [e for e in log if e.get("failures")]
    done = len(log) == len(steps) and not failed
    kind = "QUIET" if args.quiet else "plan"
    with u.SCHEDULE.open("a", encoding="utf-8") as f:
        f.write(f"\n[{u.OWNER} {kind} {'completed' if done else 'stopped'}] label={args.label} "
                f"{len(log)}/{len(steps)} steps{'; gates failed, lease released early' if failed else ''}. "
                f"{u.utc().isoformat()}\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("build")
    p.add_argument("--as", dest="name", default=BINARY.stem, help="staged name under bin/ (variants for a paired A/B)")
    p.add_argument("--beside-leases", action="store_true", help="start beside other owners' non-quiet leases")
    p.add_argument("--release", action="store_true", help="optimized test binary (quarry runs)")
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
    p = sub.add_parser("plan")
    p.add_argument("label")
    p.add_argument("plan", help="JSON list of steps")
    p.add_argument("--minutes", type=int, default=14)
    p.add_argument("--spp", type=int, default=2048)
    p.add_argument("--paths", default="rt,nonrt")
    p.add_argument("--after", default="", help="comma list of owners whose completion tag must follow our queue line")
    p.add_argument("--check-only", action="store_true", help="exit 0 if the plan could start now, 3 if deferred")
    p.add_argument("--quiet", action="store_true", help="announce a QUIET window (final cost claims only)")
    p.add_argument("--force-schedule", action="store_true", help="ignore pending requests (root-assigned slot)")
    args = parser.parse_args()
    dict(build=build, gpu=gpu, plan=plan)[args.command](args)


if __name__ == "__main__":
    main()
