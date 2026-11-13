"""F0's bounded, hidden-process GPU/build lease. Exit 3 means defer and do CPU work.

py -3 -B scripts/m6-film-run.py --label build -- cargo test -p renderlab --lib --release --no-run -j 2 --target-dir D:/Coding/renderlab-work/m6-exact-quality/f0/target
py -3 -B scripts/m6-film-run.py --label thin-policy --manifest <json> -- <test-binary> renderer::fresh_shadow_tests::primary_ray_gpu_tests::film_reference_tests::gpu_film_reference --ignored --exact --nocapture --test-threads=1
"""
import argparse
import atexit
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
PREFLIGHT_MEMORY_MBYTES = None
RUNTIME_MEMORY_MBYTES = None
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


# The environment may only tighten these. The one sanctioned raise is --quarry-lease
# (coordinator, 2026-11-10): child cap 12 GiB, 14 GiB to start. Machine floor 5 GiB, non-timing
# leases start at 8,000 MB (coordinator, 2026-11-13: the PC is handed over; up to three builds
# run beside leases, the newest child stops first).
WATCH_CHILD_MBYTES = min(8192.0, float(os.environ.get("RENDERLAB_WATCH_CHILD_MB", 8192)))
WATCH_FREE_MBYTES = max(5120.0, float(os.environ.get("RENDERLAB_WATCH_FREE_MB", 5120)))
WORK = Path("D:/Coding/renderlab-work")
# A timing lease holds every build slot: no build starts while TIMING_HOLD exists, and the lease
# waits (at most 10 minutes) for the running builds to finish before it takes the GPU lock.
BUILD_SLOTS = [WORK / "build-slot.lock", WORK / "build-slot-2.lock", WORK / "build-slot-3.lock"]
TIMING_HOLD = WORK / "timing-hold"
QUARRY_CHILD_MBYTES = 12288.0
QUARRY_PREFLIGHT_MBYTES = 14336.0


def gpu_clocks():
    """The GPU's clocks, power, temperature, utilisation and P-state, one nvidia-smi line. The
    coordinator (2026-11-11): the RTX sat in a slow state for a whole lease (22.1 ms against 10.44
    at the same commit later) with no contention check firing; every lease records this at its
    start and end, and a timing lease also opens with a known reference run."""
    try:
        return subprocess.run(["nvidia-smi", "--query-gpu=clocks.gr,clocks.mem,power.draw,temperature.gpu,utilization.gpu,pstate",
                               "--format=csv,noheader"], capture_output=True, text=True, timeout=10,
                              creationflags=FLAGS).stdout.strip()
    except Exception as e:
        return f"nvidia-smi failed: {e}"


