"""F0's bounded, hidden-process GPU/build lease. Exit 3 means defer and do CPU work.

py -3 -B scripts/m6-film-run.py --label build -- cargo test -p renderlab --lib --release --no-run -j 2 --target-dir D:/Coding/renderlab-work/m6-exact-quality/f0/target
py -3 -B scripts/m6-film-run.py --label thin-policy --manifest <json> -- <test-binary> renderer::fresh_shadow_tests::primary_ray_gpu_tests::film_reference_tests::gpu_film_reference --ignored --exact --nocapture --test-threads=1
"""
import argparse
import ctypes
from ctypes import wintypes
import datetime
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import threading
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
OUT = Path("D:/Coding/renderlab-work/m6-exact-quality/f0")
OWNER = "m6-film-reference-f0"
LOCK = Path("D:/Coding/renderlab-work/gpu-slot.lock")
FLAGS = subprocess.CREATE_NO_WINDOW
GAMES = ("eldenring", "cyberpunk", "helldivers", "cs2.exe", "valorant", "fortniteclient", "overwatch",
         "arcraiders", "bf6.exe", "league of legends", "r5apex", "dota2", "rocketleague", "minecraft",
         "nightreign", "blackops", "cod.exe", "witcher", "baldur", "starfield", "roblox", "destiny2",
         "escapefromtarkov", "factorio")


def resource_sample():
    """Cheap 10 Hz Windows resource context, without launching a subprocess."""
    class Memory(ctypes.Structure):
        _fields_ = [("size", wintypes.DWORD), ("load", wintypes.DWORD)] + [
            (name, ctypes.c_ulonglong) for name in ("total", "available", "page_total", "page_available", "virtual_total", "virtual_available", "extended")]
    class Process(ctypes.Structure):
        _fields_ = [("size", wintypes.DWORD), ("usage", wintypes.DWORD), ("pid", wintypes.DWORD),
            ("heap", ctypes.c_size_t), ("module", wintypes.DWORD), ("threads", wintypes.DWORD),
            ("parent", wintypes.DWORD), ("priority", wintypes.LONG), ("flags", wintypes.DWORD),
            ("name", wintypes.WCHAR*260)]
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    kernel.Process32FirstW.argtypes = [wintypes.HANDLE, ctypes.POINTER(Process)]
    kernel.Process32NextW.argtypes = [wintypes.HANDLE, ctypes.POINTER(Process)]
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    memory = Memory(); memory.size = ctypes.sizeof(memory)
    if not kernel.GlobalMemoryStatusEx(ctypes.byref(memory)): raise ctypes.WinError(ctypes.get_last_error())
    snapshot = kernel.CreateToolhelp32Snapshot(2, 0)
    if snapshot == ctypes.c_void_p(-1).value: raise ctypes.WinError(ctypes.get_last_error())
    entry = Process(); entry.size = ctypes.sizeof(entry); count = 0
    try:
        valid = kernel.Process32FirstW(snapshot, ctypes.byref(entry))
        while valid:
            count += entry.name.lower() == "rustc.exe"
            valid = kernel.Process32NextW(snapshot, ctypes.byref(entry))
    finally: kernel.CloseHandle(snapshot)
    return dict(host_unix_seconds=time.time(), available_mbytes=memory.available/(1024**2), rustc_count=count)


def utc():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def save(path, obj):
    path.write_text(json.dumps(obj, indent=2), encoding="utf-8")


def ps(command):
    p = subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-Command", command],
                       capture_output=True, text=True, creationflags=FLAGS, timeout=25)
    if p.returncode:
        raise RuntimeError(p.stderr)
    return json.loads(p.stdout)


