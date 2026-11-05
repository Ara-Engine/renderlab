"""Run ignored offscreen determinism tests under the shared Windows GPU slot.

py -3 scripts/determinism-run.py --binary E:/.../renderlab.exe --manifest E:/.../frame.json \
    --output E:/renderlab-work/determinism/check --test arithmetic --iterations 3

No compilation or window launch. See docs/determinism.md for manifest fields.
"""
from pathlib import Path
import argparse
import datetime
import hashlib
import json
import os
import subprocess
import threading
import time
import uuid

LOCK = Path("D:/Coding/renderlab-work/gpu-slot.lock")
FLAGS = subprocess.CREATE_NO_WINDOW
PREFIX = "renderer::fresh_shadow_tests::primary_ray_gpu_tests::soft_shadow_probe_tests::"
TESTS = {
    "arithmetic": PREFIX + "determinism_tests::gpu_ray_arithmetic_repeat",
    "static": PREFIX + "determinism_tests::gpu_static_repeat",
    "acceptance": PREFIX + "soft_shadow_acceptance_tests::gpu_soft_shadow_traced_acceptance",
}
GAMES = ("eldenring", "cyberpunk", "helldivers", "cs2.exe", "valorant", "fortniteclient",
         "overwatch", "arcraiders", "bf6.exe", "league of legends", "r5apex", "dota2",
         "rocketleague", "minecraft", "nightreign", "blackops", "cod.exe", "witcher",
         "baldur", "starfield", "roblox", "destiny2", "escapefromtarkov", "factorio")


def utc():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def save(path, value):
    path.write_text(json.dumps(value, indent=2))


def sha(path):
    with path.open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


def powershell(command):
    p = subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-Command", command],
                       capture_output=True, text=True, creationflags=FLAGS, timeout=30, check=True)
    return p.stdout.strip()