def memory_watch_sample(root_pid):
    """Machine available MiB and the largest working set (MiB) in root_pid's process tree.

    The always-on watchdog (coordinator rule, 2026-11-10): an admission bar is not a memory
    guard, so every leased child tree is sampled once a second, apart from --runtime-memory-mbytes.
    """
    class Memory(ctypes.Structure):
        _fields_ = [("size", wintypes.DWORD), ("load", wintypes.DWORD)] + [
            (name, ctypes.c_ulonglong) for name in ("total", "available", "page_total", "page_available", "virtual_total", "virtual_available", "extended")]
    class Process(ctypes.Structure):
        _fields_ = [("size", wintypes.DWORD), ("usage", wintypes.DWORD), ("pid", wintypes.DWORD),
            ("heap", ctypes.c_size_t), ("module", wintypes.DWORD), ("threads", wintypes.DWORD),
            ("parent", wintypes.DWORD), ("priority", wintypes.LONG), ("flags", wintypes.DWORD),
            ("name", wintypes.WCHAR*260)]
    class Counters(ctypes.Structure):
        _fields_ = [("cb", wintypes.DWORD), ("faults", wintypes.DWORD)] + [
            (name, ctypes.c_size_t) for name in ("peak_ws", "ws", "qpp", "qp", "qnpp", "qnp", "pagefile", "peak_pagefile")]
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    psapi = ctypes.WinDLL("psapi", use_last_error=True)
    kernel.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    kernel.Process32FirstW.argtypes = [wintypes.HANDLE, ctypes.POINTER(Process)]
    kernel.Process32NextW.argtypes = [wintypes.HANDLE, ctypes.POINTER(Process)]
    kernel.OpenProcess.restype = wintypes.HANDLE
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    psapi.GetProcessMemoryInfo.argtypes = [wintypes.HANDLE, ctypes.POINTER(Counters), wintypes.DWORD]
    memory = Memory(); memory.size = ctypes.sizeof(memory)
    kernel.GlobalMemoryStatusEx(ctypes.byref(memory))
    parents = {}
    snapshot = kernel.CreateToolhelp32Snapshot(2, 0)
    entry = Process(); entry.size = ctypes.sizeof(entry)
    try:
        valid = kernel.Process32FirstW(snapshot, ctypes.byref(entry))
        while valid:
            parents[entry.pid] = entry.parent
            valid = kernel.Process32NextW(snapshot, ctypes.byref(entry))
    finally:
        kernel.CloseHandle(snapshot)
    tree, grew = {root_pid}, True
    while grew:
        more = {pid for pid, parent in parents.items() if parent in tree} - tree
        grew = bool(more); tree |= more
    largest, largest_pid = 0.0, None
    for pid in tree:
        handle = kernel.OpenProcess(0x1000 | 0x0010, False, pid)  # QUERY_LIMITED_INFORMATION | VM_READ
        if not handle:
            continue
        try:
            counters = Counters(); counters.cb = ctypes.sizeof(counters)
            if psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb):
                ws = max(counters.ws, counters.pagefile) / 2**20
                if ws > largest:
                    largest, largest_pid = ws, pid
        finally:
            kernel.CloseHandle(handle)
    return memory.available / 2**20, largest, largest_pid


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
        # Ara's .NET script compile server lives under renderlab-work's Ara trees but never
        # touches the GPU; counting it as a renderer stopped every lease (2026-11-12 15:52).
        compile_server = name.startswith("ara.compileserver")
        renderer_path = not compile_server and ("\\renderlab-work\\" in path or "\\renderlab\\" in path) and any(s in path for s in ("\\deps\\", "\\builds\\", "\\release\\", "\\debug\\"))
        if "unrealeditor" in name or "ue5editor" in name or name == "unrealgame.exe" or name.startswith(("renderlab", "fresh-tests", "primary-tests", "screen-", "quarry-resets", "m6-shadows")) or (renderer_path and name.endswith(".exe")):
            gpu.append(p)
        if name in ("cargo.exe", "rustc.exe", "clippy-driver.exe", "link.exe", "unrealbuildtool.exe"):
            builds.append(p)
    # This is deliberately the requested performance counter, not FreePhysicalMemory.
    mb = ps("(Get-Counter '\\Memory\\Available MBytes').CounterSamples.CookedValue | ConvertTo-Json -Compress")
    smi = subprocess.run(["nvidia-smi", "--query-gpu=name,driver_version,utilization.gpu,memory.used,temperature.gpu", "--format=csv,noheader,nounits"],
                         capture_output=True, text=True, creationflags=FLAGS, timeout=10)
    return dict(utc=utc(), available_mbytes=mb, games=games, gpu_processes=gpu, builds=builds, gpu=smi.stdout.strip())


def timing_lease_running():
    """Another lease holds the GPU, or another tool marks a build slot with a timing lease's label."""
    if LOCK.exists():
        return True
    for slot in BUILD_SLOTS:
        try:
            if "timing" in json.loads((slot / "owner.json").read_text()).get("label", ""):
                return True
        except Exception:
            pass
    return False


