"""M7-MP1: build and run the per-program dispatch checks (docs/m7-mp1.md). R1's runner
(scripts/m7-r1-run.py) under MP1's owner and evidence root: the same shared lock, schedule
lines and game/RAM/foreign-renderer guards. No window opens.

  quiet                another owner's window in force? (exit 1 when one is)
  build LABEL          cargo test --no-run, two jobs, own target on D:; stops for another
                       owner's QUIET window. Never inside a lease. Stages the test binary as
                       OUT/bin/LABEL.exe.
  gpu LABEL STEPS...   one lease for the listed steps, taken at the first free moment (the
                       lock polled in-process). STEP is PATH:TEST[:SECTIONS[:BINARY]], PATH
                       rt or nonrt, BINARY a staged label replacing --binary, TEST one of:
                         mp1       MP1's mixed-program identity and function gate
                         example   API1's game_scene example exe (BINARY), PNG frames
                         lib       the non-ignored lib suite
                         identity  the production mirror-camera identity (rt only)
                         rf        R0a's RF1-RF10 current-renderer capture with this build
                         rftr1     the same after 32 TR1 frames per pose (RENDERLAB_R1_TR1)
                         quarry    ES1's quarry protocol: main's standard program (the
                                   baseline) then this build's; SECTIONS as ES1's
                         or an exact test name.

Evidence goes to D:/Coding/renderlab-work/m7-mp1/runs/LABEL; R0a's fixtures and
references on E: are read only.
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
OUT = Path("D:/Coding/renderlab-work/m7-mp1")
TARGET = OUT / "target"
LOCK = Path("D:/Coding/renderlab-work/gpu-slot.lock")
OWNER = "m7-mp1"
R0A = Path("E:/renderlab-work/m7-r0a")
FIXTURES = [f"RF{i}" for i in range(1, 11)]
TESTS = {
    "identity": "reference::reflection_tests::gpu_r1_mirror_identity",
    "rf": "reference::r0a_tests::gpu_r0a_current",
    "rftr1": "reference::r0a_tests::gpu_r0a_current",
    "quarry": "tests::environment_tests::specular::gpu_es1_quarry",
    "mp1": "renderer::primary_ray_gpu::program_lists::gpu_tests::gpu_mp1_mixed_programs",
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
    g.OWNER = OWNER
    g.OUT = OUT
    return g


def schedule_read_retry(fn, *args, **kwargs):
    """Another owner may hold the schedule open for an append: read again."""
    while True:
        try:
            return fn(*args, **kwargs)
        except PermissionError:
            time.sleep(0.2)


def quiet(args):
    line = schedule_read_retry(load("m7-g1-run").quiet_active, guard(), any_lease=True)
    print(line or "no window in force")
    raise SystemExit(1 if line else 0)


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
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip()
    dirty = subprocess.check_output(["git", "status", "--porcelain"], cwd=REPO, text=True)
    (OUT / "bin" / (args.label + ".json")).write_text(json.dumps(dict(head=head, dirty=dirty.splitlines()), indent=2))
    print("binary", staged)


def gpu(args):
    runner = load("m7-g1-run")
    g = guard()
    out = OUT / "runs" / args.label
    if out.exists():
        raise SystemExit(f"label {args.label} already used")
    binary = OUT / "bin" / (args.binary + ".exe")
    baseline = OUT / "baseline-standard.wgsl"
    if not baseline.exists():
        # The baseline program is main's standard source, before R1's reflection hook.
        text = subprocess.check_output(["git", "show", "origin/main:shaders/programs/standard.wgsl"], cwd=REPO, text=True)
        baseline.write_text(text, encoding="utf-8", newline="\n")
    g.schedule_line(f"[{OWNER} request] owner={OWNER} asks for one <={args.minutes} min lease \"mp1-{args.label}\" "
                    f"({', '.join(args.steps)}; headless, functional, no timing claim); "
                    f"takes the lock at the next free moment. {g.utc().isoformat()}")
    deadline = time.time() + args.wait * 60
    while True:
        free = not LOCK.exists() and not schedule_read_retry(runner.quiet_active, g, any_lease=True)
        if free and schedule_read_retry(runner.hold_state, g) != "hold":
            try:
                run_steps(g, out, binary, baseline, args)
                return
            except SystemExit as e:
                if e.code != 3:
                    raise
        if time.time() > deadline:
            raise SystemExit("lease wait timed out")
        time.sleep(0.05)


def run_steps(g, out, binary, baseline, args):
    base = dict(os.environ, RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
                RENDERLAB_ES1_BASELINE=str(baseline),
                VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation")
    base.pop("VK_INSTANCE_LAYERS", None)
    with g.lease("mp1-" + args.label, args.minutes) as owner:
        # A deferred attempt (another unit's build) leaves only preflight files: reuse it.
        out.mkdir(parents=True, exist_ok=True)
        for index, step in enumerate(args.steps):
            path, test, *extra = step.replace("::", "@").split(":")
            test = test.replace("@", "::")
            env = dict(base, RENDERLAB_ES1_PATH=path, RENDERLAB_ES1_OUT=str(out),
                       RENDERLAB_TEST_PRIMARY="traced" if path == "rt" else "raster",
                       RENDERLAB_EVIDENCE_DIR=str(out / "fixtures"), RENDERLAB_R1_OUT=str(out / "identity"))
            if extra and extra[0]:
                env["RENDERLAB_ES1_SECTIONS"] = extra[0]
            step_binary = OUT / "bin" / (extra[1] + ".exe") if len(extra) > 1 else binary
            if test == "rftoonstd":
                # MP1 diagnostic: R0a's RF10 with its toon material on the standard program.
                test = "rf"
                env["RENDERLAB_R0A_TOON"] = "standard"
            if test in ("rf", "rftr1"):
                tag = path + ("-tr1" if test == "rftr1" else "") + ("-toonstd" if env.get("RENDERLAB_R0A_TOON") else "")
                env.update(RENDERLAB_M7_R0A_SCENES=";".join(str(R0A / "scenes" / f) for f in FIXTURES),
                           RENDERLAB_M7_R0A_PATH=path, RENDERLAB_M7_REFERENCE_OUT=str(out / f"rf-{tag}"),
                           RENDERLAB_R1_TR1="1" if test == "rftr1" else "0")
            if test == "quarry":
                manifest = json.loads(Path("E:/renderlab-work/m7-es1/quarry-manifest.json").read_text())
                manifest["out"] = str(out)
                (out / "quarry-manifest.json").write_text(json.dumps(manifest, indent=2))
                env["RENDERLAB_ES1_QUARRY"] = str(out / "quarry-manifest.json")
                # Each binary's quarry rows in their own directory (ES1's tag).
                env["RENDERLAB_ES1_TAG"] = extra[1] if len(extra) > 1 else args.binary
            name = TESTS.get(test, test)
            command = ([step_binary, "--test-threads=2"] if test == "lib" else
                       [step_binary, name, "--ignored", "--exact", "--nocapture", "--test-threads=1"])
            if test == "example":
                # API1's game_scene example (BINARY: its staged exe), PNG frames per path.
                command = [step_binary, "--path", path, "--out", str(out / f"example-{path}")]
            step_dir = out / f"step-{index}-{path}-{name.rsplit('::', 1)[-1]}"
            if step_dir.exists() and not (step_dir / "process.json").exists():
                shutil.rmtree(step_dir)
            g.guarded(command, step_dir,
                      timeout=max(60, int(g.remaining_seconds(owner)) - 20), cwd=REPO, env=env)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("quiet")
    p.set_defaults(run=quiet)
    p = sub.add_parser("build")
    p.add_argument("label")
    p.set_defaults(run=build)
    p = sub.add_parser("gpu")
    p.add_argument("label")
    p.add_argument("steps", nargs="+")
    p.add_argument("--binary", required=True)
    p.add_argument("--minutes", type=int, default=15)
    p.add_argument("--wait", type=float, default=60, help="minutes to wait for a free lock")
    p.set_defaults(run=gpu)
    args = parser.parse_args()
    args.run(args)


if __name__ == "__main__":
    main()
