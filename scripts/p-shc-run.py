"""P-SHC: bounded n8 leases and paired PC1 compile/startup/image screens.

Review the latest anchored root order, then pass its SHA-256 to `gpu`.
All outputs and the two-job Cargo target belong to D:/Coding/renderlab-work/p-shc.
"""
import argparse
import datetime
import hashlib
import importlib.util
import json
import os
import re
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parents[1]
OUT = Path("D:/Coding/renderlab-work/p-shc")
SCHEDULE = Path("E:/renderlab-work/gpu-schedule.md")
FLAGS = subprocess.CREATE_NO_WINDOW


def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


lease = load(REPO / "tools/ara1/lease.py", "shc_order")


def build(baseline):
    """Immutable main snapshot plus the current measurement example; two Cargo jobs."""
    import io
    import tarfile
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "bin").mkdir(exist_ok=True)
    sha = subprocess.check_output(["git", "rev-parse", baseline], cwd=REPO, text=True).strip()
    source = OUT / ("baseline-src-" + sha[:12])
    if not source.exists():
        source.mkdir()
        archive = subprocess.check_output(["git", "archive", sha], cwd=REPO)
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar: tar.extractall(source, filter="data")
    shutil.copy2(REPO / "examples/game_scene.rs", source / "examples/game_scene.rs")
    (OUT / "baseline-source.json").write_text(json.dumps(dict(git_head=sha,
        harness_sha256=hashlib.sha256((REPO / "examples/game_scene.rs").read_bytes()).hexdigest()), indent=2))
    n8 = load(REPO / "scripts/n8-run.py", "shc_build_resources")
    guard = load(REPO / "scripts/m7-mp1-run.py", "shc_build_owner").guard()
    guard.OWNER = "p-shc"
    quiet = load(REPO / "scripts/m7-g1-run.py", "shc_build_quiet").quiet_active
    env = dict(os.environ, CARGO_TARGET_DIR=str(OUT / "target"), CARGO_BUILD_JOBS="2")
    rows = []
    for tag, cwd, command in [
        ("baseline", source, ["cargo", "build", "-p", "renderlab", "--release", "--example", "game_scene", "--example", "ara1_capture", "-j", "2"]),
        ("final", REPO, ["cargo", "build", "-p", "renderlab", "--release", "--example", "game_scene", "--example", "ara1_capture", "-j", "2"]),
        ("tests", REPO, ["cargo", "test", "-p", "renderlab", "--lib", "--no-run", "-j", "2"]),
    ]:
        # Cargo can reuse same-name path-package fingerprints across snapshots when
        # archived mtimes are older. Keep the baseline's compilation target isolated.
        target = OUT / ("target-before" if tag == "baseline" else "target")
        env["CARGO_TARGET_DIR"] = str(target)
        while quiet(guard, any_lease=False) or n8.s1.runner.resource_sample()["available_mbytes"] < 8192: time.sleep(1)
        start = time.perf_counter()
        with (OUT / ("build-" + tag + "-final.log")).open("w") as log:
            child = subprocess.Popen(command, cwd=cwd, env=env, stdout=log, stderr=subprocess.STDOUT, creationflags=FLAGS)
            while child.poll() is None:
                time.sleep(1)
                if quiet(guard, any_lease=False) or n8.s1.runner.resource_sample()["available_mbytes"] < 6144:
                    subprocess.run(["taskkill", "/PID", str(child.pid), "/T", "/F"], capture_output=True, creationflags=FLAGS)
                    child.wait()
                    raise RuntimeError("build yielded to compiler-quiet window or 6 GiB RAM floor")
        rows.append(dict(tag=tag, command=command, seconds=time.perf_counter()-start, exit=child.returncode))
        (OUT / "build-final.json").write_text(json.dumps(rows, indent=2))
        if child.returncode: raise RuntimeError("build failed: " + tag)
        if tag == "tests":
            test = max((target / "debug/deps").glob("renderlab-*.exe"), key=lambda p:p.stat().st_mtime)
            shutil.copy2(test, OUT / "bin/tests-final.exe")
        else:
            shutil.copy2(target / "release/examples/game_scene.exe", OUT / ("bin/" + tag + ".exe"))
            shutil.copy2(target / "release/examples/ara1_capture.exe", OUT / ("bin/ara1-" + tag + ".exe"))
        print("built", tag, "seconds", rows[-1]["seconds"], flush=True)


