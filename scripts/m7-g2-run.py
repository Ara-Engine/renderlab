"""M7-G2: build, check and run the bounce-light measurements (docs/m7-g2.md).

  check [--clippy] [--fmt]  cargo check (or clippy) of the renderlab lib and tests, two jobs,
                            own target on D:. Stops for another owner's QUIET window. Never
                            inside a lease. CPU only; no renderlab binary runs.
  build LABEL               cargo test --no-run, two jobs, own target on D:; stops for another
                            owner's QUIET window. Copies the test binary to OUT/bin/LABEL.exe.
  gpu LABEL STEPS...        one shared GPU lease for the listed steps, taken at the first free
                            moment (the lock polled in-process); STEP is
                            PATH:TEST[:SECTIONS[:VARIANT[:SETTINGS]]], PATH rt or nonrt; TEST quarry
                            (gpu_g2_quarry), trace (an Nsight GPU trace of its `trace` section), sv
                            (G1's SV harness), lib (the non-ignored lib suite) or an exact suite test
                            name. VARIANT reads the shaders in OUT/variants/VARIANT
                            (RENDERLAB_GI_SHADERS); SETTINGS, comma-separated, replaces --variants
                            (RENDERLAB_G2_VARIANTS) for that step.

Leases reuse the shared lock, schedule lines and game/RAM/foreign-renderer guards of
m7-u0-ue.py (through m7-u0b.py), as ES1 does. No window opens. Evidence goes to
D:/Coding/renderlab-work/m7-g2/runs/LABEL (Islam, 2026-11-08: new bulk output on D:).
"""
import argparse
import importlib.util
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import time

REPO = Path(__file__).resolve().parent.parent
OUT = Path("D:/Coding/renderlab-work/m7-g2")
# The unit that owns leases and schedule lines, and its lease-label prefix. scripts/m7-g2b-run.py
# reuses this runner with its own unit and output root (docs/m7-g2b.md).
UNIT = "m7-g2"
PREFIX = "g2-"
TARGET = OUT / "target"
LOCK = Path("D:/Coding/renderlab-work/gpu-slot.lock")
NGFX = "C:/Program Files/NVIDIA Corporation/Nsight Graphics 2026.4.0/host/windows-desktop-nomad-x64/ngfx.exe"
TESTS = {
    "quarry": "renderer::gi_cache::gpu_tests::gpu_g2_quarry",
    "sv": "renderer::gi_cache::gpu_tests::gpu_gi_sky_visibility_sv",
}


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), REPO / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def guard():
    host = load("m7-u0b")
    host.OUT = OUT
    g = host.ue()
    g.OWNER = UNIT
    g.OUT = OUT
    return g


def schedule_read_retry(fn, *args, **kwargs):
    """Another owner may hold the schedule open for an append: read again."""
    while True:
        try:
            return fn(*args, **kwargs)
        except PermissionError:
            time.sleep(0.2)


def cargo(args, command, log_name):
    """Runs a Cargo command with two jobs, stopping it for another owner's QUIET window."""
    runner = load("m7-g1-run")
    g = guard()
    if (line := schedule_read_retry(runner.quiet_active, g, any_lease=False)):
        print(f"DEFERRED: quiet window: {line[:160]}")
        raise SystemExit(3)
    if schedule_read_retry(runner.hold_state, g) == "hold":
        print("note: game hold in force; CPU-only Cargo with two jobs continues")
    OUT.mkdir(parents=True, exist_ok=True)
    with (OUT / f"{log_name}.out").open("w") as log, (OUT / f"{log_name}.err").open("w") as err:
        proc = subprocess.Popen(command, cwd=REPO, stdout=log, stderr=err, text=True)
        while proc.poll() is None:
            time.sleep(3)
            if (line := schedule_read_retry(runner.quiet_active, g, any_lease=False)):
                subprocess.run(["taskkill", "/T", "/F", "/PID", str(proc.pid)], capture_output=True)
                proc.wait()
                print(f"STOPPED for quiet window: {line[:160]}")
                raise SystemExit(3)
    return proc.returncode


def check(args):
    tool = "clippy" if args.clippy else "check"
    command = ["cargo", tool, "-p", "renderlab", "--lib", "--tests", "-j", "2",
               "--target-dir", str(TARGET), "--message-format=short"]
    code = cargo(args, command, tool)
    err = (OUT / f"{tool}.err").read_text(errors="replace")
    lines = [l for l in err.splitlines() if ("error" in l or "warning" in l)]
    print("\n".join(lines[-80:]))
    if args.fmt:
        fmt = subprocess.run(["cargo", "fmt", "-p", "renderlab", "--", "--check"], cwd=REPO,
                             capture_output=True, text=True)
        print("fmt exit", fmt.returncode, fmt.stdout[-3000:])
    raise SystemExit(code)


