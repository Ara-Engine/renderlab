"""M7-ES1: build and run the occluded environment-specular measurements (docs/m7-es1.md).

  build LABEL          cargo test --no-run, two jobs, own target on D:; stops for another
                       owner's QUIET window. Never inside a lease. Copies the test binary to
                       E:/renderlab-work/m7-es1/bin/LABEL.exe.
  gpu LABEL STEPS...   one shared GPU lease for the listed steps, taken at the first free
                       moment (the lock polled in-process); STEP is PATH:TEST[:SECTIONS],
                       PATH rt or nonrt; TEST plane, quarry, lib (the non-ignored lib
                       suite) or an exact suite test name (tests::..., on that path's suite
                       device: traced or raster). PATH:quarry:SECTIONS:VARIANT reads the
                       shaders in OUT/variants/VARIANT.
                       --validate enables Khronos validation with an owned empty implicit
                       layer directory (as MAT1).

Leases reuse the shared lock, schedule lines and game/RAM/foreign-renderer guards of
m7-u0-ue.py (through m7-u0b.py), as MAT1 does. No window opens. Evidence goes to
E:/renderlab-work/m7-es1/runs/LABEL.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

REPO = Path(__file__).resolve().parent.parent
OUT = Path("E:/renderlab-work/m7-es1")
TARGET = Path("D:/Coding/renderlab-work/m7-es1/target")
LOCK = Path("D:/Coding/renderlab-work/gpu-slot.lock")
TESTS = {
    "plane": "tests::environment_tests::specular::gpu_es1_sky_plane",
    "quarry": "tests::environment_tests::specular::gpu_es1_quarry",
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
    g.OWNER = "m7-es1"
    g.OUT = OUT
    return g


def schedule_read_retry(fn, *args, **kwargs):
    """Another owner may hold the schedule open for an append: read again."""
    while True:
        try:
            return fn(*args, **kwargs)
        except PermissionError:
            time.sleep(0.2)


def build(args):
    runner = load("m7-g1-run")
    g = guard()
    command = ["cargo", "test", "-p", "renderlab", "--lib", "--no-run", "-j", "2",
               "--target-dir", str(TARGET), "--message-format=json"]
    if (line := schedule_read_retry(runner.quiet_active, g, any_lease=False)):
        print(f"DEFERRED: quiet window: {line[:160]}")
        raise SystemExit(3)
    OUT.mkdir(parents=True, exist_ok=True)
    with (OUT / "build.jsonl").open("w") as log, (OUT / "build.err").open("w") as err:
        proc = subprocess.Popen(command, cwd=REPO, stdout=log, stderr=err, text=True)
        while proc.poll() is None:
            time.sleep(5)
            if (line := schedule_read_retry(runner.quiet_active, g, any_lease=False)):
                subprocess.run(["taskkill", "/T", "/F", "/PID", str(proc.pid)], capture_output=True)
                proc.wait()
                for stale in (TARGET / "debug" / "incremental").glob("renderlab-*"):
                    shutil.rmtree(stale, ignore_errors=True)
                print(f"STOPPED for quiet window: {line[:160]}")
                raise SystemExit(3)
    if proc.returncode:
        raise SystemExit((OUT / "build.err").read_text()[-4000:])
    artifacts = [json.loads(l) for l in (OUT / "build.jsonl").read_text().splitlines() if l.startswith("{")]
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
    g.schedule_line(f"[m7-es1 request] owner=m7-es1 asks for one <={args.minutes} min lease \"es1-{args.label}\" "
                    f"({', '.join(args.steps)}; headless, functional, no timing claim unless labelled quiet); "
                    f"takes the lock at the next free moment. {g.utc().isoformat()}")
    deadline = time.time() + args.wait * 60
    # --quiet (final cost claims only): start after IDLE s without a foreign stillkit
    # compiler, announce a QUIET window, and record any stillkit sample inside it (G1c).
    busy, last_busy = [], [time.time() if args.quiet else 0.0]
    if args.quiet:
        import threading
        query = ("@(Get-CimInstance Win32_Process | Where-Object { $_.Name -match '^(rustc|cargo|clippy-driver|rustdoc)[.]exe$' "
                 "-and $_.CommandLine -match 'stillkit' }).Count")

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
                run_steps(g, out, binary, args)
                if args.quiet:
                    seen = [b for b in busy if b >= start - 3]
                    (out / "contention.json").write_text(json.dumps(dict(
                        stillkit_samples=len(seen), idle_before_seconds=args.idle,
                        timings="clean of stillkit" if not seen else "contended: stillkit compiled inside"), indent=2))
                    g.schedule_line(f"[m7-es1 QUIET completed] label=es1-{args.label} "
                                    f"({'clean' if not seen else f'stillkit seen {len(seen)}x'}). {g.utc().isoformat()}")
                return
            except SystemExit as e:
                if e.code != 3:
                    raise
        if time.time() > deadline:
            raise SystemExit("lease wait timed out")
        time.sleep(0.05)


def run_steps(g, out, binary, args):
    base = dict(os.environ, RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
                RENDERLAB_ES1_BASELINE=str(OUT / "baseline-standard.wgsl"),
                RENDERLAB_ES1_MAP="D:/Coding/renderlab-work/m5-independent-production/sun128.rle2",
                RENDERLAB_ES1_QUARRY=str(OUT / "quarry-manifest.json"),
                VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation",
                # Inputs of the retained M5 environment gates (docs/m5-environment-*.md).
                RENDERLAB_ENVIRONMENT_FIELD="D:/Coding/renderlab-work/m5-independent-production/sun128.rle2",
                RENDERLAB_ENVIRONMENT_FIELD_REFERENCES="D:/Coding/renderlab-work/m5-gpu-preparation/references/a-screen.json",
                RENDERLAB_ENVIRONMENT_QUALITY="D:/Coding/renderlab-work/m5-environment-runtime/sun64.rle",
                RENDERLAB_NORMAL_FIXTURE_DIR="D:/Coding/renderlab-work/m5-normal-correction/fixture")
    base.pop("VK_INSTANCE_LAYERS", None)
    if args.validate:
        empty = OUT / "empty-implicit-layers"
        empty.mkdir(exist_ok=True)
        base.pop("VK_LOADER_LAYERS_DISABLE", None)
        base["VK_IMPLICIT_LAYER_PATH"] = str(empty)
    with g.lease("es1-" + args.label, args.minutes) as owner:
        out.mkdir(parents=True)
        if args.quiet:
            import datetime as dt
            now = g.utc()
            g.schedule_line(f"[m7-es1 QUIET active] owner=m7-es1 label=es1-{args.label} start={now.isoformat()} "
                            f"end={(now + dt.timedelta(minutes=args.minutes)).isoformat()}; no other GPU lease or "
                            "compiler/heavy build inside. Headless only: no window, focus, input or capture.")
        for index, step in enumerate(args.steps):
            path, test, *extra = step.replace("::", "@").split(":")
            test = test.replace("@", "::")
            env = dict(base, RENDERLAB_ES1_PATH=path, RENDERLAB_ES1_OUT=str(out),
                       RENDERLAB_TEST_PRIMARY="traced" if path == "rt" else "raster",
                       RENDERLAB_EVIDENCE_DIR=str(out / "fixtures"))
            if extra and extra[0]:
                env["RENDERLAB_ES1_SECTIONS"] = extra[0]
            # A fourth field names a shader variant directory, OUT/variants/NAME, read in
            # place of the built-in GI/ES1 shaders (src/program.rs gi_shader).
            if len(extra) > 1:
                env["RENDERLAB_GI_SHADERS"] = str(OUT / "variants" / extra[1])
                env["RENDERLAB_ES1_TAG"] = extra[1]
            if args.repeat:
                env["RENDERLAB_ES1_REPEAT"] = "1"
            if args.variants:
                env["RENDERLAB_ES1_VARIANTS"] = ";".join(
                    f"{v}={OUT / 'variants' / (v + '.wgsl')}" for v in args.variants.split(","))
            if test == "quarry":
                manifest = json.loads((OUT / "quarry-manifest.json").read_text())
                manifest["out"] = str(out)
                (out / "quarry-manifest.json").write_text(json.dumps(manifest, indent=2))
                env["RENDERLAB_ES1_QUARRY"] = str(out / "quarry-manifest.json")
            # "lib" runs the whole non-ignored lib suite (`cargo test -p renderlab --lib`):
            # renderlab test binaries run only inside a lease holding the lock.
            name = TESTS.get(test, test)
            command = ([binary, "--test-threads=2"] if test == "lib" else
                       [binary, name, "--ignored", "--exact", "--nocapture", "--test-threads=1"])
            g.guarded(command, out / f"step-{index}-{path}-{name.rsplit('::', 1)[-1]}",
                      timeout=max(60, int(g.remaining_seconds(owner)) - 20), cwd=REPO, env=env)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("build")
    p.add_argument("label")
    p.set_defaults(run=build)
    p = sub.add_parser("gpu")
    p.add_argument("label")
    p.add_argument("steps", nargs="+")
    p.add_argument("--binary", required=True)
    p.add_argument("--minutes", type=int, default=10)
    p.add_argument("--wait", type=float, default=60, help="minutes to wait for a free lock")
    p.add_argument("--validate", action="store_true")
    p.add_argument("--quiet", action="store_true", help="announce a QUIET window (final cost claims only)")
    p.add_argument("--idle", type=float, default=0, help="seconds without a stillkit compiler before starting")
    p.add_argument("--repeat", action="store_true", help="render the baseline again after ES1")
    p.add_argument("--variants", help="comma-separated standard-program variants OUT/variants/NAME.wgsl")
    p.set_defaults(run=gpu)
    args = parser.parse_args()
    args.run(args)


if __name__ == "__main__":
    main()