def state(owned=()):
    procs = ps("Get-CimInstance Win32_Process | Select-Object Name,ProcessId,ExecutablePath | ConvertTo-Json -Compress")
    games, gpu, builds = [], [], []
    for p in procs:
        if p["ProcessId"] in owned or p["ProcessId"] == os.getpid():
            continue
        name = p["Name"].lower()
        path = (p.get("ExecutablePath") or "").lower().replace("/", "\\")
        launcher = "\\riot client\\" in path or name.startswith(("riotclient", "steam", "epicgameslauncher"))
        if not launcher and (any(g in name for g in GAMES) or any(s in path for s in ("\\steamapps\\common\\", "\\epic games\\", "\\riot games\\", "\\xboxgames\\"))):
            games.append(p)
        renderer_path = ("\\renderlab-work\\" in path or "\\renderlab\\" in path) and any(s in path for s in ("\\deps\\", "\\builds\\", "\\release\\", "\\debug\\"))
        if "unrealeditor" in name or "ue5editor" in name or name == "unrealgame.exe" or name.startswith(("renderlab", "fresh-tests", "primary-tests", "screen-", "quarry-resets", "m6-shadows")) or (renderer_path and name.endswith(".exe")):
            gpu.append(p)
        if name in ("cargo.exe", "rustc.exe", "clippy-driver.exe", "link.exe", "unrealbuildtool.exe"):
            builds.append(p)
    # This is deliberately the requested performance counter, not FreePhysicalMemory.
    mb = ps("(Get-Counter '\\Memory\\Available MBytes').CounterSamples.CookedValue | ConvertTo-Json -Compress")
    smi = subprocess.run(["nvidia-smi", "--query-gpu=name,driver_version,utilization.gpu,memory.used,temperature.gpu", "--format=csv,noheader,nounits"],
                         capture_output=True, text=True, creationflags=FLAGS, timeout=10)
    return dict(utc=utc(), available_mbytes=mb, games=games, gpu_processes=gpu, builds=builds, gpu=smi.stdout.strip())


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--label", required=True)
    p.add_argument("--manifest", type=Path)
    p.add_argument("--minutes", type=float, default=10)
    p.add_argument("--wait-seconds", type=float, default=0, help="bounded resource queue wait, at most 45 seconds")
    p.add_argument("--runtime-memory-mbytes", type=float, default=6144,
                   help="runtime available-memory floor; 0 permits owned allocation transients (preflight always requires 6144)")
    p.add_argument("--no-build-contention", action="store_true", help="defer/abort performance runs if any compiler or linker is active")
    p.add_argument("--contended-screen", action="store_true", help="authorized screen only: 5 GiB preflight, 3 GiB runtime, marked contended")
    p.add_argument("--record-resources", action="store_true", help="attach 10 Hz memory/rustc context to each timing sample")
    p.add_argument("--fast-check", action="store_true",
                   help="batched fast check (AGENTS.md speed rule): 4 GiB preflight, 2 GiB runtime, contention recorded")
    p.add_argument("command", nargs=argparse.REMAINDER)
    a = p.parse_args()
    preflight_floor = 4096 if a.fast_check else 5120 if a.contended_screen else 6144
    if a.contended_screen or a.fast_check:
        a.runtime_memory_mbytes = 2048 if a.fast_check else 3072
        a.record_resources = True
    if not 0 < a.minutes <= 45:
        p.error("lease must be at most 45 minutes")
    if not 0 <= a.wait_seconds <= 45:
        p.error("resource queue wait must be at most 45 seconds")
    if a.runtime_memory_mbytes < 0:
        p.error("runtime memory floor cannot be negative")
    cmd = a.command[1:] if a.command[:1] == ["--"] else a.command
    if not cmd:
        p.error("command required")
    runs = OUT / "runs"
    runs.mkdir(parents=True, exist_ok=True)
    log = runs / f"{a.label}.log"
    if log.exists():
        p.error(f"refusing to overwrite {log}")
    deadline=time.monotonic()+a.wait_seconds
    while True:
        try:
            LOCK.mkdir()
            break
        except FileExistsError:
            if time.monotonic()>=deadline:
                print("DEFERRED: GPU lock occupied", flush=True)
                save(runs / f"{a.label}-deferred.json", dict(utc=utc(), owner=(LOCK / "owner.json").read_text() if (LOCK / "owner.json").exists() else None))
                return 3
            time.sleep(min(1,max(0,deadline-time.monotonic())))
    owner = dict(owner=OWNER, utc=utc(), expected_minutes=a.minutes, launcher_pid=os.getpid(), token=uuid.uuid4().hex, label=a.label)
    save(LOCK / "owner.json", owner)
    start = time.monotonic()
    try:
        pre = state()
        save(runs / f"{a.label}-preflight.json", pre)
        if pre["games"] or pre["gpu_processes"] or pre["available_mbytes"] < preflight_floor or (a.no_build_contention and pre["builds"]):
            print("DEFERRED: preflight", json.dumps(pre), flush=True)
            return 3
        env = os.environ.copy()
        implicit = OUT / "empty-implicit"
        implicit.mkdir(exist_ok=True)
        env.update(CARGO_BUILD_JOBS="2", OPENBLAS_NUM_THREADS="2", OMP_NUM_THREADS="2", NUMBA_NUM_THREADS="2",
                   RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
                   VK_LAYER_PATH="C:/VulkanSDK/1.4.363.0/Bin", VK_IMPLICIT_LAYER_PATH=str(implicit), VK_INSTANCE_LAYERS="VK_LAYER_KHRONOS_validation")
        env.pop("VK_LOADER_LAYERS_DISABLE", None)
        if a.manifest:
            env["RENDERLAB_PRIMARY_MANIFEST"] = str(a.manifest.resolve())
        samples, resources, stop, failure = [], [], threading.Event(), []
        with log.open("w", encoding="utf-8") as f:
            child = subprocess.Popen(cmd, cwd=ROOT, env=env, stdout=f, stderr=subprocess.STDOUT, creationflags=FLAGS)
            def terminate_owned_tree():
                if child.poll() is None:
                    subprocess.run(["taskkill", "/PID", str(child.pid), "/T", "/F"], stdout=subprocess.DEVNULL,
                                   stderr=subprocess.DEVNULL, creationflags=FLAGS, timeout=10)
            def monitor():
                while not stop.wait(2):
                    try:
                        s = state((child.pid,))
                        samples.append(s)
                        if s["games"] or s["gpu_processes"] or s["available_mbytes"] < a.runtime_memory_mbytes or (a.no_build_contention and s["builds"]):
                            failure.append(f"contention or available memory below {a.runtime_memory_mbytes:g} MiB")
                            terminate_owned_tree()
                            return
                    except Exception as e:
                        failure.append(str(e))
                        terminate_owned_tree()
                        return
            thread = threading.Thread(target=monitor)
            thread.start()
            def resource_monitor():
                while not stop.is_set():
                    try:
                        sample = resource_sample(); resources.append(sample)
                        if sample["available_mbytes"] < a.runtime_memory_mbytes:
                            failure.append("resource sampler: available memory below runtime floor")
                            terminate_owned_tree(); return
                    except Exception as e:
                        failure.append(str(e)); terminate_owned_tree(); return
                    stop.wait(.1)
            resource_thread = threading.Thread(target=resource_monitor) if a.record_resources else None
            if resource_thread: resource_thread.start()
            try:
                child.wait(timeout=max(1, a.minutes * 60 - (time.monotonic() - start) - 90))
            except subprocess.TimeoutExpired:
                failure.append("bounded lease deadline")
                terminate_owned_tree()
                child.wait()
            finally:
                stop.set()
                thread.join()
                if resource_thread: resource_thread.join()
        binary = Path(cmd[0])
        record = dict(command=cmd, start=owner["utc"], end=utc(), seconds=time.monotonic() - start,
                      exit=child.returncode, failures=failure, samples=samples,
                      manifest=json.loads(a.manifest.read_text()) if a.manifest else None,
                      runtime_memory_floor_mbytes=a.runtime_memory_mbytes,
                      reject_build_contention=a.no_build_contention,
                      contended_screen=a.contended_screen, preflight_memory_floor_mbytes=preflight_floor,
                      resource_samples=resources,
                      executable_sha256=hashlib.file_digest(binary.open("rb"), "sha256").hexdigest() if binary.is_file() else None)
        save(runs / f"{a.label}.json", record)
        if a.manifest and resources:
            directory = Path(record["manifest"]["output"])
            for filename in ("paired-timing.json", "fresh-screen.json", "kernel-screen.json"):
                path = directory/filename
                if not path.exists() or path.stat().st_mtime < datetime.datetime.fromisoformat(owner["utc"]).timestamp(): continue
                data = json.loads(path.read_text())
                def annotate(value):
                    if isinstance(value, dict):
                        if "host_unix_seconds" in value and "rustc_count" not in value:
                            sample = min(resources, key=lambda s: abs(s["host_unix_seconds"]-value["host_unix_seconds"]))
                            value["resource_context"] = dict(sample, age_ms=1000*abs(sample["host_unix_seconds"]-value["host_unix_seconds"]))
                            value["contended_screen"] = a.contended_screen
                        for child_value in list(value.values()):
                            if isinstance(child_value, (dict,list)): annotate(child_value)
                    elif isinstance(value, list):
                        for child_value in value: annotate(child_value)
                annotate(data); save(path, data)
        print(a.label, "exit", child.returncode, "seconds", record["seconds"], "failures", failure, flush=True)
        return 0 if child.returncode == 0 and not failure else 1
    finally:
        if json.loads((LOCK / "owner.json").read_text())["token"] != owner["token"]:
            raise RuntimeError("GPU lease ownership changed")
        if LOCK.resolve() != Path("D:/Coding/renderlab-work/gpu-slot.lock").resolve():
            raise RuntimeError("unexpected lock path")
        # Only this fixed, token-checked directory is removed. Another process reading owner.json
        # holds a Windows share lock on it briefly; retry rather than leave a stale lease.
        for attempt in range(60):
            try:
                shutil.rmtree(LOCK)
                break
            except PermissionError:
                if attempt == 59:
                    raise
                time.sleep(0.5)
        ledger = OUT / "gpu-windows.json"
        rows = json.loads(ledger.read_text()) if ledger.exists() else []
        rows.append(dict(label=a.label, acquired=owner["utc"], released=utc(), seconds=time.monotonic() - start))
        save(ledger, rows)


if __name__ == "__main__":
    raise SystemExit(main())
