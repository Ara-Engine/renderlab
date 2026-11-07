"""M7-U0b: build, run and cross-check the reference path tracer.

  build      cargo test --no-run (own target on E:, -j 2); copies the test binary. Never inside a lease.
  gpu        one shared GPU lease (m7-u0-ue.py lease/guard): analytic gates, then the benches.
  mitsuba    CPU: Mitsuba 3 renders of the cross-check scenes, two seeds each (>= 8 GiB available).
  compare    CPU: scripts/m7-u0-crosscheck.py compare for each cross-check scene.

Evidence goes to E:/renderlab-work/m7-u0/reference/<label>.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess

REPO = Path(__file__).resolve().parent.parent
OUT = Path("E:/renderlab-work/m7-u0")
TARGET = OUT / "target"
BINARY = OUT / "bin/reference-tests.exe"
SCENES = OUT / "scenes"
MITSUBA = OUT / "tools/mitsuba-venv/Scripts/python.exe"
CROSS = ("X1", "X2")


def ue():
    spec = importlib.util.spec_from_file_location("m7_u0_ue", REPO / "scripts/m7-u0-ue.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
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


def gpu(args):
    u = ue()
    out = OUT / "reference" / args.label
    out.mkdir(parents=True, exist_ok=False)
    benches = ";".join(f"{SCENES / name}@{spp}" for name, spp in (b.split("@") for b in args.benches))
    env = dict(os.environ, RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
               RENDERLAB_M7_REFERENCE_OUT=str(out), RENDERLAB_M7_BENCHES=benches, VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation")
    env.pop("VK_INSTANCE_LAYERS", None)
    tests = ["reference::tests::gpu_reference_analytic", "reference::tests::gpu_reference_benches"]
    with u.lease("reference-" + args.label, args.minutes) as owner:
        for test in tests:
            u.guarded([BINARY, test, "--ignored", "--exact", "--nocapture", "--test-threads=1"], out / test.rsplit("::", 1)[1],
                      timeout=max(60, int(u.remaining_seconds(owner)) - 30), cwd=REPO, env=env)
    print(json.dumps(sorted(p.name for p in out.iterdir()), indent=1))


def mitsuba(args):
    u = ue()
    if u.E2E.available_gib() < 8:
        raise SystemExit("deferred: Mitsuba CPU renders start only with >= 8 GiB available")
    out = OUT / "mitsuba"
    out.mkdir(exist_ok=True)
    for name in args.scenes:
        for seed in (1, 2):
            target = out / f"{name}-seed{seed}.npy"
            if target.exists():
                continue
            subprocess.run([str(MITSUBA), "-I", str(REPO / "scripts/m7-u0-crosscheck.py"), "mitsuba", str(SCENES / name), str(target),
                            "--spp", str(args.spp), "--seed", str(seed)], check=True, creationflags=subprocess.CREATE_NO_WINDOW)


def compare(args):
    out = OUT / "reference" / args.label
    for name in args.scenes:
        pose = json.loads((SCENES / name / "scene.json").read_text())["poses"][0]["name"]
        subprocess.run(["py", "-3", "-B", str(REPO / "scripts/m7-u0-crosscheck.py"), "compare", str(out), f"{name}-{pose}",
                        str(OUT / "mitsuba" / f"{name}-seed1.npy"), str(OUT / "mitsuba" / f"{name}-seed2.npy")], check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("build")
    p = sub.add_parser("gpu")
    p.add_argument("label")
    p.add_argument("--benches", nargs="+", default=["X1@32768", "X2@32768", "SV@256", "S4@256", "S5@256", "S2G@64"])
    p.add_argument("--minutes", type=int, default=20)
    p = sub.add_parser("mitsuba")
    p.add_argument("--scenes", nargs="+", default=list(CROSS))
    p.add_argument("--spp", type=int, default=2048)
    p = sub.add_parser("compare")
    p.add_argument("label")
    p.add_argument("--scenes", nargs="+", default=list(CROSS))
    args = parser.parse_args()
    dict(build=build, gpu=gpu, mitsuba=mitsuba, compare=compare)[args.command](args)


if __name__ == "__main__":
    main()