def preflight(owned=()):
    processes = json.loads(powershell("Get-CimInstance Win32_Process | Select-Object Name,ProcessId,ExecutablePath | ConvertTo-Json -Compress"))
    conflicts = []
    for p in processes:
        if p["ProcessId"] in owned:
            continue
        name = p["Name"].lower()
        path = (p.get("ExecutablePath") or "").lower().replace("/", "\\")
        launcher = name.startswith(("steam", "riotclient", "epicgameslauncher")) or "\\riot client\\" in path
        game = any(g in name for g in GAMES) or any(g in path for g in (
            "\\steamapps\\common\\", "\\epic games\\", "\\riot games\\", "\\xboxgames\\"))
        renderer = name.startswith(("renderlab", "fresh-tests", "primary-tests", "screen-",
            "sel2shadow", "quarry-resets", "m6-shadows", "virtual-stream", "native.exe",
            "climb-tests", "probe-tests", "prog-tests"))
        renderer |= "\\renderlab-work\\" in path and "\\deps\\" in path and name.endswith(".exe")
        if (game and not launcher) or renderer or "unrealeditor" in name or "ue5editor" in name:
            conflicts.append(p)
    available = float(powershell("(Get-Counter '\\Memory\\Available MBytes').CounterSamples[0].CookedValue"))
    return dict(utc=utc(), conflicts=conflicts, available_mb=available)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--test", choices=TESTS, default="arithmetic")
    parser.add_argument("--iterations", type=int, default=1)
    parser.add_argument("--minutes", type=float, default=5)
    parser.add_argument("--layer-path", type=Path, default=Path(os.environ.get("VULKAN_SDK", "C:/VulkanSDK/1.4.363.0")) / "Bin")
    parser.add_argument("--reference", type=Path)
    parser.add_argument("--keep-images", action="store_true")
    parser.add_argument("--api-dump", action="store_true")
    args = parser.parse_args()
    if not 0 < args.minutes <= 44 or args.iterations < 1:
        parser.error("minutes must be in (0,44], iterations >=1")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    binary = args.binary.resolve()
    manifest = json.loads(args.manifest.read_text())
    results = []
    windows = []
    for iteration in range(args.iterations):
        folder = output / str(iteration)
        folder.mkdir()
        token = uuid.uuid4().hex
        owner = dict(owner="determinism", utc=utc(), launcher_pid=os.getpid(), token=token,
                     expected_minutes=args.minutes, output=str(folder))
        try:
            LOCK.mkdir()
        except FileExistsError:
            save(folder / "deferred.json", dict(reason="GPU slot held", utc=utc(),
                 owner=(LOCK / "owner.json").read_text() if (LOCK / "owner.json").exists() else None))
            print("DEFERRED: GPU slot held", flush=True)
            return 3
        save(LOCK / "owner.json", owner)
        try:
            state = preflight()
            save(folder / "preflight.json", state)
            if state["conflicts"] or state["available_mb"] < 6144:
                save(folder / "deferred.json", state)
                print("DEFERRED: GPU process or available memory preflight", flush=True)
                return 3
            m = dict(manifest, output=str(folder), validate=True)
            save(folder / "manifest.json", m)
            settings = folder / "vk_layer_settings.txt"
            settings.write_text("khronos_validation.validate_sync = true\n" + (
                "lunarg_api_dump.show_shader = true\nlunarg_api_dump.file = true\n"
                f"lunarg_api_dump.log_filename = {folder.as_posix()}/api.txt\n"
                "lunarg_api_dump.flush = false\n" if args.api_dump else ""))
            implicit = folder / "empty-implicit"
            implicit.mkdir()
            env = dict(os.environ, RENDERLAB_PRIMARY_MANIFEST=str(folder / "manifest.json"),
                       RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
                       VK_LAYER_PATH=str(args.layer_path.resolve()), VK_IMPLICIT_LAYER_PATH=str(implicit),
                       VK_LAYER_SETTINGS_PATH=str(settings), VK_INSTANCE_LAYERS="VK_LAYER_KHRONOS_validation" + (
                           ";VK_LAYER_LUNARG_api_dump" if args.api_dump else ""))
            for key in ("VK_LOADER_LAYERS_DISABLE", "VK_LAYER_ENABLES", "VK_LAYER_DISABLES"):
                env.pop(key, None)
            command = [str(binary), TESTS[args.test], "--ignored", "--exact", "--nocapture", "--test-threads=1"]
            started = time.monotonic()
            stop = threading.Event()
            reasons = []
            with (folder / "test.log").open("w", encoding="utf-8") as log:
                child = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, env=env, creationflags=FLAGS)

                def monitor():
                    while not stop.wait(3):
                        try:
                            state = preflight((child.pid,))
                            if state["conflicts"]:
                                reasons.append(state)
                                child.kill()
                                return
                        except Exception as error:
                            reasons.append(dict(monitor_error=str(error)))
                            child.kill()
                            return

                thread = threading.Thread(target=monitor)
                thread.start()
                try:
                    child.wait(timeout=args.minutes * 60)
                except subprocess.TimeoutExpired:
                    reasons.append(dict(timeout=True))
                    child.kill()
                    child.wait()
                finally:
                    stop.set()
                    thread.join()
            captures = {p.name: sha(p) for p in folder.iterdir() if p.suffix in (".rgba16f", ".rgba32u", ".f32", ".u32")}
            mismatches = [name for name, value in captures.items() if name.startswith("pose-") and args.reference and (
                not (args.reference / name).is_file() or sha(args.reference / name) != value)]
            complete = (len([name for name in captures if name.startswith("pose-")]) == 36
                        if args.test != "arithmetic" else (folder / "ray.json").is_file())
            row = dict(iteration=iteration, binary_sha256=sha(binary), command=command,
                       seconds=time.monotonic() - started, exit=child.returncode, stop_reasons=reasons,
                       captures=captures, complete=complete, reference_mismatches=mismatches)
            save(folder / "run.json", row)
            results.append(row)
            save(output / "results.json", results)
            print(iteration, "exit", child.returncode, "reference mismatches", len(mismatches), flush=True)
            if not child.returncode and not args.keep_images:
                for name in captures:
                    path = folder / name
                    assert path.resolve().is_relative_to(output)
                    path.unlink()
            if child.returncode or reasons or mismatches or not complete:
                return 1
        finally:
            assert json.loads((LOCK / "owner.json").read_text())["token"] == token
            (LOCK / "owner.json").unlink()
            LOCK.rmdir()
            windows.append(dict(acquired=owner["utc"], released=utc()))
            save(output / "gpu-windows.json", windows)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