def build(args):
    command = ["cargo", "test", "-p", "renderlab", "--lib", "--no-run", "-j", "2",
               "--target-dir", str(TARGET), "--message-format=json"]
    code = cargo(args, command, "build")
    if code:
        raise SystemExit((OUT / "build.err").read_text(errors="replace")[-4000:])
    artifacts = [json.loads(l) for l in (OUT / "build.out").read_text().splitlines() if l.startswith("{")]
    exe = next(a["executable"] for a in reversed(artifacts) if a.get("reason") == "compiler-artifact"
               and a.get("executable") and a["target"]["name"] == "renderlab")
    staged = OUT / "bin" / (args.label + ".exe")
    staged.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(exe, staged)
    print("binary", staged)


def gpu(args):
    runner = load("m7-g1-run")
    g = guard()
    out = OUT / "runs" / args.label
    if out.exists():
        raise SystemExit(f"label {args.label} already used")
    binary = OUT / "bin" / (args.binary + ".exe")
    g.schedule_line(f"[{UNIT} request] owner={UNIT} asks for one <={args.minutes} min lease \"{PREFIX}{args.label}\" "
                    f"({', '.join(args.steps)}; headless, {'QUIET timing' if args.quiet else 'functional, no timing claim'}); "
                    f"takes the lock at the next free moment. {g.utc().isoformat()}")
    deadline = time.time() + args.wait * 60
    # --quiet (cost claims only): start after IDLE s without a foreign compiler, announce a
    # QUIET window, and classify every step by the compiler samples inside it (G1c, G1d).
    busy, last_busy = [], [time.time() if args.quiet else 0.0]
    if args.quiet:
        import threading
        query = ("@(Get-CimInstance Win32_Process | Where-Object { $_.Name -match '^(rustc|cargo|clippy-driver|rustdoc)[.]exe$' "
                 f"-and $_.CommandLine -notmatch '{UNIT}' }}).Count")

        def sampler():
            while True:
                n = subprocess.run(["powershell", "-NoProfile", "-Command", query], capture_output=True, text=True).stdout
                if int(n.strip() or "0"):
                    last_busy[0] = time.time()
                    busy.append(time.time())
                time.sleep(3)
        threading.Thread(target=sampler, daemon=True).start()
    while True:
        free = not LOCK.exists() and not schedule_read_retry(runner.quiet_active, g, any_lease=True)
        idle = time.time() - last_busy[0] >= args.idle
        if free and idle and schedule_read_retry(runner.hold_state, g) != "hold":
            try:
                start = time.time()
                steps = run_steps(g, out, binary, args)
                if args.quiet:
                    classes = [dict(step=s, start=a, end=b, compilers=len([t for t in busy if a - 3 <= t <= b + 3]))
                               for s, a, b in steps]
                    seen = [b for b in busy if b >= start - 3]
                    (out / "contention.json").write_text(json.dumps(dict(
                        compiler_samples=len(seen), idle_before_seconds=args.idle, steps=classes,
                        timings="clean of foreign compilers" if not seen else "contended: a compiler ran inside"), indent=2))
                    g.schedule_line(f"[{UNIT} QUIET completed] label={PREFIX}{args.label} "
                                    f"({'clean' if not seen else f'compiler seen {len(seen)}x'}). {g.utc().isoformat()}")
                return
            except SystemExit as e:
                if e.code != 3:
                    raise
        if time.time() > deadline:
            raise SystemExit("lease wait timed out")
        time.sleep(0.05)


