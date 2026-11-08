"""PC1 startup measurements. Bulk files and own Cargo target stay on D:.

build [dev|release]   Two Cargo jobs, no GPU; stops for a declared quiet window.
gpu LABEL [dev|release|probe|driver|lib]   Wait in-turn, then one <=15 minute n8 lease.
The cache-empty/cold/warm labels describe our cache, not NVIDIA's private cache.
"""
import argparse
import datetime
import hashlib
import importlib.util
import json
import os
import re
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = Path("D:/Coding/renderlab-work/pc1")
TARGET = OUT / "target"
SCHEDULE = Path("E:/renderlab-work/gpu-schedule.md")
FLAGS = subprocess.CREATE_NO_WINDOW


def schedule_text():
    for _ in range(100):
        try: return SCHEDULE.read_text(encoding="utf-8")
        except PermissionError: time.sleep(.2)
    raise RuntimeError("schedule remained locked")


def order_context():
    text = schedule_text()
    orders = list(re.finditer(r"^\[root order\] .+$", text, re.MULTILINE))
    if not orders: raise RuntimeError("no root order found; review the schedule")
    return text[orders[-1].start():]


def latest_order():
    return order_context().splitlines()[0]


def yield_requested(out, step):
    order = latest_order()
    if os.environ.get("PC1_LEASE_ORDER", order) == order: return False
    (out / "yield.json").write_text(json.dumps(dict(next_step=step, order=order), indent=2))
    return True


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), ROOT / "scripts" / f"{name}.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def build(profile):
    OUT.mkdir(parents=True, exist_ok=True)
    guard = load("m7-mp1-run").guard()
    guard.OWNER = "m7-pc1"
    quiet = load("m7-g1-run").quiet_active
    command = ["cargo", "build", "-p", "renderlab", "--example", "game_scene", "-j", "2"]
    if profile == "release": command.append("--release")
    env = dict(os.environ, CARGO_TARGET_DIR=str(TARGET), CARGO_BUILD_JOBS="2")
    for kind, cmd in [("example", command), ("lib", ["cargo", "test", "-p", "renderlab", "--lib", "--no-run", "-j", "2"])] if profile == "dev" else [("example", command)]:
        while quiet(guard, any_lease=False): time.sleep(5)
        started = time.monotonic()
        log = OUT / f"build-{profile}-{kind}.log"
        with log.open("w") as output:
            child = subprocess.Popen(cmd, cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT, creationflags=FLAGS)
            while child.poll() is None:
                time.sleep(5)
                if quiet(guard, any_lease=False):
                    subprocess.run(["taskkill", "/PID", str(child.pid), "/T", "/F"], capture_output=True, creationflags=FLAGS)
                    raise RuntimeError("build stopped for quiet window; rerun to resume warm target")
        if child.returncode: raise RuntimeError(log.read_text()[-10000:])
        print(f"built {profile} {kind} in {time.monotonic()-started:.1f}s", flush=True)