def take_timing_hold(label):
    """Take TIMING_HOLD (atomic mkdir), then wait at most 10 minutes for the running builds' slots
    to clear. Released at exit. False (the caller defers) if another timing lease holds it or the GPU
    is busy: a waiting timing lease must not hold every build slot while another lease runs."""
    if timing_lease_running():
        return False
    try:
        TIMING_HOLD.mkdir()
    except FileExistsError:
        return False
    (TIMING_HOLD / "owner.json").write_text(json.dumps(dict(owner=OWNER, label=label, utc=utc(), pid=os.getpid())))
    def release():
        shutil.rmtree(TIMING_HOLD, ignore_errors=True)
    atexit.register(release)
    deadline = time.monotonic() + 600
    while any(s.exists() for s in BUILD_SLOTS) and time.monotonic() < deadline:
        time.sleep(2)
    held = [s.name for s in BUILD_SLOTS if s.exists()]
    if held:
        print("timing lease: build slots still held after 10 minutes:", held, flush=True)
    return True


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--label", required=True)
    p.add_argument("--manifest", type=Path)
    p.add_argument("--minutes", type=float, default=10)
    p.add_argument("--wait-seconds", type=float, default=0, help="bounded resource queue wait, at most 45 seconds")
    p.add_argument("--runtime-memory-mbytes", type=float, default=5120,
                   help="runtime available-memory floor; 0 permits owned allocation transients (preflight requires 8000)")
    p.add_argument("--no-build-contention", action="store_true", help="defer/abort performance runs if any compiler or linker is active")
    p.add_argument("--contended-screen", action="store_true", help="authorized screen only: 5 GiB preflight, 3 GiB runtime, marked contended")
    p.add_argument("--record-resources", action="store_true", help="attach 10 Hz memory/rustc context to each timing sample")
    p.add_argument("--fast-check", action="store_true", help="correctness batch: 4 GiB preflight, 2 GiB runtime abort")
    p.add_argument("--quarry-lease", action="store_true",
                   help="full-quarry lease: watchdog child cap 12 GiB (machine floor stays 8 GiB), starts only at 14 GiB available")
    p.add_argument("--timing", action="store_true",
                   help="timing lease: holds all three build slots (waits up to 10 minutes for running builds), at most 5 minutes, no build contention")
    p.add_argument("command", nargs=argparse.REMAINDER)
    a = p.parse_args()
    preflight_floor = PREFLIGHT_MEMORY_MBYTES if PREFLIGHT_MEMORY_MBYTES is not None else (5120 if a.contended_screen else 8000)
    if a.timing:
        if a.minutes > 5:
            p.error("a timing lease is at most 5 minutes")
        a.no_build_contention = True
        if not take_timing_hold(a.label):
            print("DEFERRED: the GPU or another timing lease is busy; a timing lease holds the build slots only once it can run", flush=True)
            return 3
    elif TIMING_HOLD.exists():
        # A timing lease that took the hold is draining builds and owns the next GPU turn (root, 2026-11-13:
        # an ordinary lease took the lock inside a waiting timing hold and the timing lease lost its turn).
        print("DEFERRED: a timing lease holds timing-hold and runs next", flush=True)
        return 3
    if a.contended_screen:
        a.runtime_memory_mbytes = 3072
        a.record_resources = True
    if a.fast_check:
        preflight_floor = 4096
        a.runtime_memory_mbytes = 2048
        a.record_resources = True
    if RUNTIME_MEMORY_MBYTES is not None:
        a.runtime_memory_mbytes = RUNTIME_MEMORY_MBYTES
    global WATCH_CHILD_MBYTES
    global WATCH_FREE_MBYTES
    if a.quarry_lease:
        WATCH_CHILD_MBYTES = QUARRY_CHILD_MBYTES
        preflight_floor = max(preflight_floor, QUARRY_PREFLIGHT_MBYTES)
        # An owner-granted quarry window (coordinator relays his word): RENDERLAB_QUARRY_WINDOW_MB
        # = "START,FLOOR" lowers the start bar and the machine floor for this one lease. The only
        # way below the defaults, never below 12 GiB to start or 4 GiB free; the child cap stays.
        window = os.environ.get("RENDERLAB_QUARRY_WINDOW_MB")
        if window:
            start, floor = (float(x) for x in window.split(","))
            preflight_floor = max(12288.0, start)
            WATCH_FREE_MBYTES = max(4096.0, floor)
            print(f"quarry window: start bar {preflight_floor:g} MiB, machine floor {WATCH_FREE_MBYTES:g} MiB", flush=True)
    print(f"memory watchdog: child cap {WATCH_CHILD_MBYTES:g} MiB, machine floor {WATCH_FREE_MBYTES:g} MiB"
          f"{' (quarry lease)' if a.quarry_lease else ''}", flush=True)
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
    owner = dict(owner=OWNER, utc=utc(), expected_minutes=a.minutes, launcher_pid=os.getpid(), token=uuid.uuid4().hex, label=a.label,
                 watch_child_mbytes=WATCH_CHILD_MBYTES, watch_free_mbytes=WATCH_FREE_MBYTES, quarry_lease=a.quarry_lease)
    save(LOCK / "owner.json", owner)
    start = time.monotonic()
    try:
        pre = state()
        save(runs / f"{a.label}-preflight.json", pre)
        # Headless GPU work may run while Islam plays (AGENTS.md, 2026-11-12): a game is contention to
        # record, not a reason to stop. The lease opens no windows and sends no input.
        if pre["games"]:
            print("contended: a game is running:", [g["Name"] for g in pre["games"]], flush=True)
            # Game-time bars (coordinator, 2026-11-13): the machine floor is 6 GiB, not 8, while a game
            # runs, the bars PC integration's leases ran on beside VALORANT. Not for quarry leases.
            if not a.quarry_lease:
                WATCH_FREE_MBYTES = min(WATCH_FREE_MBYTES, 6144.0)
                print(f"game-time bars: machine floor {WATCH_FREE_MBYTES:g} MiB", flush=True)
        if pre["gpu_processes"] or pre["available_mbytes"] < preflight_floor or (a.no_build_contention and pre["builds"]):
            print("DEFERRED: preflight", json.dumps(pre), flush=True)
            return 3
        env = os.environ.copy()
        implicit = OUT / "empty-implicit"
        implicit.mkdir(exist_ok=True)
        env.update(CARGO_BUILD_JOBS="2", OPENBLAS_NUM_THREADS="2", OMP_NUM_THREADS="2", NUMBA_NUM_THREADS="2",
                   RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
                   VK_IMPLICIT_LAYER_PATH=str(implicit))
        # The Khronos validation layer is for correctness batches only (--fast-check, or
        # RENDERLAB_LEASE_VALIDATION=1). Timing leases run without it, as a shipping build does:
        # P-PIPE (2026-11-10) found it forced on every leased child since F0.
        if a.fast_check or os.environ.get("RENDERLAB_LEASE_VALIDATION") == "1":
            env.update(VK_LAYER_PATH="C:/VulkanSDK/1.4.363.0/Bin", VK_INSTANCE_LAYERS="VK_LAYER_KHRONOS_validation")
        else:
            env.pop("VK_INSTANCE_LAYERS", None)
        env.pop("VK_LOADER_LAYERS_DISABLE", None)
        if a.manifest:
            env["RENDERLAB_PRIMARY_MANIFEST"] = str(a.manifest.resolve())
        samples, resources, stop, failure = [], [], threading.Event(), []
        gpu_state = {"start": gpu_clocks()}
        print("gpu state at start:", gpu_state["start"], flush=True)
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
                        if s["gpu_processes"] or s["available_mbytes"] < a.runtime_memory_mbytes or (a.no_build_contention and s["builds"]):
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
            def memory_watchdog():
                while not stop.is_set():
                    try:
                        free, largest, pid = memory_watch_sample(child.pid)
                        if largest > WATCH_CHILD_MBYTES or free < WATCH_FREE_MBYTES:
                            failure.append(f"memory watchdog: process {pid} at {largest:.0f} MiB (limit {WATCH_CHILD_MBYTES:g}), "
                                           f"machine available {free:.0f} MiB (floor {WATCH_FREE_MBYTES:g})")
                            print("MEMORY WATCHDOG:", failure[-1], flush=True)
                            terminate_owned_tree(); return
                    except Exception as e:
                        failure.append(f"memory watchdog error: {e}")
                    stop.wait(1)
            watchdog_thread = threading.Thread(target=memory_watchdog)
            watchdog_thread.start()
            try:
                child.wait(timeout=max(1, a.minutes * 60 - (time.monotonic() - start) - 90))
            except subprocess.TimeoutExpired:
                failure.append("bounded lease deadline")
                terminate_owned_tree()
                child.wait()
            finally:
                stop.set()
                thread.join()
                watchdog_thread.join()
                if resource_thread: resource_thread.join()
        gpu_state["end"] = gpu_clocks()
        print("gpu state at end:", gpu_state["end"], flush=True)
        binary = Path(cmd[0])
        record = dict(command=cmd, start=owner["utc"], end=utc(), seconds=time.monotonic() - start, gpu_state=gpu_state,
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
        # Only this fixed, token-checked directory is removed.
        shutil.rmtree(LOCK)
        ledger = OUT / "gpu-windows.json"
        rows = json.loads(ledger.read_text()) if ledger.exists() else []
        rows.append(dict(label=a.label, acquired=owner["utc"], released=utc(), seconds=time.monotonic() - start))
        save(ledger, rows)


if __name__ == "__main__":
    raise SystemExit(main())