def run_steps(g, out, binary, args):
    base = dict(os.environ, RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
                RENDERLAB_G2_QUARRY=str(OUT / "quarry-manifest.json"),
                VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation")
    base.pop("VK_INSTANCE_LAYERS", None)
    for k, v in (args.env or []):
        base[k] = v
    done = []
    with g.lease(PREFIX + args.label, args.minutes) as owner:
        out.mkdir(parents=True)
        if args.quiet:
            import datetime as dt
            now = g.utc()
            g.schedule_line(f"[{UNIT} QUIET active] owner={UNIT} label={PREFIX}{args.label} start={now.isoformat()} "
                            f"end={(now + dt.timedelta(minutes=args.minutes)).isoformat()}; no other GPU lease or "
                            "compiler/heavy build inside. Headless only: no window, focus, input or capture.")
        for index, step in enumerate(args.steps):
            # A short test name (quarry, trace, sv, lib, cpu=...) ends at the next ':'; an exact
            # test name (a::b::c) keeps its '::'. Empty fields are allowed after either.
            path, rest = step.split(":", 1)
            short = rest.split(":")
            if short[0] in TESTS or short[0] in ("trace", "lib") or short[0].startswith("cpu="):
                test, extra = short[0], short[1:]
            else:
                m = re.match(r"(\w+(?:::\w+)*)(?::(.*))?$", rest)
                test, extra = m.group(1), (m.group(2).split(":") if m.group(2) is not None else [])
            env = dict(base, RENDERLAB_G2_PATH=path, RENDERLAB_G2_OUT=str(out),
                       RENDERLAB_TEST_PRIMARY="traced" if path == "rt" else "raster",
                       RENDERLAB_M7_G1_PATHS="rt" if path == "rt" else "nonrt",
                       RENDERLAB_ES1_PATH=path,
                       RENDERLAB_EVIDENCE_DIR=str(out / "fixtures"))
            if extra and extra[0]:
                env["RENDERLAB_G2_SECTIONS"] = extra[0]
                env["RENDERLAB_M7_G1_SECTIONS"] = extra[0]
            if len(extra) > 1 and extra[1]:
                env["RENDERLAB_GI_SHADERS"] = str(OUT / "variants" / extra[1])
                env["RENDERLAB_G2_TAG"] = extra[1]
            # The lease's --variants, or this step's own list (a fifth field, comma-separated).
            if args.variants:
                env["RENDERLAB_G2_VARIANTS"] = args.variants
            if len(extra) > 2 and extra[2]:
                env["RENDERLAB_G2_VARIANTS"] = extra[2]
            name = TESTS.get(test, test)
            traced = None
            # "trace": an Nsight GPU Trace (real-time shader profiler, 200 ms) of the quarry
            # test's `trace` section, the camera moving (G1d's headless recipe: Nsight's layer
            # manifests only, never --platform; the traced image is killed afterwards). Its
            # window starts RENDERLAB_G2_TRACE_START_MS (default 45,000) after launch.
            if test == "trace":
                traced = OUT / "bin" / f"{UNIT}-trace.exe"
                shutil.copy2(binary, traced)
                env["RENDERLAB_G2_SECTIONS"] = extra[0] if extra and extra[0] else "trace"
                env["VK_IMPLICIT_LAYER_PATH"] = "E:/renderlab-work/n8f/layers"
                passed = {k: v for k, v in env.items() if k.startswith(("RENDERLAB_", "VK_LOADER"))}
                step_dir = out / f"trace-{index}-{path}"
                step_dir.mkdir(parents=True, exist_ok=True)
                command = [NGFX, "--activity", "GPU Trace Profiler", "--output-dir", str(step_dir),
                           "--exe", str(traced), "--dir", str(traced.parent),
                           "--args", f"{TESTS['quarry']} --ignored --exact --nocapture --test-threads=1",
                           "--env", "; ".join(f"{k}={v}" for k, v in passed.items()),
                           "--start-after-ms", env.get("RENDERLAB_G2_TRACE_START_MS", "45000"),
                           "--limit-to-submits", "4", "--max-duration-ms", "200",
                           "--set-gpu-clocks", "unaltered", "--collect-screenshot", "0", "--architecture", "Ada",
                           "--metric-set-name", "Top-Level Triage", "--real-time-shader-profiler"]
            # "cpu=FILTER": the non-ignored lib tests whose names contain FILTER (naga checks).
            elif test.startswith("cpu="):
                name = test[4:]
                command = [binary, name, "--test-threads=2"]
            elif test == "lib":
                command = [binary, "--test-threads=2"]
            else:
                command = [binary, name, "--ignored", "--exact", "--nocapture", "--test-threads=1"]
            start = time.time()
            try:
                g.guarded(command, out / f"step-{index}-{path}-{name.rsplit('::', 1)[-1]}",
                          timeout=max(60, int(g.remaining_seconds(owner)) - 20), cwd=REPO, env=env)
            finally:
                if traced:
                    subprocess.run(["taskkill", "/IM", traced.name, "/F", "/T"], capture_output=True)
            done.append((step, start, time.time()))
    return done


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("check")
    p.add_argument("--clippy", action="store_true")
    p.add_argument("--fmt", action="store_true")
    p.set_defaults(run=check)
    p = sub.add_parser("build")
    p.add_argument("label")
    p.set_defaults(run=build)
    p = sub.add_parser("gpu")
    p.add_argument("label")
    p.add_argument("steps", nargs="+")
    p.add_argument("--binary", required=True)
    p.add_argument("--minutes", type=int, default=10)
    p.add_argument("--wait", type=float, default=60, help="minutes to wait for a free lock")
    p.add_argument("--quiet", action="store_true", help="announce a QUIET window (cost claims only)")
    p.add_argument("--idle", type=float, default=0, help="seconds without a foreign compiler before starting")
    p.add_argument("--env", nargs=2, action="append", metavar=("NAME", "VALUE"))
    p.add_argument("--variants", help="RENDERLAB_G2_VARIANTS for quarry steps (default off,g2,es1)")
    p.set_defaults(run=gpu)
    args = parser.parse_args()
    args.run(args)


if __name__ == "__main__":
    main()
