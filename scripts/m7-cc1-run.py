"""CC1 compile screens and PC1 startup matrix. Own targets/evidence on D:.

build dev|release; gpu LABEL screen|game-gate|quarry-check|all|dev|release|edit-dev|edit-release|lib
All waits are in this process; each acquired n8 lease is <=15 minutes.
"""
import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = Path("D:/Coding/renderlab-work/cc1")
TARGET = OUT / "target"
SCHEDULE = Path("E:/renderlab-work/gpu-schedule.md")
REVIEWED_ORDER = OUT / "reviewed-order.txt"
FLAGS = subprocess.CREATE_NO_WINDOW


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), ROOT / "scripts" / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


pc1 = load("m7-pc1-run")
pc1.OUT, pc1.TARGET = OUT, TARGET


def append_schedule(message):
    for _ in range(100):
        try:
            with SCHEDULE.open("a", encoding="utf-8") as file:
                file.write(message)
            return
        except PermissionError:
            time.sleep(.2)
    raise RuntimeError("schedule remained locked during append")


def batch(label, profile):
    if pc1.latest_order() != os.environ["PC1_LEASE_ORDER"]:
        return 75
    if profile == "all":
        for item in ("dev", "release", "edit-dev", "edit-release"):
            result = batch(f"{label}-{item}", item)
            if result:
                return result
        return 0
    if profile == "game-gate":
        for item in ("dev", "release"):
            result = batch(f"{label}-{item}", f"game-{item}")
            if result: return result
        return 0
    if profile == "quarry-check":
        for item in ("dev", "release"):
            for name in ("base-final", label):
                pc1.TARGET = OUT / "baseline" if name == "base-final" else TARGET
                result = pc1.batch(f"{name}-{item}", item,
                    ("--frames", "48", "--every", "47", "--timing-frames", "32"),
                    scenes_override=("quarry",), modes_override=("off",), before_process=memory_ready)
                if result: return result
        return 0
    if profile != "screen":
        pc1.TARGET = OUT / "baseline" if label.startswith("base-") else TARGET
        if profile.startswith("edit-"):
            return edits(label, profile.removeprefix("edit-"))
        game_only = profile.startswith("game-")
        if game_only: profile = profile.removeprefix("game-")
        return pc1.batch(label, profile, ("--frames", "48", "--every", "47", "--timing-frames", "32") if profile in ("dev", "release") else (),
            scenes_override=("game",) if game_only else None, before_process=memory_ready)
    binary = OUT / "bin" / f"{label}.exe"
    if not binary.exists():
        binary = max((TARGET / "debug/deps").glob("renderlab-*.exe"), key=lambda p: p.stat().st_mtime)
    run = OUT / "runs" / label
    run.mkdir(parents=True, exist_ok=True)
    (run / "binary.json").write_text(json.dumps({"path":str(binary),
        "sha256":hashlib.file_digest(binary.open("rb"), "sha256").hexdigest(),
        "base_head":subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()}, indent=2))
    for path in ("rt", "nonrt"):
        directory = OUT / "runs" / label / path
        overrides = OUT / "screen-cases.json"
        selected = json.loads(overrides.read_text()).get(label) if overrides.exists() else None
        cases = (selected or os.environ.get("RENDERLAB_CC1_CASES", "full,no-hit-shade,no-gi,no-lights,no-decals,no-software")).split(',')
        expected = len([case for case in cases if case != "no-software" or path == "nonrt"])
        if (directory / "codegen.json").exists() and len(json.loads((directory / "codegen.json").read_text())) == expected:
            continue
        if pc1.latest_order() != os.environ["PC1_LEASE_ORDER"]:
            return 75
        directory.mkdir(parents=True, exist_ok=True)
        env = dict(os.environ, RENDERLAB_CC1_PROFILE="1", RENDERLAB_CC1_OUT=str(directory),
                   RENDERLAB_ES1_PATH=path, RENDERLAB_RELOAD_PROFILE="1",
                   RENDERLAB_CC1_CASES=','.join(cases),
                   VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation")
        env.pop("VK_INSTANCE_LAYERS", None)
        with (directory / "screen.log").open("w") as log:
            result = subprocess.run([str(binary), "gpu_cc1_codegen", "--ignored", "--nocapture", "--test-threads=1"],
                cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, creationflags=FLAGS,
                timeout=180 if "fused-runtime" in cases else 600)
        if result.returncode:
            print((directory / "screen.log").read_text()[-6000:], flush=True)
            return result.returncode
        print((directory / "codegen.json").read_text(), flush=True)
    return 0


def memory_ready(scene, path, mode):
    if scene != "quarry": return True
    available = load("m6-film-run").resource_sample()["available_mbytes"]
    if available >= 7168: return True
    print(f"CC1 yields before quarry {path}/{mode}: {available:.0f} MiB available, need 7168", flush=True)
    return False


def edits(label, profile):
    """PC1's API edit/readback probe on both scenes, paths and build profiles."""
    out = OUT / "runs" / label
    out.mkdir(parents=True, exist_ok=True)
    binary = pc1.TARGET / ("release" if profile == "release" else "debug") / "examples/game_scene.exe"
    manifest = json.loads(Path("E:/renderlab-work/m7-es1/quarry-manifest.json").read_text())
    manifest["limits"] = "D:/Coding/projects/desktop/renderlab/measurements/quarry-tuning/detail/limits-r128-u64-6e417dc13431.json"
    quarry = out / "quarry.json"
    quarry.write_text(json.dumps(manifest))
    for scene in ("game", "quarry"):
        for path in ("rt", "nonrt"):
            directory = out / f"{scene}-{path}"
            if (directory / "run.json").exists():
                continue
            if pc1.latest_order() != os.environ["PC1_LEASE_ORDER"]:
                return 75
            if not memory_ready(scene, path, "edit"):
                return 75
            tint = .6 + secrets.randbelow(3000000)/10000000
            env = dict(os.environ, RENDERLAB_STARTUP_PROFILE="1", RENDERLAB_RELOAD_PROBE_TINT=str(tint),
                VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation")
            env.pop("VK_INSTANCE_LAYERS", None)
            command = [str(binary), "--path", path, "--out", str(directory), "--frames", "2", "--every", "1",
                "--size", "1280x720" if scene == "game" else "1920x1080", "--no-pipeline-cache",
                "--reload-probe", "--no-gi", "--no-temporal"]
            if scene == "quarry":
                command += ["--quarry", str(quarry)]
            with (out / f"{scene}-{path}.log").open("w") as log:
                result = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT,
                    creationflags=FLAGS, timeout=240)
            if result.returncode:
                return result.returncode
            (directory / "edit-tint.json").write_text(json.dumps({"tint":tint}))
            data = json.loads((directory / "run.json").read_text())
            print(json.dumps({"scene":scene,"path":path,"reload":data["reload"]}), flush=True)
    return 0


def gpu(label, profile):
    OUT.mkdir(parents=True, exist_ok=True)
    append_schedule(f"\n[m7-cc1 request] owner=m7-cc1 label={label} <=15 min headless {profile}; yields to latest root order. {datetime.datetime.now(datetime.timezone.utc).isoformat()}\n")
    deadline = time.monotonic() + 100*60
    attempt = 0
    while time.monotonic() < deadline:
        # Poll the lock cheaply in-process. Spawning n8 only when it is free avoids
        # both a shell retry loop and a directory full of occupied-lock deferrals.
        if Path("D:/Coding/renderlab-work/gpu-slot.lock").exists():
            time.sleep(.1)
            continue
        context = pc1.order_context()
        order = context.splitlines()[0]
        hold_file = OUT / "gpu-hold.json"
        if hold_file.exists():
            hold = json.loads(hold_file.read_text())
            tail = pc1.schedule_text()[hold["schedule_offset"]:]
            if not re.search(r"^\[?"+re.escape(hold["marker"])+r"(?:\]|\s|$)", tail, re.MULTILINE):
                time.sleep(5)
                continue
        # A new natural-language directive needs review before another reservation;
        # do not guess its priority from owner names embedded in the sentence.
        if not REVIEWED_ORDER.exists() or REVIEWED_ORDER.read_text().strip() != order:
            time.sleep(5)
            continue
        gates = re.findall(r"root-gate-[A-Za-z0-9-]+", order)
        if any(not re.search(r"^(?:\S+ )?"+re.escape(gate)+r"(?:-\S+)? window released \d{4}-", context, re.MULTILINE) for gate in gates):
            time.sleep(10)
            continue
        attempt += 1
        env = dict(os.environ, N8_OWNER="m7-cc1", N8_OUT=str(OUT), PC1_LEASE_ORDER=order)
        command = [sys.executable, "-B", str(ROOT / "scripts/n8-run.py"), "--label", f"{label}-{os.getpid()}-{attempt}",
            "--minutes", "15", "--fast-check", "--wait-seconds", "0", "--",
            sys.executable, "-B", str(Path(__file__).resolve()), "batch", label, profile]
        result = subprocess.run(command, cwd=ROOT, env=env, creationflags=FLAGS)
        if result.returncode not in (3, 75):
            return result.returncode
        time.sleep(15)
    raise RuntimeError("CC1 GPU queue deadline; no owned jobs remain")


if __name__ == "__main__":
    command, label, *profile = sys.argv[1:]
    if command == "build":
        pc1.build(label)
    else:
        raise SystemExit((gpu if command == "gpu" else batch)(label, profile[0]))