def batch(label, cases):
    if cases == ["@startup"]:
        for tag in ("baseline", "final"):
            os.environ["P_SHC_BINARY"] = str(OUT / ("bin/" + tag + ".exe"))
            code = batch(label + "-" + tag, ["rt:cold", "rt:warm", "nonrt:cold", "nonrt:warm"])
            if code: return code
        return 0
    out = OUT / "runs" / label
    out.mkdir(parents=True, exist_ok=True)
    binary = Path(os.environ.get("P_SHC_BINARY", str(OUT / "bin/final.exe")))
    binary_hash = hashlib.file_digest(binary.open("rb"), "sha256").hexdigest()
    if (out / "binary.json").exists() and json.loads((out / "binary.json").read_text())["sha256"] != binary_hash:
        raise RuntimeError("cannot resume with another executable")
    (out / "binary.json").write_text(json.dumps(dict(sha256=binary_hash,
        head=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip())))
    for case in cases:
        lease.check_order(os.environ["P_SHC_ORDER"])
        path, hint = case.split(":")
        render_path = path.removeprefix("quarry-")
        quality = hint.startswith("quality-")
        mode = hint.removeprefix("quality-")
        if hint not in ("baseline", "quality-baseline", "reload-gate", "lib", "devloop", "cold", "warm"):
            raise ValueError("experimental modes require the archived experiment binary and runner")
        startup = hint in ("cold", "warm")
        directory = out / (path + "-" + hint)
        if (directory / "run.json").exists(): continue
        if hint == "devloop":
            scene = json.loads(Path("D:/Coding/renderlab-work/ara1/phase2-scenes/game-base-quality.json").read_text())
            scene["frames"] = scene["frames"][:2]
            for frame in scene["frames"]: frame["capture"] = True
            spec = out / (path + "-devloop-scene.json")
            spec.write_text(json.dumps(scene))
            adapter = OUT / "bin/ara1-final.exe"
            recipe = dict(engine="renderlab",source=str(REPO),kind="startup to first GPU readback",
                scope="ARA1 game-base scene, first two frames; renderer startup, no Rust edit or scanout",
                commands=[[str(adapter),str(spec),path,str(directory)]],ready=str(directory / "first-image.ready"),
                out=str(out / (path + "-devloop-result")),environment={"WGPU_DEBUG":"0","WGPU_VALIDATION":"0"})
            recipe_path = out / (path + "-devloop-recipe.json")
            recipe_path.write_text(json.dumps(recipe,indent=2))
            env = dict(os.environ, ARA1_EXPECTED_ORDER=os.environ["P_SHC_ORDER"], ARA1_LEASE_OWNER="p-shc",
                VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation")
            env.pop("VK_INSTANCE_LAYERS",None)
            env.pop("RENDERLAB_STARTUP_PROFILE_DRIVER",None)
            with (out / (path + "-devloop.log")).open("w") as log:
                result=subprocess.run([sys.executable,"-B",str(REPO / "tools/ara1/devloop.py"),str(recipe_path)],
                    cwd=REPO,env=env,stdout=log,stderr=subprocess.STDOUT,creationflags=FLAGS,timeout=180)
            if result.returncode:return result.returncode
            print(path + " ARA1 devloop passed",flush=True)
            continue
        if hint in ("reload-gate", "lib"):
            test = OUT / "bin/tests-final.exe"
            env = dict(os.environ, RENDERLAB_ES1_PATH=path)
            with (out / (path + "-" + hint + ".log")).open("w") as log:
                arguments = ["gpu_shc_mixed_reload", "--ignored", "--nocapture", "--test-threads=1"] if hint == "reload-gate" else ["--test-threads=2"]
                result = subprocess.run([str(test), *arguments],
                    cwd=REPO, env=env, stdout=log, stderr=subprocess.STDOUT, creationflags=FLAGS, timeout=120)
            if result.returncode: return result.returncode
            print(path + " " + hint + " passed", flush=True)
            continue
        tint = .6 + secrets.randbelow(3000000) / 10000000
        env = dict(os.environ, RENDERLAB_STARTUP_PROFILE="1", RENDERLAB_STARTUP_PROFILE_DRIVER="1",
            RENDERLAB_RELOAD_PROFILE="1", RENDERLAB_RELOAD_PROBE_TINT=str(tint),
            WGPU_DEBUG="0", WGPU_VALIDATION="0", VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation")
        if os.environ.get("P_SHC_NATIVE", "0") != "1": env.pop("RENDERLAB_STARTUP_PROFILE_DRIVER", None)
        env.pop("VK_INSTANCE_LAYERS", None)
        env.pop("RENDERLAB_SHC_HINT", None)
        env.pop("RENDERLAB_SHC_DYNAMIC_GI", None)
        env.pop("RENDERLAB_SHC_PROGRAM_TINT", None)
        cache = out / (path + "-cache")
        if startup:
            nonce_file = out / (path + "-cache-tint.json")
            if hint == "cold":
                if cache.exists(): raise RuntimeError("cold cache already exists")
                nonce_file.write_text(json.dumps(dict(tint=tint)))
            else: tint = json.loads(nonce_file.read_text())["tint"]
            env["RENDERLAB_SHC_PROGRAM_TINT"] = str(tint)
        command = [str(binary), "--path", render_path, "--out", str(directory),
            "--frames", "48" if quality else "2", "--every", "47" if quality else "1",
            "--size", "1920x1080" if path.startswith("quarry-") else "1280x720",
            "--timing-frames", "32"]
        command += ["--pipeline-cache", str(cache)] if startup else ["--no-pipeline-cache"]
        if quality: env["RENDERLAB_SHC_HDR_HASHES"] = "1"
        elif not startup: command += ["--reload-probe", "--no-gi", "--no-temporal"]
        if path.startswith("quarry-"): command += ["--quarry", str(OUT / "quarry.json")]
        start = time.perf_counter()
        with (out / (path + "-" + hint + ".log")).open("w") as log:
            result = subprocess.run(command, cwd=REPO, env=env, stdout=log, stderr=subprocess.STDOUT,
                creationflags=FLAGS, timeout=240)
        if result.returncode: return result.returncode
        data = json.loads((directory / "run.json").read_text())
        row = dict(case=case, tint=tint, seconds=time.perf_counter()-start,
            startup=data["startup_to_first_image_seconds"], reload=data["reload"], timing=data["timing"])
        (directory / "case.json").write_text(json.dumps(row, indent=2))
        print(json.dumps(row), flush=True)
    return 0


def gpu(label, expected, cases):
    lease.check_order(expected)
    with SCHEDULE.open("a", encoding="utf-8") as f:
        f.write(f"\n[p-shc request] owner=p-shc label={label} <=15 min headless compile screen; 6 GiB RAM floor, pipeline/composition ownership only. {datetime.datetime.now(datetime.timezone.utc).isoformat()}\n")
    deadline = time.monotonic() + 45*60
    after_owner = os.environ.get("P_SHC_AFTER_OWNER")
    def released():
        return not after_owner or any(re.fullmatch(re.escape(after_owner)+r" .+? window released \d{4}-.+", line)
            for line in lease.order_state()["following"])
    os.environ.update(N8_OWNER="p-shc", N8_OUT=str(OUT), P_SHC_ORDER=expected)
    n8 = load(REPO / "scripts/n8-run.py", "shc_n8")
    n8.s1.runner.PREFLIGHT_MEMORY_MBYTES = float(os.environ.get("P_SHC_ADMISSION_MB", "6144"))
    n8.s1.runner.RUNTIME_MEMORY_MBYTES = 6144
    original_save = n8.s1.runner.save
    def save(path, value):
        if path == n8.s1.runner.LOCK / "owner.json":
            try: lease.check_order(expected)
            except RuntimeError:
                n8.s1.runner.LOCK.rmdir()
                raise
            value["root_order"] = lease.order_state()["line"]
            if not released():
                n8.s1.runner.LOCK.rmdir()
                raise RuntimeError("owner handoff is not released")
            if os.environ.get("P_SHC_QUIET") == "1":
                with SCHEDULE.open("a", encoding="utf-8") as f:
                    f.write(f"\n[p-shc QUIET active] owner=p-shc label={value['label']} QUIET active; delegated coordinator granted final <=15 min window; no foreign builds until owner release.\n")
        original_save(path, value)
    n8.s1.runner.save = save
    quiet = os.environ.get("P_SHC_QUIET") == "1"
    if quiet:
        original_state = n8.s1.runner.state
        first = True
        def state(owned=()):
            nonlocal first
            value = original_state(owned)
            if first:
                first = False
                deadline = time.monotonic() + 20
                while value["builds"] and not value["games"] and not value["gpu_processes"] and time.monotonic() < deadline:
                    time.sleep(1)
                    value = original_state(owned)
            return value
        n8.s1.runner.state = state
    attempt = 0
    while time.monotonic() < deadline:
        lease.check_order(expected)
        if not released():
            time.sleep(1)
            continue
        while Path("D:/Coding/renderlab-work/gpu-slot.lock").exists():
            lease.check_order(expected)
            if time.monotonic() >= deadline: raise RuntimeError("bounded GPU queue expired")
            time.sleep(.05)
        lease.check_order(expected)
        attempt += 1
        tag = f"{label}-{attempt}"
        sys.argv = ["n8-run.py", "--label", tag, "--minutes", "15", "--record-resources", "--",
            sys.executable, "-B", str(Path(__file__).resolve()), "batch", label, *cases]
        if quiet: sys.argv.insert(6, "--no-build-contention")
        try: code = n8.s1.runner.main()
        finally:
            if n8.s1.claimed:
                with SCHEDULE.open("a", encoding="utf-8") as f:
                    f.write(f"\np-shc {tag} window released {n8.s1.runner.utc()} (no owned GPU/build job remains).\n")
                n8.s1.claimed = False
        if code != 3: return code
        time.sleep(1)
    raise RuntimeError("bounded GPU queue expired")


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("command", choices=["order", "build", "gpu", "batch"])
    p.add_argument("label", nargs="?")
    p.add_argument("cases", nargs="*")
    p.add_argument("--expected-order")
    p.add_argument("--baseline-ref", default="dc93b7e")
    a = p.parse_args()
    if a.command == "order": print(json.dumps(lease.order_state(), indent=2))
    elif a.command == "build": build(a.baseline_ref)
    elif a.command == "batch": raise SystemExit(batch(a.label, a.cases))
    else: raise SystemExit(gpu(a.label, a.expected_order, a.cases))