def batch(label, profile):
    if profile == "final":
        for item in ("dev", "release", "probe", "driver"):
            code = batch(f"{label}-{item}", item)
            if code: return code
        return 0
    out = OUT / "runs" / label
    out.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, RENDERLAB_STARTUP_PROFILE="1",
               VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation")
    env.pop("VK_INSTANCE_LAYERS", None)
    if profile in ("lib", "dev"):
        binaries = list((TARGET / "debug/deps").glob("renderlab-*.exe"))
        binary = max(binaries, key=lambda p:p.stat().st_mtime)
        log_path = out / "lib.log"
        reuse = (log_path.exists() and binary.stat().st_mtime < log_path.stat().st_mtime
                 and "test result: ok." in log_path.read_text())
        if not reuse:
            if yield_requested(out, "lib"): return 75
            with log_path.open("w") as log:
                result = subprocess.run([str(binary), "--test-threads=2"], cwd=ROOT, env=env,
                                        stdout=log, stderr=subprocess.STDOUT, creationflags=FLAGS)
            if result.returncode: return result.returncode
        print((out / "lib.log").read_text()[-2000:], flush=True)
        if profile == "lib": return 0
    binary = TARGET / ("release" if profile == "release" else "debug") / "examples/game_scene.exe"
    manifest = json.loads(Path("E:/renderlab-work/m7-es1/quarry-manifest.json").read_text())
    manifest["limits"] = "D:/Coding/projects/desktop/renderlab/measurements/quarry-tuning/detail/limits-r128-u64-6e417dc13431.json"
    # Prefer API1's actual policy path if the scan policy moved.
    if not Path(manifest["limits"]).exists():
        candidates = list(Path("D:/Coding/projects/desktop/renderlab").glob("**/*limits*.json"))
        raise RuntimeError(f"set quarry limits path; candidates: {candidates[:20]}")
    quarry = out / "quarry.json"
    quarry.write_text(json.dumps(manifest, indent=2))
    binary_sha = hashlib.file_digest(binary.open("rb"), "sha256").hexdigest()
    if (out / "binary.json").exists() and json.loads((out / "binary.json").read_text())["sha256"] != binary_sha:
        raise RuntimeError("refusing to mix binaries in a resumed measurement")
    (out / "binary.json").write_text(json.dumps(dict(path=str(binary), sha256=binary_sha,
        git_head=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()), indent=2))
    rows = json.loads((out / "summary.json").read_text()) if (out / "summary.json").exists() else []
    scenes = ["game"] if profile in ("probe", "driver") else ["game", "quarry"]
    for scene in scenes:
        for path in ("rt", "nonrt"):
            cache = out / f"cache-{scene}-{path}"
            modes = ["probe"] if profile == "probe" else (["off", "cold", "warm", "corrupt"] if profile == "driver" else ["off", "cold", "warm"])
            for mode in modes:
                if any(r["scene"] == scene and r["path"] == path and r["mode"] == mode for r in rows): continue
                directory = out / f"{scene}-{path}-{mode}"
                command = [str(binary), "--path", path, "--out", str(directory), "--frames", "2", "--every", "1",
                           "--size", "1280x720" if scene == "game" else "1920x1080"]
                command += ["--no-pipeline-cache"] if mode in ("off", "driver") else ["--pipeline-cache", str(cache)]
                if scene == "quarry": command += ["--quarry", str(quarry)]
                if profile == "driver": env.update(RENDERLAB_STARTUP_PROFILE_DRIVER="1", RENDERLAB_RELOAD_PROFILE="1")
                if profile == "probe": command += ["--reload-probe", "--no-temporal", "--no-gi"]
                completed = (directory / "run.json").exists()
                if not completed and yield_requested(out, f"{scene}:{path}:{mode}"): return 75
                if not completed and mode == "cold" and cache.exists():
                    # A killed cold process can leave partial cache data. Remove only
                    # this run's own cache files before retrying an empty-cache launch.
                    if not cache.resolve().is_relative_to(OUT.resolve()): raise RuntimeError("cache escaped PC1 output")
                    for file in cache.iterdir(): file.unlink()
                    cache.rmdir()
                if not completed and mode == "corrupt":
                    files = list(cache.glob("*.bin"))
                    if len(files) != 1: raise RuntimeError("expected one populated driver cache")
                    data = bytearray(files[0].read_bytes())
                    if len(data) < 41: raise RuntimeError("cache was not populated")
                    data[40] ^= 1
                    files[0].write_bytes(data)
                process_seconds = None
                if not completed:
                    started = time.monotonic()
                    with (out / f"{scene}-{path}-{mode}.log").open("w") as log:
                        result = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, creationflags=FLAGS, timeout=180)
                    if result.returncode: raise RuntimeError(f"{command}: exit {result.returncode}")
                    process_seconds = time.monotonic()-started
                data = json.loads((directory / "run.json").read_text())
                row = dict(scene=scene, path=path, mode=mode, binary_sha256=binary_sha, process_seconds=process_seconds,
                           first_image_seconds=data["startup_to_first_image_seconds"], scene_seconds=data["startup_to_scene_ready_seconds"],
                           hashes=[f["rgba8_sha256"] for f in data["frames"]], reload=data["reload"])
                rows.append(row)
                (out / "summary.json").write_text(json.dumps(rows, indent=2))
                print(json.dumps(row), flush=True)
            if profile != "probe":
                group = [r for r in rows if r["scene"] == scene and r["path"] == path]
                equal = all(r["hashes"] == group[0]["hashes"] for r in group)
                print(f"identity {scene} {path}: {equal}", flush=True)
                if not equal: raise RuntimeError(f"cache on/off image identity failed: {scene} {path}")
    return 0


def gpu(label, profile):
    env = dict(os.environ, N8_OWNER="m7-pc1", N8_OUT=str(OUT))
    with SCHEDULE.open("a", encoding="utf-8") as f:
        f.write(f"\n[m7-pc1 request] owner=m7-pc1 label={label} <=15 min headless {profile}; yields to root order (lc2, root b4b, gc1, r1n, tp2). {datetime.datetime.now(datetime.timezone.utc).isoformat()}\n")
    end = time.monotonic()+100*60
    invocation = time.strftime("%H%M%S", time.gmtime())
    index = 0
    while time.monotonic() < end:
        # Do not race the explicitly ordered gates at a release boundary.
        after = order_context()
        order = after.splitlines()[0]
        gates = re.findall(r"root-gate-[A-Za-z0-9-]+", order)
        if any(not re.search(r"^(?:\S+ )?"+re.escape(gate)+r"(?:-\S+)? window released \d{4}-", after, re.MULTILINE) for gate in gates):
            time.sleep(10)
            continue
        if "m7-lc2" in order and not re.search(r"^m7-lc2 \S+ window released \d{4}-", after, re.MULTILINE):
            time.sleep(10)
            continue
        index += 1
        env["PC1_LEASE_ORDER"] = after.splitlines()[0]
        command = [sys.executable, "-B", str(ROOT / "scripts/n8-run.py"), "--label", f"{label}-{invocation}-{index}",
                   "--minutes", "15", "--fast-check", "--wait-seconds", "0", "--",
                   sys.executable, "-B", str(Path(__file__).resolve()), "batch", label, profile]
        result = subprocess.run(command, cwd=ROOT, env=env, creationflags=FLAGS)
        if result.returncode != 3: return result.returncode
        time.sleep(5)
    raise RuntimeError("GPU queue deadline; no owned jobs remain")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["build", "gpu", "batch"])
    parser.add_argument("label")
    parser.add_argument("profile", nargs="?", default="dev")
    args = parser.parse_args()
    if args.command == "build": build(args.label); return 0
    if args.command == "gpu": return gpu(args.label, args.profile)
    return batch(args.label, args.profile)


if __name__ == "__main__":
    raise SystemExit(main())
