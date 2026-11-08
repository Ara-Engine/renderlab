"""DC1: guarded two-job compilation and n8-run.py GPU leases. All new output on D:."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = Path("D:/Coding/renderlab-work/m7-dc1")
TEST = "renderer::gi_cache::gpu_tests::decal_tests::gpu_decals"


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), ROOT / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def guard():
    g = load("m7-u0b").ue()
    g.OWNER = "m7-dc1"
    g.OUT = OUT
    return g


def build(args):
    base = load("m7-g2-run")
    base.OUT = OUT
    base.TARGET = OUT / "target"
    base.guard = guard
    base.build(args)


def batch(args):
    binary = OUT / "bin" / (args.binary + ".exe")
    for label, flags, extra in [
        ("lib", ["--test-threads=1"], {}),
        ("rt", [TEST, "--ignored", "--exact", "--nocapture", "--test-threads=1"], {"RENDERLAB_DC1_PATH": "rt"}),
        ("nonrt", [TEST, "--ignored", "--exact", "--nocapture", "--test-threads=1"], {"RENDERLAB_DC1_PATH": "nonrt"}),
    ]:
        output = OUT / "runs" / args.label / label
        output.mkdir(parents=True, exist_ok=True)
        env = dict(os.environ, **extra, RENDERLAB_DC1_OUT=str(output), RENDERLAB_DC1_QUARRY=str(OUT / "quarry-manifest.json"))
        command = [str(binary), *flags]
        with (output / "test.log").open("w") as log:
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, creationflags=subprocess.CREATE_NO_WINDOW)
        print(label, "exit", result.returncode, flush=True)
        if result.returncode:
            print((output / "test.log").read_text(errors="replace")[-12000:], flush=True)
            raise SystemExit(result.returncode)


def gpu(args):
    g, checks = guard(), load("m7-g1-run")
    g.schedule_line(f"[m7-dc1 request] owner=m7-dc1 asks for <=18 min DC1 {args.label}: lib tests, decal fixture/quarry on RT and non-RT, eight paired timing blocks. Headless; n8-run.py; no superiority claim. {g.utc().isoformat()}")
    deadline = time.monotonic() + args.wait * 60
    attempt = 0
    while time.monotonic() < deadline:
        if checks.quiet_active(g, any_lease=True) or checks.hold_state(g) == "hold" or (Path("D:/Coding/renderlab-work/gpu-slot.lock")).exists():
            time.sleep(3)
            continue
        attempt += 1
        env = dict(os.environ, N8_OWNER="m7-dc1", N8_OUT=str(OUT / "leases"))
        command = [sys.executable, "-B", str(ROOT / "scripts/n8-run.py"), "--label", f"dc1-{args.label}-{attempt}", "--minutes", "18", "--fast-check", "--",
                   sys.executable, "-B", str(Path(__file__)), "batch", args.label, "--binary", args.binary]
        result = subprocess.run(command, cwd=ROOT, env=env)
        if result.returncode == 3:
            time.sleep(3)
            continue
        raise SystemExit(result.returncode)
    raise SystemExit("DC1 lease wait timed out")


if __name__ == "__main__":
    p = argparse.ArgumentParser()
    p.add_argument("command", choices=["build", "gpu", "batch"])
    p.add_argument("label")
    p.add_argument("--binary", default="dc1")
    p.add_argument("--wait", type=float, default=75)
    args = p.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    {"build": build, "gpu": gpu, "batch": batch}[args.command](args)
