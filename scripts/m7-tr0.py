"""TR0 measurement tooling; never edits renderer code or the U0 package.

plan creates immutable input timelines; run blocks on its own bounded, hidden
offscreen GPU lease; summarize and clean are CPU-only. Missing reference frames
and post-TSR direct/visibility estimands are explicit gaps. Restored T1b ages
are reused only after the original replay and black-primary controls pass.
"""
import argparse
import csv
import copy
import contextlib
import ctypes
import datetime as dt
import hashlib
import importlib.util
import io
import json
import math
import os
from pathlib import Path
import re
import shutil
import subprocess
import threading
import time
import uuid

import numpy as np

REPO = Path(__file__).resolve().parents[1]
ROOT = Path("E:/renderlab-work/m7-tr0")
T1 = Path("E:/renderlab-work/t1-gpu")
SCRIPT_BYTES = Path(__file__).read_bytes()
SCRIPT_SHA = hashlib.sha256(SCRIPT_BYTES).hexdigest()


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), REPO / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


U0 = load("m7-u0-ue")
U0.OUT, U0.OWNER = ROOT, "m7-tr0"
ORIGINAL_PACKAGE_DIR = U0.PACKAGE_DIR
_engine_tool = U0.engine_tool
_preflight = U0.preflight
# Queue outside the lease. A foreign process must not consume a short lease
# in U0's normal 180-second preflight wait and allow a late child launch.
U0.preflight = lambda path, wait_seconds=0: _preflight(path, wait_seconds=0)
NATIVE = load("bench-motion-score")
_state = U0.E2E.state
_powershell = U0.E2E.powershell
_available_gib = U0.E2E.available_gib
_compiler_checked_at = 0.
_monitor_context = threading.local()


class MonitorCommands:
    """Keep optional GPU telemetry out of the live process-conflict poll."""
    def __getattr__(self, name):
        return getattr(subprocess, name)

    def run(self, command, *args, **kwargs):
        if getattr(_monitor_context, "active", False) and Path(str(command[0])).stem.lower() == "nvidia-smi":
            return subprocess.CompletedProcess(command, 0, "not sampled by live conflict guard", "")
        return subprocess.run(command, *args, **kwargs)


U0.E2E.subprocess = MonitorCommands()


def process_inventory():
    """Read the guard's same four process fields without a WMI worker."""
    from ctypes import wintypes as w

    class Entry(ctypes.Structure):
        _fields_ = [("size", w.DWORD), ("usage", w.DWORD), ("pid", w.DWORD),
                    ("heap", ctypes.c_size_t), ("module", w.DWORD), ("threads", w.DWORD),
                    ("parent", w.DWORD), ("priority", w.LONG), ("flags", w.DWORD),
                    ("name", w.WCHAR * 260)]

    api = ctypes.WinDLL("kernel32", use_last_error=True)
    api.CreateToolhelp32Snapshot.argtypes = [w.DWORD, w.DWORD]
    api.CreateToolhelp32Snapshot.restype = w.HANDLE
    for name in ("Process32FirstW", "Process32NextW"):
        function = getattr(api, name)
        function.argtypes, function.restype = [w.HANDLE, ctypes.POINTER(Entry)], w.BOOL
    api.OpenProcess.argtypes, api.OpenProcess.restype = [w.DWORD, w.BOOL, w.DWORD], w.HANDLE
    api.QueryFullProcessImageNameW.argtypes = [w.HANDLE, w.DWORD, w.LPWSTR, ctypes.POINTER(w.DWORD)]
    api.QueryFullProcessImageNameW.restype = w.BOOL
    api.GetExitCodeProcess.argtypes, api.GetExitCodeProcess.restype = [w.HANDLE, ctypes.POINTER(w.DWORD)], w.BOOL
    api.CloseHandle.argtypes, api.CloseHandle.restype = [w.HANDLE], w.BOOL
    snapshot = api.CreateToolhelp32Snapshot(2, 0)  # TH32CS_SNAPPROCESS
    if snapshot == ctypes.c_void_p(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    rows, entry = [], Entry()
    entry.size = ctypes.sizeof(entry)
    try:
        if not api.Process32FirstW(snapshot, ctypes.byref(entry)):
            raise ctypes.WinError(ctypes.get_last_error())
        while True:
            path = None
            process = api.OpenProcess(0x1000, False, entry.pid)  # QUERY_LIMITED_INFORMATION
            if process:
                try:
                    buffer, length = ctypes.create_unicode_buffer(32768), w.DWORD(32768)
                    if api.QueryFullProcessImageNameW(process, 0, buffer, ctypes.byref(length)):
                        path = buffer.value
                    else:
                        error = ctypes.get_last_error()
                        if error != 5:
                            exit_code = w.DWORD()
                            if not api.GetExitCodeProcess(process, ctypes.byref(exit_code)) or exit_code.value == 259:
                                raise ctypes.WinError(error)
                finally:
                    api.CloseHandle(process)
            elif ctypes.get_last_error() not in (5, 87):
                raise ctypes.WinError(ctypes.get_last_error())
            # Protected/exited processes retain their snapshot name/IDs and a
            # null path, as Win32_Process does for inaccessible image paths.
            rows.append(dict(Name=entry.name, ProcessId=entry.pid, ParentProcessId=entry.parent, ExecutablePath=path))
            if not api.Process32NextW(snapshot, ctypes.byref(entry)):
                if ctypes.get_last_error() != 18:  # ERROR_NO_MORE_FILES
                    raise ctypes.WinError(ctypes.get_last_error())
                break
    finally:
        api.CloseHandle(snapshot)
    return rows


def monitor_powershell(command):
    # Memory must not depend on a WMI query completing while unrelated Rust
    # builds saturate the CPU.
    if command.startswith("Get-CimInstance Win32_OperatingSystem"):
        class Memory(ctypes.Structure):
            _fields_ = [("length", ctypes.c_ulong), ("load", ctypes.c_ulong)] + [
                (name, ctypes.c_ulonglong) for name in
                ("total_physical", "available_physical", "total_page", "available_page",
                 "total_virtual", "available_virtual", "extended_virtual")]
        value = Memory()
        value.length = ctypes.sizeof(value)
        if not ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(value)):
            raise ctypes.WinError()
        return json.dumps(dict(FreePhysicalMemory=value.available_physical // 1024,
                               TotalVisibleMemorySize=value.total_physical // 1024))
    if command == "Get-CimInstance Win32_Process | Select-Object Name,ProcessId,ParentProcessId,ExecutablePath | ConvertTo-Json -Compress":
        return json.dumps(process_inventory())
    return _powershell(command)


def monitored_state():
    previous = getattr(_monitor_context, "active", False)
    # This script calls preflight synchronously on its main thread. The
    # inherited helper calls state from its watch_conflicts worker thread.
    _monitor_context.active = threading.current_thread() is not threading.main_thread()
    try:
        result = _state()
        result["monitor_memory_source"] = "GlobalMemoryStatusEx; launch/live floor uses PDH available memory"
        result["monitor_process_source"] = "Toolhelp process snapshot plus QueryFullProcessImageNameW; same names, IDs, parent IDs and paths"
        result["monitor_gpu_telemetry"] = "preflight only; live process/game checks remain mandatory"
        return result
    except Exception as error:
        # The inherited monitor used to lose its thread on a query timeout.
        # A synthetic conflict makes its existing guard stop only our child.
        sentinel = dict(Name="tr0-monitor-error", ProcessId=-1, ParentProcessId=-1,
                        ExecutablePath="", monitor_error=repr(error))
        return dict(utc=U0.utc().isoformat(), games=[], foreign_renderers=[sentinel], builds=[],
                    free_gib=0, available_gib=0, gpu="unavailable", monitor_error=repr(error))
    finally:
        _monitor_context.active = previous


def monitored_available_gib():
    try:
        value = _available_gib()
        return value if math.isfinite(value) and value >= 0 else 0.
    except Exception:
        return 0.  # The inherited low-memory guard stops only our child.


U0.E2E.powershell = monitor_powershell
U0.E2E.state = monitored_state
U0.E2E.available_gib = monitored_available_gib


def activity_policy():
    lines = U0.SCHEDULE.read_text(encoding="utf-8", errors="replace").splitlines()
    controls = [line for line in lines if re.match(r"^\[GAME HOLD(?: RELAXED| RELEASED)?\]", line)]
    last = controls[-1] if controls else ""
    state = "relaxed" if last.startswith("[GAME HOLD RELAXED]") else "hold" if last.startswith("[GAME HOLD]") else "released"
    return dict(state=state, directive=last, speed_claim_eligible=False)


def engine_tool(p):
    name = p["Name"].lower()
    return _engine_tool(p) or name == "build-script-build.exe" or (name == "cs2.exe" and activity_policy()["state"] == "relaxed")


U0.engine_tool = engine_tool


def headless_policy(minutes):
    policy = activity_policy()
    if policy["state"] == "hold":
        print("deferred before lease: GAME HOLD", flush=True)
        raise SystemExit(3)
    if policy["state"] == "relaxed" and minutes > 3:
        raise ValueError("game-play work is limited to short <=3-minute headless leases")
    U0.START_GIB = 6 if policy["state"] == "relaxed" else 4
    return policy


def compiler_safety_valve():
    """After a resource failure, wait outside the lock for 60 Cargo-free seconds."""
    global _compiler_checked_at
    path = ROOT / "resource-pause.json"
    if not path.exists():
        return True
    if time.monotonic() - _compiler_checked_at < 5:
        return False
    _compiler_checked_at = time.monotonic()
    value = json.loads(path.read_text())
    try:
        result = subprocess.run(["tasklist", "/FI", "IMAGENAME eq cargo.exe", "/FO", "CSV", "/NH"],
                                capture_output=True, text=True, creationflags=U0.FLAGS, timeout=15)
        busy = result.returncode != 0 or '"cargo.exe"' in result.stdout.lower()
    except subprocess.TimeoutExpired:
        busy = True
    since = None if busy else value.get("cargo_free_since") or time.time()
    value["cargo_free_since"] = since
    save(path, value)
    if since is not None and time.time() - since >= 60:
        save(ROOT / "resource-pause-released.json", dict(value, released_utc=U0.utc().isoformat()))
        path.unlink()
        return True
    return False


@contextlib.contextmanager
def queued_lease(label, minutes, wait_seconds=0, *, correctness_only=False):
    """Retry atomic acquisition after preparation; never release a foreign lease."""
    deadline = time.monotonic() + wait_seconds
    announced = False
    while True:
        headless_policy(minutes)
        resources_ready = correctness_only or compiler_safety_valve()
        if (not (ROOT / "gpu-defer.txt").exists() and not U0.LOCK.exists()
                and U0.E2E.available_gib() >= U0.START_GIB and resources_ready):
            stack = contextlib.ExitStack()
            try:
                owner = stack.enter_context(U0.lease(label, minutes))
            except SystemExit as error:
                stack.close()
                if error.code != 3:
                    raise
            else:
                with stack:
                    yield owner
                return
        if not wait_seconds or time.monotonic() >= deadline:
            raise SystemExit("deferred: atomic GPU/memory acquisition expired; no owned lease")
        if not announced:
            print("Waiting outside lease for shared GPU/memory", flush=True)
            announced = True
        time.sleep(.25)


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n", encoding="utf-8")


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def pose_commands(p):
    x, y, z = (-100 * p["eye"][2], 100 * p["eye"][0], 100 * p["eye"][1])
    d = np.array(p["target"]) - p["eye"]
    dx, dy, dz = -d[2], d[0], d[1]
    pitch = math.degrees(math.atan2(dz, math.hypot(dx, dy)))
    yaw = math.degrees(math.atan2(dy, dx))
    lines = [f'ke CameraActor K2_SetActorLocation "(X={x:.9f},Y={y:.9f},Z={z:.9f})" false () false',
             f'ke CameraActor K2_SetActorRotation "(Pitch={pitch:.9f},Yaw={yaw:.9f},Roll=0)" false']
    if "to_light" in p:
        d = -np.array(p["to_light"])
        pitch = math.degrees(math.atan2(d[1], math.hypot(d[0], d[2])))
        yaw = math.degrees(math.atan2(d[0], -d[2]))
        lines.append(f'ke DirectionalLight K2_SetActorRotation "(Pitch={pitch:.9f},Yaw={yaw:.9f},Roll=0)" false')
    return lines


def plan(_):
    ROOT.mkdir(parents=True, exist_ok=True)
    (ROOT / "inputs").mkdir(exist_ok=True)
    registered = json.loads((T1 / "sequences/registered.json").read_text())
    inputs = {}
    for name in ("q-walk", "q-disocclusion", "q-zoom"):
        source = T1 / "sequences" / (name + ".json")
        actual = digest(source)
        if actual != registered[name]["sha256"]:
            raise ValueError("T1b registered sequence changed: " + name)
        data = json.loads(source.read_text())
        destination = ROOT / "inputs" / source.name
        if destination.exists() and digest(destination) != actual:
            raise ValueError("refusing to overwrite a different registration")
        shutil.copyfile(source, destination)
        inputs[name] = dict(path=str(destination), sha256=actual, frames=len(data["frames"]),
                            t1b_scored=name == "q-walk")
    lighting = json.loads(Path(json.loads((T1 / "walk-b15.json").read_text())["lighting"]).read_text())
    # Preserve the quarry's actual directional vector when constructing light-event tracks.
    inputs["lighting"] = dict(path=json.loads((T1 / "walk-b15.json").read_text())["lighting"],
                              contents=lighting)
    base = json.loads((T1 / "cameras.json").read_text())["frames"][0]
    light = -np.array(lighting["lights"][0]["travel"])
    light /= np.linalg.norm(light)
    for name, count in (("q-sun-slow", 128), ("q-sun-fast", 64), ("q-sun-jump", 33)):
        frames = []
        for f in range(count):
            degrees = .25 * f / 60 if name == "q-sun-slow" else 10 * min(f, 31) / 60 if name == "q-sun-fast" else 5
            a = math.radians(degrees)
            p = copy.deepcopy(base)
            p.update(frame=f, phase="hold" if name == "q-sun-fast" and f >= 32 else "sun",
                     to_light=[light[0] * math.cos(a) + light[2] * math.sin(a), light[1],
                               light[2] * math.cos(a) - light[0] * math.sin(a)])
            frames.append(p)
        destination = ROOT / "inputs" / (name + ".json")
        save(destination, dict(frames=frames, registration="TR0 from T1 section 7; T1c was never run", hz=60, shutter=0))
        inputs[name] = dict(path=str(destination), sha256=digest(destination), frames=count, t1b_scored=False)
    glossy_path = Path("E:/renderlab-work/m7-u0/scenes/S2G/scene.json")
    glossy = json.loads(glossy_path.read_text())
    for track in glossy["tracks"]:
        name = "s2g-" + track["name"]
        frames = [dict(p, up=[0, 1, 0], fov_y_radians=math.radians(glossy["fov_y_degrees"]),
                       near=.005, far=100, width=1920, height=1080) for p in track["frames"]]
        destination = ROOT / "inputs" / (name + ".json")
        save(destination, dict(frames=frames, scene="S2G", source_sha256=digest(glossy_path),
                               exposure_stops=glossy["exposure_stops"], sun_radius=glossy["sun"]["angular_radius"]))
        inputs[name] = dict(path=str(destination), sha256=digest(destination), frames=len(frames), t1b_scored=False)
    save(ROOT / "inputs/manifest.json", dict(base_commit="708e87c", inputs=inputs,
         reference_gaps=["Original T1b frozen compact references deleted; U0b physical references are a separate estimand",
                         "Restored T1b age masks require history-check and the black-primary compatibility control",
                         "T1b never ran Q-disocclusion, Q-zoom or Q-sun"],
         package=str(U0.PACKAGE), package_exe_sha256=digest(U0.PACKAGE)))
    print(json.dumps({k: {a: b for a, b in v.items() if a != "contents"} for k, v in inputs.items()}, indent=2))


def macro(out, name, lines):
    path = out / "macros" / (name + ".txt")
    path.parent.mkdir(exist_ok=True)
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return "exec " + Path(os.path.relpath(path, U0.PACKAGE.parent.parent)).as_posix()


def settings(quality, path):
    commands = [c for c in U0.BASE_COMMANDS + U0.CONFIGS["ue-tsr"]["commands"]
                if not c.startswith(("r.TSR.History.ScreenPercentage", "sg.AntiAliasingQuality"))]
    commands += [f"sg.AntiAliasingQuality {quality}", "r.ScreenPercentage 100", "r.DynamicRes.OperationMode 0",
                 "ShowFlag.MotionBlur 0", "r.MotionBlurQuality 0", "r.EyeAdaptation.PreExposureOverride 0",
                 "r.DumpGPU.Upload 0", "r.DumpGPU.Explore 0", "r.DumpGPU.Stream 0",
                 "r.DumpGPU.Texture 2", "r.DumpGPU.Buffer 2", "r.DumpGPU.PassParameters 1"]
    if path == "hrt":
        commands += U0.HRT_SHADOWS
    return commands


def colour_input(dump, events, recipe):
    candidates = []
    for event in events:
        if not event["EventName"].startswith("Tonemap "):
            continue
        for name in event["InputResources"]:
            resource = dump.resources[name.split(".")[0]]["Name"]
            matches = (resource == "SceneColor" or resource.endswith(".SceneColor") or resource == "SceneColorResolve") if recipe.get("aa", "tsr") == "none" else resource in ("TSR.Output", "TSR.History.Color")
            if matches:
                candidates.append((event, name))
    if len(candidates) != 1:
        raise ValueError("expected one native Tonemap colour input, found " + str(len(candidates)))
    return candidates[0]


def readback(out, quality, path):
    rows = U0.cvar_rows(out / "cvars.csv")
    expected = {"sg.AntiAliasingQuality": str(quality), "r.AntiAliasingMethod": "4", "r.ScreenPercentage": "100",
                "r.EyeAdaptation.PreExposureOverride": "0", "r.RayTracing.Shadows": str(int(path == "hrt")),
                "r.RayTracing": str(int(path != "vsm-nort")),
                "r.DynamicRes.OperationMode": "0", "r.MotionBlurQuality": "0"}
    groups = U0.scalability_expectations()
    expected.update(groups.get(f"AntiAliasingQuality@{quality}", {}))
    checks = [dict(name=k, expected=v, actual=rows.get(k), ok=k in rows and U0.same(rows[k]["value"], v))
              for k, v in expected.items()]
    log = (out / "ue.log").read_text(encoding="utf-8-sig", errors="replace")
    cameras = [line for line in log.splitlines() if "CameraCachePrivate =" in line]
    recipe = json.loads((out / "recipe.json").read_text())
    if recipe.get("capture_render_thread_sync"):
        expected_lag = "0"
        actual_lag = rows.get("r.OneFrameThreadLag")
        checks.append(dict(name="r.OneFrameThreadLag", expected=expected_lag, actual=actual_lag,
                           ok=actual_lag is not None and U0.same(actual_lag["value"], expected_lag)))
    expected_offset = recipe.get("exposure_offset", 0)
    actual_offset = rows.get("r.ExposureOffset")
    checks.append(dict(name="r.ExposureOffset", expected=str(expected_offset), actual=actual_offset,
                       ok=actual_offset is not None and U0.same(actual_offset["value"], str(expected_offset))))
    for check in checks:
        if check["name"] == "r.AntiAliasingMethod":
            check["expected"] = "0" if recipe.get("aa", "tsr") == "none" else "4"
            check["ok"] = check["actual"] is not None and U0.same(check["actual"]["value"], check["expected"])
    camera_checks = []
    if recipe["capture"]:
        if len(cameras) != len(recipe["frames"]):
            raise ValueError("missing per-frame camera readback")
        for p, line in zip(recipe["frames"], cameras, strict=True):
            m = re.search(r"Location=\(X=([^,]+),Y=([^,]+),Z=([^\)]+)\),Rotation=\(Pitch=([^,]+),Yaw=([^,]+),Roll=([^\)]+)\),FOV=([^,]+)", line)
            if m is None:
                raise ValueError("camera readback syntax")
            x, y, z, pitch, yaw, roll, fov = map(float, m.groups())
            eye = np.array([y, z, -x]) / 100
            a, b = np.radians([pitch, yaw])
            forward = np.array([math.cos(a) * math.sin(b), math.sin(a), -math.cos(a) * math.cos(b)])
            desired = np.array(p["target"]) - p["eye"]
            desired /= np.linalg.norm(desired)
            fov_expected = math.degrees(2 * math.atan(math.tan(p["fov_y_radians"] / 2) * 1920 / 1080))
            good = np.max(np.abs(eye - p["eye"])) < 1e-6 and np.max(np.abs(forward - desired)) < 1e-6 and abs(fov - fov_expected) < 1e-5
            camera_checks.append(dict(frame=p["frame"], ok=bool(good), eye=eye.tolist(), forward=forward.tolist(), horizontal_fov=fov))
    light_checks = []
    light_lines = [line for line in log.splitlines() if "DirectionalLightComponent" in line and ".RelativeRotation =" in line]
    light_poses = [p for p in recipe["frames"] if "to_light" in p] if recipe["capture"] else []
    if light_poses and len(light_lines) == len(light_poses):
        for p, line in zip(light_poses, light_lines, strict=True):
            m = re.search(r"Pitch=([^,]+),Yaw=([^,]+),Roll=([^\)]+)", line)
            if m is None:
                raise ValueError("light readback syntax")
            a, b, _ = np.radians(list(map(float, m.groups())))
            actual = -np.array([math.cos(a) * math.sin(b), math.sin(a), -math.cos(a) * math.cos(b)])
            desired = np.array(p["to_light"])
            desired /= np.linalg.norm(desired)
            light_checks.append(dict(frame=p["frame"], ok=bool(np.max(np.abs(actual - desired)) < 1e-6), to_light=actual.tolist()))
    source_angles = [float(m.group(1)) for m in re.finditer(r"\.LightSourceAngle = ([0-9.eE+-]+)", log)]
    angle_expected = recipe.get("sun_diameter_degrees")
    angle_check = dict(expected=angle_expected, actual=source_angles,
                       ok=angle_expected is not None and bool(source_angles) and all(abs(v - angle_expected) < 1e-5 for v in source_angles))
    sky_intensities = [float(m.group(1)) for m in re.finditer(r"SkyLightComponent.*\.Intensity = ([0-9.eE+-]+)", log)]
    sun_only = recipe.get("program", "full") == "sun"
    result = dict(ok=all(c["ok"] for c in checks + camera_checks + light_checks), checks=checks, camera_checks=camera_checks,
                  light_checks=light_checks, light_readback_complete=not light_poses or len(light_checks) == len(light_poses),
                  sun_angle=angle_check,
                  sun_only_sky_disabled=not sun_only or bool(sky_intensities) and all(v == 0 for v in sky_intensities),
                  camera_readbacks=cameras,
                  d3d12_sm6="PCD3D_SM6" in log and "LogD3D12RHI" in log,
                  commandline_offscreen="-RenderOffscreen" in log)
    save(out / "readback.json", result)
    if not result["ok"] or not result["d3d12_sm6"] or not result["light_readback_complete"]:
        raise ValueError("runtime readback failed: " + str(out))
    if angle_expected is not None and not angle_check["ok"]:
        raise ValueError("runtime sun-diameter readback failed: " + str(out))
    if not result["sun_only_sky_disabled"]:
        raise ValueError("sun-only capture lacks zero SkyLight intensity readback")


def run(args):
    policy = headless_policy(args.minutes)
    if args.wait_seconds:
        deadline = time.monotonic() + args.wait_seconds
        print("Waiting for shared GPU/memory guard (bounded)", flush=True)
        while U0.E2E.available_gib() < U0.START_GIB:
            if time.monotonic() >= deadline:
                print("deferred: GPU/memory wait expired; no lease acquired", flush=True)
                raise SystemExit(3)
            time.sleep(2)
    source = json.loads((ROOT / "inputs" / (args.track + ".json")).read_text())
    glossy = source.get("scene") == "S2G"
    U0.PACKAGE_DIR = args.package_dir or (Path("E:/renderlab-work/m7-u0b/ue-package") if glossy else ORIGINAL_PACKAGE_DIR)
    U0.PACKAGE = U0.PACKAGE_DIR / "Windows/Engine/Binaries/Win64/UnrealGame.exe"
    if not U0.PACKAGE.exists():
        raise SystemExit("package not ready: " + str(U0.PACKAGE))
    frames = source["frames"]
    static_frame = getattr(args, "static_frame", None)
    exposure_offset = getattr(args, "exposure_offset", 0.)
    if exposure_offset and (static_frame is None or not args.capture):
        raise ValueError("exposure offsets are static capture diagnostics only")
    if static_frame is not None:
        frames = [frames[static_frame]]
    if args.smoke:
        frames = frames[:9]
    out = ROOT / "runs" / args.label
    out.mkdir(parents=True, exist_ok=False)
    (out / "capture-source.py").write_bytes(SCRIPT_BYTES)
    settle = getattr(args, "settle_frames", 0)
    temporal_readback = getattr(args, "temporal_readback", False)
    start = 96 + settle
    events = [(40, f"DumpCVars -csv={(out / 'cvars.csv').as_posix()}")]
    # Each repeat starts with a reset, so repeats exercise the same cold input sequence.
    # Installed UE's FCommandLine is 16 KiB. Two full-track A/B pairs fit;
    # four independently guarded invocations provide eight pairs without truncation.
    repeats = 1 if args.capture else 4
    for repeat in range(repeats):
        offset = start + repeat * (len(frames) + 16)
        if not args.capture:
            events.append((offset - 12, f"r.AntiAliasingMethod {0 if repeat % 2 == 0 else 4}"))
        if not args.capture or settle:
            events.append((offset - 1 - settle, "r.ResetViewState"))
        if args.capture:
            # Tuple sorting used to execute the arm macro before a separate
            # same-tick reset. Reset releases RHI history buffers; keep the
            # required cold reset before arming readback, on the same frame.
            arm = (["r.ResetViewState"] if not settle else [])
            initial_root = "PostProcessing*" if temporal_readback else getattr(args, "dump_root", "Tonemap*")
            events.append((offset - 1, macro(out, "arm", arm + [f"r.DumpGPU.Root {initial_root}", f"r.DumpGPU.FrameCount {len(frames)}",
                          f"r.DumpGPU.Directory {(out / 'dump').as_posix()}", "DumpGPU"])))
        for i, p in enumerate(frames):
            lines = (["getall PlayerCameraManager CameraCachePrivate"] +
                     (["getall DirectionalLightComponent RelativeRotation"] if "to_light" in frames[i - 1] else [])
                     if args.capture and i > 0 else []) + pose_commands(p)
            if temporal_readback and i == 2:
                lines.insert(0, f"r.DumpGPU.Root {getattr(args, 'dump_root', 'Tonemap*')}")
            if args.capture:
                # Ordinary Shot is the native main view. HighResShot can replace/reset temporal state.
                if i in {0, 1, 2, 4, 8, 16, 32, 48, 60, 64, 65, 66, 68, 72, 80, 88, 92, 95, 96, 108, 120, 128, 160}:
                    lines.append(f"Shot filename={(out / f'f{i:03d}.png').as_posix()}")
            events.append((offset + i, macro(out, f"r{repeat}-f{i:03d}", lines)))
        if args.capture:
            events.append((offset + len(frames), macro(out, "last-readback", ["getall PlayerCameraManager CameraCachePrivate"] +
                          (["getall DirectionalLightComponent RelativeRotation"] if "to_light" in frames[-1] else []))))
    count = start + repeats * (len(frames) + 16) + (96 if args.capture else 0)
    config = settings(args.quality, args.path)
    synchronous = getattr(args, "synchronous_render", False)
    temporal_readback = getattr(args, "temporal_readback", False)
    if temporal_readback and (not args.capture or not synchronous):
        raise ValueError("temporal readback requires a synchronized quality capture")
    if synchronous:
        if not args.capture:
            raise ValueError("render-thread synchronization is capture-only; cost runs retain defaults")
        config.append("r.OneFrameThreadLag 0")
    config.append(f"r.ExposureOffset {exposure_offset:g}")
    if getattr(args, "aa", "tsr") == "none":
        config.append("r.AntiAliasingMethod 0")
    sun_diameter = math.degrees(2 * source["sun_radius"]) if glossy else math.degrees(.08)
    config.append(f"set DirectionalLightComponent LightSourceAngle {sun_diameter:.9f}")
    config.append("getall DirectionalLightComponent LightSourceAngle")
    if args.program == "sun":
        config += ["set SkyLightComponent Intensity 0", "ShowFlag.SkyLighting 0", "getall SkyLightComponent Intensity"]
    expected_fov = math.degrees(2 * math.atan(math.tan(frames[0]["fov_y_radians"] / 2) * 1920 / 1080))
    initial = copy.deepcopy(frames[0])
    if args.track == "q-sun-jump":
        lighting = json.loads((ROOT / "inputs/manifest.json").read_text())["inputs"]["lighting"]["contents"]
        light = -np.array(lighting["lights"][0]["travel"])
        initial["to_light"] = (light / np.linalg.norm(light)).tolist()
    config += pose_commands(initial) + [f"set CameraComponent FieldOfView {expected_fov:.9f}"]
    startup_macro = macro(out, "startup", config + [f"csvprofile frames={count}"])
    # Exec resolves beneath Engine/Binaries and rejects absolute paths. A tiny,
    # exclusively created scratch name also keeps the CSV timeline below 16 KiB.
    while True:
        short_dir = U0.PACKAGE.parent.parent / ("t" + uuid.uuid4().hex[:2])
        try:
            short_dir.mkdir()
            break
        except FileExistsError:
            continue
    replacements = {}
    for index, macro_source in enumerate(sorted((out / "macros").glob("*.txt"))):
        destination = short_dir / f"{index:x}"
        shutil.copyfile(macro_source, destination)
        replacements["exec " + Path(os.path.relpath(macro_source, U0.PACKAGE.parent.parent)).as_posix()] = "exec " + short_dir.name + "/" + destination.name
    events = [(f, replacements.get(cmd, cmd)) for f, cmd in events]
    startup_macro = replacements[startup_macro]
    ini = [c.replace("r.RayTracing=True", "r.RayTracing=False") if args.path == "vsm-nort" else c for c in U0.COMMON_INI]
    flags = ["-corelimit=2" if policy["state"] == "relaxed" and f == "-corelimit=8" else f for f in U0.UE_FLAGS]
    command = [U0.PACKAGE, "../../../M5Calibration/M5Calibration.uproject", *flags,
               f"-abslog={out / 'ue.log'}", *ini, "-dx12", "-sm6", "-RenderOffscreen",
               "/Game/M7U0b/S2G/Bench" if glossy else "/Game/M6Quarry/View0", "-game", "-ResX=1920", "-ResY=1080", "-ForceRes", "-NoVSync",
               "-UseFixedTimeStep", "-FPS=60", "-csvGpuStats", "-csvCompression=0", "-ExitAfterCsvProfiling",
               "-csvExecCmds=" + ",".join(f"{f}:{c}" for f, c in sorted(events)),
               "-ExecCmds=" + startup_macro]
    # UE's own CmdLineFile expansion avoids Windows' 32K process-commandline limit.
    command_file = out / "commandline.txt"
    def ue_argument(value):
        value = str(value)
        if '"' in value:
            raise ValueError("nested commandline quotes belong in an exec macro")
        if " " in value:
            key, sep, tail = value.partition("=")
            return key + sep + '"' + tail + '"' if sep else '"' + value + '"'
        return value
    command_file.write_text(" ".join(ue_argument(c) for c in command[2:]), encoding="utf-8")
    if len(command_file.read_text()) > 16000:
        for path in short_dir.iterdir():
            path.unlink()
        short_dir.rmdir()
        raise ValueError("UE commandline exceeds its 16 KiB expansion limit")
    actual_command = [str(c) for c in command[:2]] + ["-CmdLineFile=" + command_file.as_posix()]
    save(out / "recipe.json", dict(track=args.track, path=args.path, quality=args.quality, capture=args.capture,
          smoke=args.smoke, start=start, frames=frames, repeats=repeats, gap=16, csv_frames=count,
          sequence_sha256=digest(ROOT / "inputs" / (args.track + ".json")), script_sha256=SCRIPT_SHA,
          package_exe_sha256=digest(U0.PACKAGE), exposure_stops=source.get("exposure_stops", -14) + exposure_offset,
          registered_exposure_stops=source.get("exposure_stops", -14), exposure_offset=exposure_offset,
          diagnostic_only=bool(exposure_offset),
          capture_render_thread_sync=synchronous,
          temporal_readback=temporal_readback,
          sun_diameter_degrees=sun_diameter,
          scene="S2G" if glossy else "quarry", shutter=0, program=args.program,
          aa=getattr(args, "aa", "tsr"), static_source_frame=static_frame, settle_frames=settle,
          note="Timing: 2 no-AA/TSR paired full-track replays in one process; four invocations give eight pairs. Even repeats no AA, odd repeats TSR. Cold reset per replay. Capture and timing are separate processes. No reference score implied."))
    csv_root = U0.PACKAGE_DIR / "Windows/M5Calibration/Saved/Profiling/CSV"
    before = set(csv_root.glob("*.csv"))
    try:
        policy = headless_policy(args.minutes)
        save(out / "activity-policy.json", policy)
        with queued_lease(args.label, args.minutes, args.wait_seconds):
            save(out / "package-identity.json", dict(executable_sha256=digest(U0.PACKAGE),
                 paks={str(p.relative_to(U0.PACKAGE_DIR)): digest(p) for p in U0.PACKAGE_DIR.rglob("*.pak")},
                 utocs={str(p.relative_to(U0.PACKAGE_DIR)): digest(p) for p in U0.PACKAGE_DIR.rglob("*.utoc")}))
            default_timeout = min(args.minutes * 60 - 20, 120) if policy["state"] == "relaxed" else args.minutes * 60 - 20
            U0.guarded(actual_command, out, timeout=getattr(args, "timeout_seconds", default_timeout), cwd=U0.PACKAGE.parent)
    finally:
        # Only the exclusively-created directory and its own copied macros.
        if short_dir.parent.resolve() != U0.PACKAGE.parent.parent.resolve():
            raise ValueError("short macro cleanup outside package Binaries")
        for path in short_dir.iterdir():
            path.unlink()
        short_dir.rmdir()
    new = set(csv_root.glob("*.csv")) - before
    if len(new) != 1:
        raise ValueError("expected exactly one CSV: " + str(new))
    source, = new
    shutil.copyfile(source, out / "profile.csv")
    source.unlink()
    readback(out, args.quality, args.path)
    print("captured", out, flush=True)


def summarize(args):
    out = ROOT / "runs" / args.label
    recipe = json.loads((out / "recipe.json").read_text())
    policy_path = out / "activity-policy.json"
    policy = json.loads(policy_path.read_text()) if policy_path.exists() else {"state": "pre-game-screen"}
    rows = list(csv.reader((out / "profile.csv").open(encoding="utf-8-sig")))
    header = rows[0]
    data = rows[1:]
    n = len(recipe["frames"])
    indices = [recipe["start"] + r * (n + recipe["gap"]) + i
               for r in range(recipe["repeats"]) for i in range(8, n)]
    stats = {}
    for key in ("GPUTime", "GPU/TemporalSuperResolution", "FrameTime", "RenderThreadTime", "GameThreadTime"):
        if key not in header:
            continue
        values = [float(data[i][header.index(key)]) for i in indices if i < len(data) and data[i][header.index(key)]]
        if values:
            stats[key] = dict(n=len(values), p50=float(np.median(values)), p95=float(np.quantile(values, .95)),
                              minimum=min(values), maximum=max(values))
    save(out / "timing.json", dict(stats=stats, capture=recipe["capture"],
         valid_timing=not recipe["capture"] and not recipe["smoke"],
         scope="Native CSV GPUTime critical-path frame and TSR GPU stat; first 8 frames of every cold repeat excluded"))
    if not recipe["capture"]:
        by_method = {}
        for method, parity in (("no_aa", 0), ("tsr", 1)):
            chosen = [recipe["start"] + r * (n + recipe["gap"]) + i
                      for r in range(parity, recipe["repeats"], 2) for i in range(8, n)]
            by_method[method] = {}
            for key in ("GPUTime", "GPU/TemporalSuperResolution"):
                if key not in header:
                    continue
                values = [float(data[i][header.index(key)]) for i in chosen if data[i][header.index(key)]]
                by_method[method][key] = dict(n=len(values), p50=float(np.median(values)), p95=float(np.quantile(values, .95)))
        pairs = []
        frame_costs = []
        col = header.index("GPUTime")
        for r in range(0, recipe["repeats"], 2):
            for i in range(8, n):
                a = recipe["start"] + r * (n + recipe["gap"]) + i
                b = a + n + recipe["gap"]
                pairs.append(float(data[b][col]) - float(data[a][col]))
                frame_costs.append(dict(pair=r // 2, frame=i, phase=recipe["frames"][i].get("phase"),
                     no_aa_gpu_ms=float(data[a][col]), tsr_gpu_ms=float(data[b][col]),
                     tsr_pass_ms=float(data[b][header.index("GPU/TemporalSuperResolution")]) if "GPU/TemporalSuperResolution" in header else None))
        save(out / "timing.json", dict(by_method=by_method, paired_delta_p50_ms=float(np.median(pairs)),
             paired_delta_p95_ms=float(np.quantile(pairs, .95)), pairs=len(pairs), valid_timing=not recipe["smoke"],
             per_frame=frame_costs,
             activity_policy=policy, contended=policy.get("state") != "quiet",
             eligible_for_speed_claim=policy.get("state") == "quiet" and policy.get("speed_claim_eligible", False),
             quiet_window=policy.get("quiet_window"),
             scope="2 adjacent no-AA/TSR full-track pairs per process, same frame indices, first 8 frames excluded per track. Four processes give eight pairs. Native CSV GPUTime plus TSR stat. Desktop contention recorded in preflight."))
        print(json.dumps(by_method, indent=2))
        return
    dump = NATIVE.s1.native.Dump(out / "dump")
    grouped = {}
    for event in dump.passes:
        frame = next(int(m.group(1)) for scope in event["ParentEventScopes"] if (m := re.match(r"Frame (\d+)", scope)))
        frame_scope = next(s for s in event["ParentEventScopes"] if s.startswith("Frame "))
        delta = re.search(r"Delta=([0-9.]+)s", frame_scope)
        if delta is None or abs(float(delta.group(1)) - 1 / 60) > 1e-6:
            raise ValueError("native simulation delta differs from registered 60 Hz")
        grouped.setdefault(frame, []).append(event)
    if len(grouped) != n:
        raise ValueError(f"native frame coverage {len(grouped)} != {n}")
    identities = []
    previous_hold = None
    hold_steps = []
    hold_indices = [i for i, p in enumerate(recipe["frames"]) if p.get("phase") == "hold"]
    hold_curve = []
    target_codes = None
    def codes_for(value):
        linear = np.maximum(0, value[..., :3])
        linear /= 1 + linear
        return 255 * np.where(linear <= .0031308, linear * 12.92, 1.055 * linear ** (1 / 2.4) - .055)
    if hold_indices:
        target_events = sorted(grouped.items())[hold_indices[-1]][1]
        p, name = colour_input(dump, target_events, recipe)
        target_codes = codes_for(NATIVE.resource(dump, name, p)[0])
    for i, (frame, events) in enumerate(sorted(grouped.items())):
        consumer, name = colour_input(dump, events, recipe)
        values, identity = NATIVE.resource(dump, name, consumer)
        if values.shape[:2] != (1080, 1920) or not np.isfinite(values).all():
            raise ValueError("invalid native post-TSR HDR")
        exposure = None
        eye_name = next((name for name in consumer["InputResources"]
                         if dump.resources[name.split(".")[0]]["Name"] == "EyeAdaptationBuffer"), None)
        if eye_name:
            paths = list((dump.root / "Resources").glob(eye_name + ".v*.bin"))
            if len(paths) == 1:
                eye = np.fromfile(paths[0], dtype="<f4")
                parameters = dump.scalar_parameters(consumer)
                tint = next((v for k, v in parameters.items() if k.endswith("ColorScale0")), None)
                exposure = dict(values=eye.tolist(), sha256=digest(paths[0]), scene_color_tint=tint,
                                manual_unit_local=bool(eye.size == 8 and eye[0] == 2 ** recipe.get("exposure_stops", -14) and eye[4] == 1
                                                      and tint is not None and np.array_equal(np.array(tint[:3]), [1, 1, 1])),
                                source="Tonemap EyeAdaptationBuffer[0].x global exposure; [1].x average local exposure; ColorScale0 tint. Manual PreExposure follows these in FViewInfo::UpdatePreExposure; warm state, unit tint/local.")
        temporal = []
        for event in events:
            if event["EventName"].startswith("TSR"):
                parameters = dump.scalar_parameters(event)
                temporal_values = {k: v for k, v in parameters.items() if k.endswith(("InputJitter", "bCameraCut", "PrevFrameIndex"))}
                if temporal_values:
                    temporal.append(dict(event=event["EventName"], values=temporal_values))
        if recipe.get("temporal_readback") and i < min(2, n):
            jitters = [v for entry in temporal for k, v in entry["values"].items() if k.endswith("InputJitter")]
            if not jitters or not all(np.allclose(v, jitters[0], rtol=0, atol=1e-7) for v in jitters):
                raise ValueError("missing or inconsistent initial TSR jitter readback")
        identities.append(dict(frame=i, engine_frame=frame, hdr=identity, exposure=exposure, temporal=temporal,
                               min=float(values.min()), max=float(values.max()), mean=float(values[..., :3].mean())))
        if recipe["frames"][i].get("phase") == "hold":
            codes = codes_for(values)
            if previous_hold is not None:
                hold_steps.append(dict(frame=i, rms=float(np.sqrt(np.mean((codes - previous_hold) ** 2)))))
            previous_hold = codes
            diff = np.abs(codes - target_codes)
            hold_curve.append(dict(frame=i, mean=float(diff.mean()), p99=float(np.quantile(diff, .99)),
                                   rms=float(np.sqrt(np.mean(diff ** 2)))))
        # Hashes prove capture coverage after raw retention cleanup; PNGs are inspection only.
        if i in (0, 8, 48, 64, 72, 95, 96, 120, 128, 160):
            display = NATIVE.s1.film.tone(values[..., :3])
            display.save(out / f'hdr-inspect-f{i:03d}.png')
    save(out / "capture-readback.json", dict(frames=n, identities=identities,
         simulation_delta_seconds=1 / 60,
         scope="Native pre-tonemap post-TSR colour, still pre-exposed; not F0 radiance or film visibility",
         exposure_readback_frames=sum(bool(r["exposure"] and r["exposure"]["manual_unit_local"]) for r in identities),
         missing=["U0 quarry reference", "T1b ages and populations", "reference-noise subtraction",
                  "F1-F5 and reference-subtracted motion F6 scores"]))
    if hold_curve:
        first = next((row["frame"] - hold_indices[0] for k, row in enumerate(hold_curve)
                      if all(r["mean"] < .5 and r["p99"] < 4 for r in hold_curve[k:])), None)
        save(out / "hold-diagnostic.json", dict(steps=hold_steps, curve=hold_curve, settle_offset=first,
             hold_step_rms_p50=float(np.median([r["rms"] for r in hold_steps])) if hold_steps else None,
             scope="Full-image F0-transform self residual in pre-exposed HDR. Target is last hold, not an independent film reference. No T1b population masks. Diagnostic only; not F2/F5/F6 acceptance."))
    print("native HDR frames", n, flush=True)


def clean(args):
    root = (ROOT / "runs" / args.label).resolve()
    if ROOT.resolve() not in root.parents or not root.is_dir():
        raise ValueError("cleanup outside owned run")
    if not any((root / name).exists() for name in ("capture-readback.json", "history-readback.json", "reference-readback.json")):
        raise ValueError("summarize before cleanup")
    deleted = []
    for path in root.rglob("*"):
        if path.is_file() and path.stat().st_size > 5_000_000 and path.suffix.lower() in (".bin", ".npz", ".npy", ".exr", ".rgba16f", ".u32", ".f32"):
            deleted.append(dict(path=str(path), bytes=path.stat().st_size, sha256=digest(path)))
            path.unlink()
    previous = root / "deleted-raw.json"
    save(previous, (json.loads(previous.read_text()) if previous.exists() else []) + deleted)
    print("deleted", sum(r["bytes"] for r in deleted), "bytes")


def prune(args):
    """Keep only the consumed HDR among large native resources while awaiting references."""
    root = (ROOT / "runs" / args.label).resolve()
    if ROOT.resolve() not in root.parents or not root.is_dir():
        raise ValueError("prune outside owned run")
    if (root / "capture-readback.json").exists():
        summary = json.loads((root / "capture-readback.json").read_text())
        keep = {Path(r["hdr"]["path"]).resolve() for r in summary["identities"]}
        candidates = (root / "dump").rglob("*.bin")
    else:
        history = json.loads((root / "history-readback.json").read_text())
        if not history["age_masks_compatible"]:
            raise ValueError("audit historical replay before pruning")
        keep = {p.resolve() for p in (root / "data").glob("full-f*.rgba16f")}
        candidates = (root / "data").glob("*.rgba16f")
    deleted = []
    for path in candidates:
        if path.stat().st_size > 5_000_000 and path.resolve() not in keep:
            deleted.append(dict(path=str(path), bytes=path.stat().st_size, sha256=digest(path)))
            path.unlink()
    previous = root / "pruned-raw.json"
    save(previous, (json.loads(previous.read_text()) if previous.exists() else []) + deleted)
    print("pruned unused resources", sum(r["bytes"] for r in deleted), "bytes; consumed HDR kept")


def baseline(_):
    """Extract the immutable recorded T1b scores without recomputing or changing their bar."""
    source = T1 / "scores-walk/score.json"
    spread_path = T1 / "scores-t1k/score.json"
    data = json.loads(source.read_text())
    spread = json.loads(spread_path.read_text())["poses"]["0"]["bars"]
    rows = []
    for frame, result in data["frames"].items():
        if not result.get("converged"):
            continue
        for channel, populations in result["converged"]["t1"].items():
            for population, score in populations.items():
                exact = result["converged"]["exact"][channel][population]["rms"]
                bar = exact * (1 + spread.get(channel, {}).get(population, {}).get("spread_2sd", 0))
                rows.append(dict(frame=int(frame), phase=result["phase"], channel=channel, population=population,
                                 n=score["n"], t1b_rms=score["rms"], exact_rms=exact, bar=bar,
                                 t1b_ratio=score["rms"] / bar if bar else None,
                                 tsr_epic=None, tsr_high=None,
                                 tsr_status="pending matched reference, population and age-mask reconstruction"))
    out = ROOT / "t1b-recorded.json"
    save(out, dict(source=str(source), source_sha256=digest(source), spread_sha256=digest(spread_path),
                   commit="b8fa1f3", rows=rows, gates=data["gates"],
                   scope="Carried historical T1b scores; not rerendered, new references do not replace their bar."))
    print("recorded T1b population/channel/frame rows", len(rows))


def cost(_):
    """Pool only completed, readback-validated paired visits; keep all visit identities."""
    groups = {}
    for path in (ROOT / "runs").glob("*/timing.json"):
        timing = json.loads(path.read_text())
        if not timing.get("valid_timing") or "per_frame" not in timing:
            continue
        recipe = json.loads((path.parent / "recipe.json").read_text())
        process = json.loads((path.parent / "process.json").read_text())
        readback = json.loads((path.parent / "readback.json").read_text())
        if process["exit_code"] or process["failures"] or not readback["ok"]:
            raise ValueError("invalid process was labelled valid timing")
        quiet_root = ROOT / "quiet" / str(timing.get("quiet_window"))
        certificate, validation = quiet_root / "certificate.json", quiet_root / "readback-validation.json"
        qualified = bool(timing.get("eligible_for_speed_claim") and certificate.exists() and validation.exists() and
                         json.loads(certificate.read_text()).get("eligible_for_speed_claim") and
                         json.loads(validation.read_text()).get("passed"))
        key = (recipe["track"], recipe["path"], recipe["quality"], recipe.get("program", "full"), "quiet" if qualified else "screen")
        group = groups.setdefault(key, dict(visits=[], samples=[]))
        group["visits"].append(dict(label=path.parent.name, preflight_sha256=digest(path.parent / "preflight.json"),
                                     process_sha256=digest(path.parent / "process.json"),
                                     minimum_available_gib=process["minimum_available_gib"],
                                     activity_policy=timing.get("activity_policy", {"state": "pre-game-screen"}),
                                     eligible_for_speed_claim=qualified,
                                     quiet_window=timing.get("quiet_window")))
        group["samples"].extend(timing["per_frame"])
    rows, per_frame = [], []
    for (track, shadow_path, quality, program, scope), group in groups.items():
        samples = group.pop("samples")
        row = dict(track=track, path=shadow_path, quality=quality, program=program, scope=scope, **group,
                   pairs=len(group["visits"]) * 2, paired_frames=len(samples), stats={})
        for key in ("no_aa_gpu_ms", "tsr_gpu_ms", "tsr_pass_ms", "frame_increment_ms"):
            values = [s["tsr_gpu_ms"] - s["no_aa_gpu_ms"] if key == "frame_increment_ms" else s[key] for s in samples]
            row["stats"][key] = dict(p50=float(np.median(values)), p95=float(np.quantile(values, .95)),
                                     p99=float(np.quantile(values, .99)), minimum=float(min(values)), maximum=float(max(values)))
        if scope == "quiet":
            for frame in sorted({s["frame"] for s in samples}):
                chosen = [s for s in samples if s["frame"] == frame]
                r = dict(track=track, path=shadow_path, quality=quality, frame=frame,
                         phase=chosen[0]["phase"], pairs=len(chosen))
                for key in ("no_aa_gpu_ms", "tsr_gpu_ms", "tsr_pass_ms", "frame_increment_ms"):
                    values = [s["tsr_gpu_ms"] - s["no_aa_gpu_ms"] if key == "frame_increment_ms" else s[key] for s in chosen]
                    r[key + "_p50"] = float(np.median(values))
                    r[key + "_p95"] = float(np.quantile(values, .95))
                per_frame.append(r)
        rows.append(row)
    save(ROOT / "cost-summary.json", dict(rows=rows,
         note="Quiet certificates and desktop screens are separate rows. All completed paired visits, no favourable-visit filtering. Capture/failed/memory-aborted runs excluded. No TR1/default claim."))
    if per_frame:
        with (ROOT / "cost-per-frame.csv").open("w", newline="", encoding="utf-8") as stream:
            writer = csv.DictWriter(stream, fieldnames=list(per_frame[0]))
            writer.writeheader()
            writer.writerows(per_frame)
    print(json.dumps([dict(track=r["track"], path=r["path"], quality=r["quality"], scope=r["scope"],
                           pairs=r["pairs"], stats=r["stats"]) for r in rows], indent=2))


def quiet(args):
    """One exclusive cost window, second in Islam's release order; no nested leases."""
    policy = headless_policy(args.minutes)
    if policy["state"] != "released":
        raise SystemExit("quiet costs wait for GAME HOLD RELEASED")
    root = ROOT / "quiet" / args.label
    root.mkdir(parents=True, exist_ok=False)
    (root / "measurement-source.py").write_bytes(SCRIPT_BYTES)
    deadline = time.monotonic() + args.wait_seconds
    release_time = dt.datetime.fromisoformat(re.search(r"\d{4}-\d\d-\d\dT[0-9:]+Z", policy["directive"]).group())
    while True:
        lines = U0.SCHEDULE.read_text(encoding="utf-8").splitlines()
        preceding = [line for line in lines if line.startswith("rt1c ") and "quiet" in line.lower() and "released" in line]
        ready = any((m := re.search(r"released (\S+)", line)) and dt.datetime.fromisoformat(m.group(1)) > release_time for line in preceding)
        if ready and not U0.LOCK.exists() and U0.E2E.available_gib() >= 4:
            break
        if time.monotonic() >= deadline:
            raise SystemExit("quiet queue deferred; rt1c quiet release/slot/headroom not ready")
        if activity_policy()["state"] != "released":
            raise SystemExit("quiet queue stopped by game directive")
        time.sleep(2)
    spec = importlib.util.spec_from_file_location("u0b_quiet", args.quiet_tool)
    helper_bytes = args.quiet_tool.read_bytes()
    (root / "quiet-helper-source.py").write_bytes(helper_bytes)
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)
    if args.quiet_tool.read_bytes() != helper_bytes:
        raise RuntimeError("quiet helper changed while loading; no lease acquired")
    variants = [(track, path, quality) for track in ("q-walk", "s2g-camera-1px")
                for path in ("vsm-nort", "hrt") for quality in (3, 2)]
    visits = []
    complete = False
    original_lease = U0.lease
    with original_lease(args.label, args.minutes) as owner:
        try:
            with helper.quiet_desktop(U0, root) as quiet_window:
                proof_root = Path(quiet_window) if quiet_window else root
                U0.lease = lambda label, minutes: contextlib.nullcontext(owner)
                for visit in range(4):
                    for track, shadow_path, quality in variants if visit % 2 == 0 else reversed(variants):
                        if U0.remaining_seconds(owner) < 45 or activity_policy()["directive"] != policy["directive"]:
                            raise RuntimeError("quiet lease/directive ended before matrix completed")
                        label = f"{args.label}-{'q' if track == 'q-walk' else 'g'}-{'n' if shadow_path == 'vsm-nort' else 'r'}-{'e' if quality == 3 else 'h'}-{visit}"
                        print("quiet visit", label, flush=True)
                        run(argparse.Namespace(label=label, track=track, quality=quality, path=shadow_path,
                            capture=False, smoke=False, program="full", minutes=2, wait_seconds=0, package_dir=None,
                            timeout_seconds=min(100, int(U0.remaining_seconds(owner)) - 10)))
                        save(ROOT / "runs" / label / "activity-policy.json", dict(state="quiet", speed_claim_eligible=True,
                             quiet_window=args.label, release_directive=policy["directive"], wallpaper_start=str(proof_root / "quiet-start.json")))
                        summarize(argparse.Namespace(label=label))
                        visits.append(label)
                        save(root / "visits.json", visits)
            complete = True
        finally:
            U0.lease = original_lease
            save(root / "certificate.json", dict(complete=complete, visits=visits, script_sha256=SCRIPT_SHA,
                 helper_sha256=hashlib.sha256(helper_bytes).hexdigest(), preceding_rt1c_release=preceding,
                 eligible_for_speed_claim=complete and activity_policy()["directive"] == policy["directive"],
                 note="Exclusive outer lease, game released, supported wallpaper pause restored in finally, <=3% GPU start, four processes/two A-B pairs each per configuration. No captures or Cargo in this window."))
    quiet_check(argparse.Namespace(label=args.label))
    cost(None)


def quiet_check(args):
    """Validate recorded AA toggles and every qualified process without new GPU work."""
    root = ROOT / "quiet" / args.label
    certificate = json.loads((root / "certificate.json").read_text())
    proofs = list(root.rglob("quiet-start.json"))
    checks = []
    for label in certificate["visits"]:
        out = ROOT / "runs" / label
        recipe, process, readback = [json.loads((out / f).read_text()) for f in ("recipe.json", "process.json", "readback.json")]
        csv_rows = list(csv.reader((out / "profile.csv").open(encoding="utf-8-sig")))
        header, rows = csv_rows[0], csv_rows[1:]
        column = header.index("GPU/TemporalSuperResolution")
        ranges = []
        toggle_ok = True
        n = len(recipe["frames"])
        for repeat in range(recipe["repeats"]):
            values = [float(rows[recipe["start"] + repeat * (n + recipe["gap"]) + i][column] or 0) for i in range(8, n)]
            ranges.append([min(values), max(values)])
            toggle_ok &= max(values) == 0 if repeat % 2 == 0 else min(values) > 0
        checks.append(dict(label=label, toggle_ranges=ranges, toggles_ok=bool(toggle_ok),
            runtime_ok=readback["ok"] and readback["d3d12_sm6"] and readback["sun_angle"]["ok"],
            process_ok=process["exit_code"] == 0 and not process["failures"] and process["minimum_available_gib"] >= 2,
            csv_sha256=digest(out / "profile.csv"), minimum_available_gib=process["minimum_available_gib"]))
    proof_ok = len(proofs) == 1 and float(json.loads(proofs[0].read_text())[-1]["gpu"].split(",")[2]) <= 3
    passed = certificate["eligible_for_speed_claim"] and len(checks) == 32 and proof_ok and all(
        c["toggles_ok"] and c["runtime_ok"] and c["process_ok"] for c in checks)
    save(root / "readback-validation.json", dict(passed=bool(passed), checks=checks,
         quiet_start=str(proofs[0]) if proofs else None, quiet_start_ok=proof_ok,
         note="CSV zero-TSR/no-AA and positive-TSR blocks checked at every measured frame; runtime settings, SM6, exact sun diameter, process exit and live memory guards checked. No GPU rerun."))
    if not passed:
        raise ValueError("quiet readback validation failed")
    print("quiet readback passed", len(checks), "processes", flush=True)


def history(args):
    """Recover T1b's deleted age masks with its original archived executable, no build."""
    binary = T1 / "builds/b15/renderlab-test.exe"
    expected = (T1 / "builds/b15/sha256.txt").read_text().split()[0]
    if digest(binary) != expected:
        raise ValueError("archived T1b executable hash changed")
    policy = headless_policy(args.minutes)
    if args.wait_seconds:
        deadline = time.monotonic() + args.wait_seconds
        print("Waiting for bounded historical-replay GPU gap", flush=True)
        while U0.E2E.available_gib() < U0.START_GIB:
            if time.monotonic() >= deadline:
                raise SystemExit(3)
            time.sleep(2)
    out = ROOT / "runs" / args.label
    out.mkdir(parents=True, exist_ok=False)
    manifest = json.loads((T1 / "walk-b15.json").read_text())
    manifest.update(output=str(out / "data"), replay=False)
    if args.black_background:
        lighting = json.loads(Path(manifest["lighting"]).read_text())
        lighting["environment"] = [0, 0, 0]
        save(out / "lighting-black-background.json", lighting)
        manifest.update(lighting=str(out / "lighting-black-background.json"), validate=False)
    save(out / "manifest.json", manifest)
    env = dict(os.environ, RENDERLAB_PRIMARY_MANIFEST=str(out / "manifest.json"),
               RAYON_NUM_THREADS="2", OPENBLAS_NUM_THREADS="2", OMP_NUM_THREADS="2",
               RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
    test = "renderer::fresh_shadow_tests::primary_ray_gpu_tests::film_reference_tests::film_accumulation_tests::gpu_film_t1_motion"
    policy = headless_policy(args.minutes)
    save(out / "activity-policy.json", policy)
    with queued_lease(args.label, args.minutes, args.wait_seconds):
        U0.guarded([binary, test, "--ignored", "--exact", "--nocapture", "--test-threads=1"],
                   out, timeout=min(args.minutes * 60 - 15, 120) if policy["state"] == "relaxed" else args.minutes * 60 - 15, cwd=REPO, env=env)
    original_path = T1 / "walk-b15/motion.json"
    actual = json.loads((out / "data/motion.json").read_text())
    original = json.loads(original_path.read_text())
    checks = {program: actual["digests"][program][0] == original["digests"][program][0]
              for program in original["digests"]}
    save(out / "history-readback.json", dict(binary_sha256=expected, original_sha256=digest(original_path),
         checks=checks, same_scored_frames=actual["scored"] == original["scored"],
         age_masks_compatible=all(checks.values()) and actual["scored"] == original["scored"],
         note="All scored image hashes and final histories must match before reusing restored masks with recorded T1b scores."))
    if not all(checks.values()) or actual["scored"] != original["scored"]:
        raise ValueError("historical image/history replay differs; restored masks are not yet accepted")
    print("historical T1b image/history identity passed", checks, flush=True)


def history_check(args):
    """Audit completed historical data independently of the test process's exit status."""
    out = ROOT / "runs" / args.label
    original_path = T1 / "walk-b15/motion.json"
    actual_path = out / "data/motion.json"
    actual, original = [json.loads(p.read_text()) for p in (actual_path, original_path)]
    checks = {p: actual["digests"][p][0] == original["digests"][p][0] for p in original["digests"]}
    counters = actual["frames"] == original["frames"]
    process = json.loads((out / "process.json").read_text())
    log = (out / "stdout.log").read_text(errors="replace")
    errors = [line for line in log.splitlines() if "gpu_diagnostic ERROR" in line]
    result = dict(checks=checks, per_frame_counters_identical=counters,
         scored_frames_identical=actual["scored"] == original["scored"],
         image_hashes=sum(len(actual["digests"][p][0]) - 1 for p in checks),
         final_histories=len(checks), frame_counter_rows=len(actual["frames"]),
         original_sha256=digest(original_path), replay_sha256=digest(actual_path),
         process_exit_code=process["exit_code"], clean_test_pass=process["exit_code"] == 0 and not errors,
         diagnostic_errors=errors,
         age_masks_compatible=all(checks.values()) and counters and actual["scored"] == original["scored"],
         limit="Original age-file hashes were not retained. Compatibility follows the original binary, all image/final-history hashes and every per-frame counter; a nonzero test exit is never relabelled a clean test.")
    save(out / "history-readback.json", result)
    print(json.dumps(result, indent=2))


def black_check(args):
    """Accept a black-clear replay only if its shading and all age masks stay unchanged."""
    out, base = [ROOT / "runs" / n for n in (args.label, args.baseline)]
    actual, original = [json.loads((p / "data/motion.json").read_text()) for p in (out, base)]
    age_checks = [digest(out / "data" / f"age-f{i:03}.u8") == digest(base / "data" / f"age-f{i:03}.u8") for i in actual["scored"]]
    controls = {p: actual["digests"][p][0] == original["digests"][p][0] for p in ("sun", "visibility")}
    old, new = [np.fromfile(p / "data/full-f000.rgba16f", "<f2").reshape(1080, 1920, 4) for p in (base, out)]
    with np.load(args.reference) as ref:
        cover = ref["pop_all-covered"]
    foreground_identical = np.array_equal(old[cover], new[cover])
    background_black = np.array_equal(new[0, 0, :3], [0, 0, 0])
    counters = actual["frames"] == original["frames"]
    passed = all(age_checks) and all(controls.values()) and foreground_identical and background_black and counters
    process = json.loads((out / "process.json").read_text())
    save(out / "history-readback.json", dict(age_masks_compatible=passed, age_files=len(age_checks),
         every_age_file_identical=all(age_checks), unchanged_control_programs=controls,
         cold_covered_pixels=int(cover.sum()), cold_foreground_hdr_identical=foreground_identical,
         black_background=background_black, per_frame_counters_identical=counters,
         baseline_history_sha256=digest(base / "data/motion.json"), process_exit_code=process["exit_code"],
         validation_enabled=False,
         note="Explicit black visible-clear benchmark variant; the RLE2 illumination stays bound. Never relabelled as the original coloured-clear recorded score. Cold foreground and sun/visibility controls plus every age file must remain byte-identical."))
    if not passed:
        raise ValueError("black-background variant changed shading or masks; cannot use it")
    print("black-clear compatibility passed", int(cover.sum()), "cold covered pixels; age files", len(age_checks))


def reference(args):
    """Run U0b's retained tracer on unchanged S2G geometry and registered zero-shutter poses."""
    policy = headless_policy(args.minutes)
    if not args.track.startswith("s2g-"):
        raise ValueError("quarry references are supplied by U0b; this interface owns S2G only")
    if policy["state"] == "relaxed" and len(args.frames) > 3:
        raise ValueError("at most three reference frames per game-play batch")
    sequence = json.loads((ROOT / "inputs" / (args.track + ".json")).read_text())
    source = Path("E:/renderlab-work/m7-u0b/scenes/S2G/scene.json")
    scene = json.loads(source.read_text())
    if scene.get("camera_background") != [0., 0., 0.]:
        raise ValueError("S2G reference source must preserve sky illumination with black primary misses")
    if scene["size"] != [1920, 1080]:
        raise ValueError("reference source must retain registered 1920x1080 size")
    if args.wait_seconds:
        deadline = time.monotonic() + args.wait_seconds
        while U0.E2E.available_gib() < U0.START_GIB:
            if time.monotonic() >= deadline:
                print("deferred: no reference lease acquired", flush=True)
                raise SystemExit(3)
            time.sleep(2)
    root = ROOT / "runs" / args.label
    root.mkdir(parents=True, exist_ok=False)
    (root / "reference-source.py").write_bytes(SCRIPT_BYTES)
    output = root / "data"
    output.mkdir()
    cases, entries = [], []
    for index in args.frames:
        p = sequence["frames"][index]
        copy_scene = dict(scene, poses=[dict(name=args.track.removeprefix("s2g-") + f"-{index:03}",
                                            eye=p["eye"], target=p["target"])])
        copy_scene["sun"] = dict(scene["sun"], to_light=p["to_light"])
        destination = root / "inputs" / str(index) / "s2g" / "scene.json"
        destination.parent.mkdir(parents=True)
        destination.write_text(json.dumps(copy_scene, separators=(",", ":")), encoding="utf-8")
        name = args.track + f"-{index:03}"
        cases.append(dict(name=name, frame=dict(name=name, open={k: p[k] for k in ("eye", "target")},
                         close={k: p[k] for k in ("eye", "target")}, to_light=p["to_light"],
                         fov_y=p["fov_y_radians"], width=1920, height=1080, sky_only=False),
                         input=str(destination), input_sha256=digest(destination)))
        entries.append(str(destination.parent) + "@" + str(args.samples))
    binary_sha = digest(args.binary)
    pinned_binary = root / "reference-tests.exe"
    shutil.copyfile(args.binary, pinned_binary)
    if digest(pinned_binary) != binary_sha or digest(args.binary) != binary_sha:
        raise ValueError("reference executable changed while pinning")
    save(root / "reference-recipe.json", dict(cases=cases, source=str(source), source_sha256=digest(source),
         binary=str(args.binary), pinned_binary=str(pinned_binary), binary_sha256=binary_sha, compactor=str(args.compact_tool),
         compactor_sha256=digest(args.compact_tool), samples=args.samples, activity_policy=policy,
         reserved_minutes=args.minutes, functional_timeout_seconds=getattr(args, "timeout_seconds", 120),
         speed_claim_eligible=False,
         illumination="Original S2G constant sky and finite-disk sun; camera_background black before integration"))
    env = dict(os.environ, RENDERLAB_M7_BENCHES=";".join(entries), RENDERLAB_M7_REFERENCE_OUT=str(output),
               RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
               VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation", RAYON_NUM_THREADS="2",
               OMP_NUM_THREADS="2", OPENBLAS_NUM_THREADS="2", RENDERLAB_M7_CAPTURE_GEOMETRY="1")
    env.pop("VK_INSTANCE_LAYERS", None)
    test = "reference::tests::gpu_reference_benches"
    policy = headless_policy(args.minutes)
    timeout = getattr(args, "timeout_seconds", 120)
    if not 0 < timeout <= 600 or timeout > args.minutes * 60 - 15 or (policy["state"] == "relaxed" and timeout > 120):
        raise ValueError("functional timeout exceeds the reserved/game-play allowance")
    with queued_lease(args.label, args.minutes, args.wait_seconds, correctness_only=True):
        try:
            U0.guarded([pinned_binary, test, "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                       root, timeout=timeout, cwd=REPO, env=env)
        except (RuntimeError, SystemExit):
            process_path, preflight_path = root / "process.json", root / "preflight.json"
            record = json.loads(process_path.read_text()) if process_path.exists() else {}
            preflight = json.loads(preflight_path.read_text()) if preflight_path.exists() else {}
            minimum = record.get("minimum_available_gib", preflight.get("available_gib"))
            if (minimum is not None and minimum < 4) or "monitor_error" in json.dumps([record, preflight]):
                save(ROOT / "resource-pause.json", dict(reason="reference resource/monitor failure", run=args.label,
                     utc=U0.utc().isoformat(), cargo_free_since=None, policy="Wait outside lease for 60 Cargo-free seconds; keep >=4GiB start/<2GiB abort"))
            raise
    if digest(pinned_binary) != binary_sha:
        raise ValueError("pinned reference executable changed during the run")
    spec = importlib.util.spec_from_file_location("u0b_compact", args.compact_tool)
    compact = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(compact)
    for case in cases:
        path = output / (case["name"] + ".json")
        meta = json.loads(path.read_text())
        if meta["scene_sha256"] != case["input_sha256"] or meta["invalid_pixels"]:
            raise ValueError("reference scene identity or finite-pixel check failed")
        meta.update(frame=case["frame"], bench="S2G", camera_background=[0., 0., 0.],
                    reference_binary_sha256=binary_sha, source_scene_sha256=digest(source),
                    population_instances={name: [i + 1 for i, mesh in enumerate(scene["meshes"]) if mesh["name"] in names]
                                          for name, names in scene["populations"].items()})
        save(path, meta)
        compact.compact(argparse.Namespace(root=output, name=case["name"]))
    replay = json.loads((output / "replay.json").read_text())
    if not replay["bit_identical"]:
        raise ValueError("U0b reference same-seed replay differs")
    save(root / "reference-readback.json", dict(cases=cases, binary_sha256=binary_sha, replay=replay, complete=True))
    # These generated copies are reproducible from the retained scene and small recipe.
    deleted = []
    for case in cases:
        path = Path(case["input"])
        if root.resolve() not in path.resolve().parents:
            raise ValueError("generated-scene cleanup outside owned reference run")
        deleted.append(dict(path=str(path), bytes=path.stat().st_size, sha256=digest(path)))
        path.unlink()
    save(root / "deleted-generated-scenes.json", deleted)
    print("reference cases compacted", len(cases), output, flush=True)


def score(args):
    """Score native colour against U0b lossless, zero-shutter physical references.

    This supplies R_L-style measurements, not T1b's old direct/environment bar,
    internal restart ages or an invented post-TSR visibility output.
    """
    from scipy.ndimage import distance_transform_edt
    started = time.monotonic()
    if args.output_tag and not re.fullmatch(r"[A-Za-z0-9_-]+", args.output_tag):
        raise ValueError("batch output tag must be a plain filename label")
    out = ROOT / "runs" / args.label
    recipe = json.loads((out / "recipe.json").read_text())
    program = recipe.get("program", "full")
    prefix = "D" if program == "sun" else "L"
    selected_frames = set(args.frames) if args.frames else None
    (out / ("physical-witness-source.py" if selected_frames else "scoring-source.py")).write_bytes(SCRIPT_BYTES)
    capture = json.loads((out / "capture-readback.json").read_text())
    if not recipe["capture"] or recipe["smoke"] or capture.get("exposure_readback_frames") != len(recipe["frames"]):
        raise ValueError("full capture and per-frame exposure proof required before scoring")
    moving_light = any("to_light" in p for p in recipe["frames"])
    def camera_key(p):
        # The reference View API and T1b camera are f32; canonicalize their effective input bits.
        values = p["eye"] + p["target"]
        if moving_light:
            if "to_light" not in p:
                return None
            values += p["to_light"]
        return np.asarray(values, dtype="<f4").tobytes()
    references = {}
    paths = list(args.reference.glob(("s2g-" if recipe.get("scene") == "S2G" else recipe["track"] + "-") + "*.json"))
    if recipe["track"] == "q-walk":
        paths += list(args.reference.glob("quarry-p*.json"))
    for path in paths:
        meta = json.loads(path.read_text())
        if meta.get("accepted_for_scoring") is False:
            raise ValueError("reference archive is explicitly unaccepted: " + str(path))
        frame = meta.get("frame", {})
        if not frame or frame.get("open") != frame.get("close") or frame.get("sky_only"):
            continue
        archive = path.with_suffix(".npz")
        if archive.exists():
            key = camera_key(dict(frame["open"], **({"to_light": frame["to_light"]} if "to_light" in frame else {})))
            if key is not None:
                if meta.get("camera_background") != [0., 0., 0.]:
                    raise ValueError("reference must apply black to primary misses before integration")
                references[key] = (archive, meta)
    if not references:
        raise ValueError("no lossless zero-shutter references available; no quality score can be published")
    arrival = {}
    for f, p in enumerate(recipe["frames"]):
        arrival.setdefault(camera_key(p), f)
    dump = NATIVE.s1.native.Dump(out / "dump")
    groups = {}
    for event in dump.passes:
        frame = next(int(m.group(1)) for s in event["ParentEventScopes"] if (m := re.match(r"Frame (\d+)", s)))
        groups.setdefault(frame, []).append(event)
    if len(groups) != len(recipe["frames"]):
        raise ValueError("native/reference frame coverage mismatch")
    codes = load("m6-temporal-gpu-score").codes
    historical = None
    if args.history:
        historical = ROOT / "runs" / args.history
        check = json.loads((historical / "history-readback.json").read_text())
        if not check["age_masks_compatible"]:
            raise ValueError("historical image/history/counter identity is not verified")
    if args.ages:
        if historical:
            raise ValueError("external geometry ages cannot replace historical T1b controls")
        protocol = json.loads((args.ages / "protocol.json").read_text())
        if protocol["source_sequences"][recipe["track"]] != recipe["sequence_sha256"]:
            raise ValueError("external ages use a different registered sequence")
    rows, residuals, missing, transient = [], [], [], {}
    previous = None
    cached_archive, cached_arrays, cached_sha = None, None, None
    for i, (_, events) in enumerate(sorted(groups.items())):
        if selected_frames is not None and i not in selected_frames:
            previous = None
            continue
        p = recipe["frames"][i]
        source = references.get(camera_key(p))
        if source is None:
            missing.append(i)
            previous = None
            continue
        archive, meta = source
        frame = meta["frame"]
        # View consumes f32. Qualified external metadata may serialize that
        # rounded value while our recipe retains its original f64 literal.
        reference_fov = np.asarray(frame["fov_y"], dtype="<f4").tobytes()
        requested_fov = np.asarray(p["fov_y_radians"], dtype="<f4").tobytes()
        if (frame["width"], frame["height"]) != (1920, 1080) or reference_fov != requested_fov:
            raise ValueError("reference projection mismatch")
        consumer, name = colour_input(dump, events, recipe)
        hdr, identity = NATIVE.resource(dump, name, consumer)
        if identity["sha256"] != capture["identities"][i]["hdr"]["sha256"]:
            raise ValueError("candidate bytes changed since capture readback")
        # Exposure diagnostics use measured native PE to return to registered
        # display units. This fixed conversion is never fitted to the image.
        registered_scale = 2 ** recipe.get("registered_exposure_stops", recipe.get("exposure_stops", -14))
        measured_scale = capture["identities"][i]["exposure"]["values"][0]
        hdr = hdr * (registered_scale / measured_scale)
        old_hdr, age = None, None
        if historical:
            age_path = historical / "data" / f"age-f{i:03}.u8"
            hdr_path = historical / "data" / f"{program}-f{i:03}.rgba16f"
            if age_path.exists():
                age = np.fromfile(age_path, dtype="u1").reshape(1080, 1920)
            if hdr_path.exists():
                old_hdr = np.fromfile(hdr_path, dtype="<f2").reshape(1080, 1920, 4)[..., :3].astype("f4")
        if args.ages:
            with np.load(args.ages / recipe["track"] / f"f{i:03}.npz") as age_file:
                age = age_file["age"]
            if age.shape != (1080, 1920) or age.dtype != np.uint8:
                raise ValueError("invalid external geometry-age buffer")
        if archive != cached_archive:
            # Validate exactly the bytes decoded, once per loaded snapshot.
            # Unused archives need no bulk read, and hashing does not cause a
            # separate second disk pass over every selected reference.
            payload = archive.read_bytes()
            archive_sha = hashlib.sha256(payload).hexdigest()
            if meta.get("archive_sha256") and archive_sha != meta["archive_sha256"]:
                raise ValueError("reference archive differs from its provenance")
            with np.load(io.BytesIO(payload)) as loaded:
                cached_arrays = {k: loaded[k] for k in loaded.files
                                 if k in ("L0", "L1", "D0", "D1", "visibility", "hit") or k.startswith("pop_")}
            del payload
            if recipe.get("scene") == "S2G":
                if "hit" not in cached_arrays or not meta.get("population_instances"):
                    raise ValueError("S2G scoring requires reference geometry and registered material populations")
                for name, instances in meta["population_instances"].items():
                    cached_arrays["pop_" + name] = np.isin(cached_arrays["hit"][..., 0], instances)
            cached_archive, cached_sha = archive, archive_sha
        with contextlib.nullcontext(cached_arrays) as ref:
            scale = registered_scale
            mean = (ref[prefix + "0"] + ref[prefix + "1"]) * .5 * scale
            halves = [ref[k] * scale for k in (prefix + "0", prefix + "1")]
            result = dict(frame=i, reference=str(archive), reference_sha256=cached_sha,
                          samples=meta.get("samples"), hdr_sha256=identity["sha256"], measures={}, transient_pools={},
                          projection=dict(width=frame["width"], height=frame["height"],
                               reference_fov_y_literal=frame["fov_y"], registered_fov_y_literal=p["fov_y_radians"],
                               effective_fov_y_f32=float(np.float32(p["fov_y_radians"])),
                               effective_fov_bits=int.from_bytes(reference_fov, "little")))
            if recipe.get("scene") == "S2G":
                reference_luma = mean @ np.array([.2126, .7152, .0722])
                candidate_luma = hdr[..., :3] @ np.array([.2126, .7152, .0722])
                highlights = {}
                for name in meta["population_instances"]:
                    mask = ref["pop_" + name]
                    if not mask.any():
                        continue
                    threshold = float(np.quantile(reference_luma[mask], .95))
                    highlight = mask & (reference_luma >= threshold)
                    coordinates = np.column_stack(np.nonzero(highlight)).astype("f8")
                    r, c = reference_luma[highlight], candidate_luma[highlight]
                    reference_centre = (coordinates * r[:, None]).sum(0) / max(float(r.sum()), 1e-30)
                    candidate_centre = (coordinates * c[:, None]).sum(0) / max(float(c.sum()), 1e-30)
                    r_peak = np.array(np.unravel_index(np.where(mask, reference_luma, -np.inf).argmax(), mask.shape))
                    c_peak = np.array(np.unravel_index(np.where(mask, candidate_luma, -np.inf).argmax(), mask.shape))
                    highlights[name] = dict(pixels=int(highlight.sum()), threshold_exposed_luminance=threshold,
                         energy_ratio=float(c.sum() / max(float(r.sum()), 1e-30)),
                         centroid_drift_px=float(np.linalg.norm(candidate_centre - reference_centre)),
                         peak_drift_px=float(np.linalg.norm(c_peak - r_peak)),
                         definition="Reference top-5% luminance within registered current-centre material mask; energy and centroid use that fixed mask, peaks use the entire material mask")
                result["highlights"] = highlights
            distance = distance_transform_edt(~ref["pop_edge"]) if "pop_edge" in ref else None
            result["background_distance_bands"] = {}
            result["background_error_extent"] = {}
            background_checks = []
            for y, x in ((0, 0), (0, 1919), (1079, 0), (1079, 1919)):
                if "pop_background-centre" in ref and ref["pop_background-centre"][y, x]:
                    value = dict(pixel=[x, y], reference_exposed=mean[y, x].tolist(),
                                 tsr_exposed=hdr[y, x, :3].tolist())
                    if old_hdr is not None:
                        value["t1b_exposed"] = old_hdr[y, x].tolist()
                    background_checks.append(value)
            result["background_checks"] = background_checks
            changed = None
            back = i - 8
            if back >= 0 and camera_key(recipe["frames"][back]) == camera_key(p):
                back = arrival[camera_key(p)] - 8
            back_source = references.get(camera_key(recipe["frames"][back])) if back >= 0 else None
            if back_source is not None and camera_key(recipe["frames"][back]) != camera_key(p):
                with np.load(back_source[0]) as before:
                    if all(k in ref and k in before for k in ("visibility", "D0", "D1")):
                        now_d = (ref["D0"] + ref["D1"]) * .5 * scale
                        before_d = (before["D0"] + before["D1"]) * .5 * scale
                        changed = (np.abs(ref["visibility"] - before["visibility"]) > .05) | (np.abs(codes(now_d, 4) - codes(before_d, 4)).mean(-1) > 8)
                        changed &= ref["pop_all-covered"]
                        result["changed"] = dict(back_frame=back, pixels=int(changed.sum()), measures={})
            for stops in (0, 4):
                candidate, reference = codes(hdr[..., :3], stops), codes(mean, stops)
                error = candidate - reference
                noise = (codes(halves[0], stops) - codes(halves[1], stops)) * .5
                e2, v2 = np.mean(error ** 2, -1), np.mean(noise ** 2, -1)
                old_error = codes(old_hdr, stops) - reference if old_hdr is not None else None
                old_e2 = np.mean(old_error ** 2, -1) if old_error is not None else None
                signed_error = error.mean(-1)
                old_signed_error = old_error.mean(-1) if old_error is not None else None
                pops = {}
                for key in ref:
                    if key.startswith("pop_"):
                        mask = ref[key]
                        if mask.any():
                            pops[key[4:]] = dict(n=int(mask.sum()), rms=float(np.sqrt(e2[mask].mean())),
                                  rms_noise_subtracted=float(np.sqrt(max(0, float((e2 - v2)[mask].mean())))),
                                  signed_mean=float(error[mask].mean()), maximum=float(np.max(np.abs(error[mask]))),
                                  reference_noise_rms=float(np.sqrt(v2[mask].mean())))
                            signed = signed_error[mask]
                            pops[key[4:]]["se"] = float(signed.std(ddof=1) / math.sqrt(signed.size)) if signed.size > 1 else 0
                            if old_e2 is not None:
                                pops[key[4:]]["t1b_replay_same_physical_reference"] = dict(
                                    rms=float(np.sqrt(old_e2[mask].mean())),
                                    rms_noise_subtracted=float(np.sqrt(max(0, float((old_e2 - v2)[mask].mean())))),
                                    signed_mean=float(old_error[mask].mean()),
                                    se=float(old_signed_error[mask].std(ddof=1) / math.sqrt(int(mask.sum()))) if mask.sum() > 1 else 0)
                                converged = mask & (age >= 32)
                                if converged.any():
                                    pops[key[4:]]["same_t1b_age32_pixels"] = dict(n=int(converged.sum()),
                                        tsr=float(np.sqrt(max(0, float((e2 - v2)[converged].mean())))),
                                        t1b=float(np.sqrt(max(0, float((old_e2 - v2)[converged].mean())))),
                                        tsr_signed_mean=float(error[converged].mean()),
                                        t1b_signed_mean=float(old_error[converged].mean()),
                                        tsr_se=float(signed_error[converged].std(ddof=1) / math.sqrt(int(converged.sum()))) if converged.sum() > 1 else 0,
                                        t1b_se=float(old_signed_error[converged].std(ddof=1) / math.sqrt(int(converged.sum()))) if converged.sum() > 1 else 0)
                            if age is not None:
                                converged = mask & (age >= 32)
                                if converged.any():
                                    pops[key[4:]]["shared_age32"] = dict(n=int(converged.sum()),
                                        rms_noise_subtracted=float(np.sqrt(max(0, float((e2 - v2)[converged].mean())))),
                                        signed_mean=float(error[converged].mean()))
                                fresh = mask & (age < 8)
                                for event in ("start", "revealed"):
                                    selected = fresh & ((age == i) if event == "start" else (age < i))
                                    if selected.any():
                                        bins = age[selected]
                                        counts = np.bincount(bins, minlength=8)
                                        fields = {"tsr_e2": e2, "reference_v2": v2}
                                        if old_e2 is not None:
                                            fields["t1b_e2"] = old_e2
                                        sums = {field: np.bincount(bins, weights=a[selected], minlength=8) for field, a in fields.items()}
                                        for value in np.flatnonzero(counts):
                                            name = f"{event}/{value}/{key[4:]}/{prefix}{stops}"
                                            for target in (transient, result["transient_pools"]):
                                                pool = target.setdefault(name, dict(n=0))
                                                pool["n"] += int(counts[value])
                                                for field, values in sums.items():
                                                    pool[field] = pool.get(field, 0.) + float(values[value])
                if distance is not None and "pop_background-centre" in ref:
                    bands = {}
                    for low, high in ((0, 1.5), (1.5, 3), (3, 6), (6, float("inf"))):
                        mask = ref["pop_background-centre"] & (distance >= low) & (distance < high)
                        if age is not None:
                            mask &= age >= 32
                        if mask.any():
                            band = dict(n=int(mask.sum()), tsr_rms=float(np.sqrt(e2[mask].mean())))
                            if old_e2 is not None:
                                band["t1b_rms"] = float(np.sqrt(old_e2[mask].mean()))
                            bands[f"{low:g}-{high:g}"] = band
                    result["background_distance_bands"][f"{prefix}{stops}"] = bands
                    base = ref["pop_background-centre"] & (distance >= 1.5)
                    if age is not None:
                        base &= age >= 32
                    extents = {}
                    for name, value in (("tsr", error), ("t1b", old_error)):
                        if value is None:
                            continue
                        magnitude = np.abs(value).mean(-1)
                        extents[name] = {}
                        for threshold in (1, 4, 8):
                            selected = base & (magnitude > threshold)
                            count = int(selected.sum())
                            extents[name][str(threshold)] = dict(pixels=count,
                                p95_distance_px=float(np.quantile(distance[selected], .95)) if count else 0,
                                maximum_distance_px=float(distance[selected].max()) if count else 0)
                    result["background_error_extent"][f"{prefix}{stops}"] = dict(values=extents,
                         age32_filter_available=age is not None,
                         definition="Background-centre pixels at least 1.5px from reference edge; mean absolute RGB error exceeds 1/4/8 F0 codes. Extent is nearest-edge distance, not a connected-trail length or the old visual-width estimate.")
                result["measures"][f"{prefix}{stops}"] = pops
                if changed is not None and changed.any():
                    field = dict(tsr_rms=float(np.sqrt(e2[changed].mean())), tsr_signed_mean=float(error[changed].mean()),
                         tsr_rms_noise_subtracted=float(np.sqrt(max(0, float((e2 - v2)[changed].mean())))),
                         tsr_se=float(signed_error[changed].std(ddof=1) / math.sqrt(int(changed.sum()))) if changed.sum() > 1 else 0)
                    if old_e2 is not None:
                        field.update(t1b_rms=float(np.sqrt(old_e2[changed].mean())), t1b_signed_mean=float(old_error[changed].mean()),
                            t1b_rms_noise_subtracted=float(np.sqrt(max(0, float((old_e2 - v2)[changed].mean())))),
                            t1b_se=float(old_signed_error[changed].std(ddof=1) / math.sqrt(int(changed.sum()))) if changed.sum() > 1 else 0)
                    result["changed"]["measures"][f"{prefix}{stops}"] = field
                if previous is not None and previous[0] == i - 1:
                    d_reference = reference - codes(previous[2], stops)
                    residual = candidate - codes(previous[1], stops) - d_reference
                    step = candidate - codes(previous[1], stops)
                    covered = (ref["pop_all-covered"] if "pop_all-covered" in ref else ref["pop_image"]) & previous[3]
                    if "hit" in ref and previous[4] is not None:
                        covered &= ref["hit"][..., 0] == previous[4]
                    if age is not None:
                        covered &= age >= 8
                    field = dict(frames=[i - 1, i], channel=f"{prefix}{stops}", pixels=int(covered.sum()),
                         phase=p.get("phase"), static=camera_key(p) == camera_key(recipe["frames"][i - 1]),
                         tsr_rms=float(np.sqrt((residual[covered] ** 2).mean())) if covered.any() else None,
                         tsr_step_rms=float(np.sqrt((step[covered] ** 2).mean())) if covered.any() else None,
                         age_filter_available=age is not None)
                    if meta.get("population_instances"):
                        field["material_populations"] = {}
                        for name in meta["population_instances"]:
                            selected = covered & ref["pop_" + name]
                            if selected.any():
                                field["material_populations"][name] = dict(n=int(selected.sum()),
                                    residual_rms=float(np.sqrt((residual[selected] ** 2).mean())),
                                    step_rms=float(np.sqrt((step[selected] ** 2).mean())))
                    if old_hdr is not None and previous[5] is not None and covered.any():
                        old_step = codes(old_hdr, stops) - codes(previous[5], stops)
                        field.update(t1b_rms=float(np.sqrt(((old_step - d_reference)[covered] ** 2).mean())),
                                     t1b_step_rms=float(np.sqrt((old_step[covered] ** 2).mean())))
                    residuals.append(field)
            previous = (i, hdr[..., :3].astype("f4"), mean,
                        ref["pop_all-covered"] if "pop_all-covered" in ref else ref["pop_image"],
                        ref["hit"][..., 0].copy() if "hit" in ref else None, old_hdr)
            rows.append(result)
            if len(rows) % 16 == 0:
                print("physical frames scored", len(rows), "through input", i, flush=True)
    for pool in transient.values():
        for name in ("tsr", "t1b"):
            if name + "_e2" in pool:
                pool[name + "_rms"] = math.sqrt(pool[name + "_e2"] / pool["n"])
                pool[name + "_rms_noise_subtracted"] = math.sqrt(max(0, (pool[name + "_e2"] - pool["reference_v2"]) / pool["n"]))
    original_checkpoints = set(json.loads((historical / "data/motion.json").read_text())["scored"]) if historical else set()
    covered_checkpoints = original_checkpoints & {r["frame"] for r in rows}
    output_path = out / "physical-batches" / (args.output_tag + ".json") if args.output_tag else out / ("physical-witness-scores.json" if selected_frames else "physical-scores.json")
    save(output_path, dict(
         rows=rows, requested_frames=sorted(selected_frames) if selected_frames else "all", missing_frames=missing, motion_residuals=residuals,
         transient_pools=transient, historical_variant=args.history, program=program,
         script_sha256=SCRIPT_SHA, recipe_sha256=digest(out / "recipe.json"), elapsed_seconds=time.monotonic() - started,
         diagnostic_only=recipe.get("diagnostic_only", False),
         registered_exposure_stops=recipe.get("registered_exposure_stops", recipe.get("exposure_stops", -14)),
         age_protocol="reference_geometry_v1" if args.ages else "historical_t1b" if historical else "unavailable",
         original_checkpoints=dict(total=len(original_checkpoints), scored=len(covered_checkpoints),
                                   missing=sorted(original_checkpoints - covered_checkpoints)),
         compared_with="U0b physical, disk-integrated multi-bounce reference, zero shutter, no fitting",
         carried_t1b_is_same_estimand=False,
         matched_temporal_comparison_certified=False,
         unavailable=["T1b original-bar ratios", "post-TSR R_F"] + (["post-TSR R_D"] if program == "full" else []) +
                     ["film-reference settle without every event frame"],
         note="Check primary-background semantics before any dominance claim. Corner diagnostics are evidence, not a replacement for full-image validation. Both raw and reference-noise-subtracted errors retained; sparse references cannot certify the motion gate."))
    print("physical reference frames scored", len(rows), "missing", len(missing))


def inspect(args):
    """Retain full-frame physical-reference comparisons before deleting raw captures."""
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    codes = load("m6-temporal-gpu-score").codes
    images, identities = [], []
    archive_sha = None
    track = None
    for label in args.labels:
        out = ROOT / "runs" / label
        recipe = json.loads((out / "recipe.json").read_text())
        if recipe.get("program", "full") != "full":
            raise ValueError("whole-image sheets currently require full-colour captures")
        if track is not None and recipe["track"] != track:
            raise ValueError("image sheet candidates use different registered tracks")
        track = recipe["track"]
        score_path = out / "physical-batches" / (args.score_tag + ".json") if args.score_tag else out / "physical-scores.json"
        score = json.loads(score_path.read_text())
        row = next(r for r in score["rows"] if r["frame"] == args.frame)
        if archive_sha is None:
            archive = Path(row["reference"])
            archive_sha = digest(archive)
            if archive_sha != row["reference_sha256"]:
                raise ValueError("reference changed before image inspection")
            with np.load(archive) as data:
                scale = 2 ** recipe.get("registered_exposure_stops", recipe.get("exposure_stops", -14))
                mean = (data["L0"] + data["L1"]) * .5 * scale
                halves = [data[k] * scale for k in ("L0", "L1")]
            images.append(("Physical reference", mean))
            if args.history:
                history = ROOT / "runs" / args.history
                if not json.loads((history / "history-readback.json").read_text())["age_masks_compatible"]:
                    raise ValueError("historical compatibility not verified")
                path = history / "data" / f"full-f{args.frame:03}.rgba16f"
                images.append(("T1b black replay", np.fromfile(path, "<f2").reshape(1080, 1920, 4)[..., :3].astype("f4")))
                identities.append(dict(label=args.history, sha256=digest(path)))
        if row["reference_sha256"] != archive_sha:
            raise ValueError("image sheet candidates use different physical targets")
        capture = json.loads((out / "capture-readback.json").read_text())
        engine_frame = capture["identities"][args.frame]["engine_frame"]
        dump = NATIVE.s1.native.Dump(out / "dump")
        events = [p for p in dump.passes if any(re.match(rf"Frame {engine_frame}(?:\D|$)", s) for s in p["ParentEventScopes"])]
        consumer, name = colour_input(dump, events, recipe)
        rgb, identity = NATIVE.resource(dump, name, consumer)
        if identity["sha256"] != row["hdr_sha256"]:
            raise ValueError("candidate changed before image inspection")
        pe = capture["identities"][args.frame]["exposure"]["values"][0]
        images.append((label, rgb[..., :3] * (scale / pe)))
        identities.append(dict(label=label, hdr=identity, score=str(score_path), score_sha256=digest(score_path)))
    destination = ROOT / "inspect"
    destination.mkdir(exist_ok=True)
    for stops in (0, 4):
        reference = codes(mean, stops)
        noise = np.abs((codes(halves[0], stops) - codes(halves[1], stops)) * .5).mean(-1)
        figure, axes = plt.subplots(2, len(images), figsize=(4 * len(images), 7), layout="constrained")
        for column, (label, rgb) in enumerate(images):
            encoded = codes(rgb, stops)
            axes[0, column].imshow(np.clip(encoded / 255, 0, 1))
            axes[0, column].set_title(label)
            error = np.abs(encoded - reference).mean(-1) if column else noise
            heat = axes[1, column].imshow(error, vmin=0, vmax=64, cmap="magma")
            axes[1, column].set_title("Mean absolute RGB error" if column else "Reference half difference / 2")
            for axis in axes[:, column]:
                axis.axis("off")
        figure.suptitle(f"{track} frame {args.frame}, F0 L{stops}; fixed error scale 0–64 codes, saturates above 64; black primary")
        figure.colorbar(heat, ax=axes[1, :].tolist(), location="bottom", fraction=.05, pad=.02, label="F0 codes (64+ saturated)")
        figure.savefig(destination / f"{track}-f{args.frame:03}-L{stops}.png", dpi=160)
        plt.close(figure)
    save(destination / f"{track}-f{args.frame:03}.json", dict(reference=str(archive), reference_sha256=archive_sha,
         identities=identities, script_sha256=SCRIPT_SHA,
         scope="Whole image and absolute-error heatmaps; same physical target/registered exposure, no fitted alignment or gain. Distinct from frozen original T1b reference."))
    print("whole-frame inspection sheets retained", args.frame)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("plan")
    commands.add_parser("baseline")
    commands.add_parser("cost")
    p = commands.add_parser("inspect")
    p.add_argument("--labels", nargs="+", required=True)
    p.add_argument("--frame", type=int, default=64)
    p.add_argument("--history")
    p.add_argument("--score-tag", help="Inspect a retained targeted score without replacing the full score")
    p = commands.add_parser("run")
    p.add_argument("label")
    p.add_argument("--track", choices=("q-walk", "q-disocclusion", "q-zoom", "q-sun-slow", "q-sun-fast", "q-sun-jump",
                   "s2g-camera-0.25px", "s2g-camera-0.5px", "s2g-camera-1px", "s2g-sun-0.25deg"), default="q-walk")
    p.add_argument("--quality", type=int, choices=(2, 3), default=3)
    p.add_argument("--path", choices=("vsm", "vsm-nort", "hrt"), default="vsm")
    p.add_argument("--capture", action="store_true")
    p.add_argument("--program", choices=("full", "sun"), default="full")
    p.add_argument("--aa", choices=("tsr", "none"), default="tsr")
    p.add_argument("--static-frame", type=int)
    p.add_argument("--settle-frames", type=int, default=0)
    p.add_argument("--synchronous-render", action="store_true", help="Capture diagnostic: r.OneFrameThreadLag=0 to isolate reset/render-thread races")
    p.add_argument("--temporal-readback", action="store_true", help="Capture TSR parameters on the first two frames, then retain only the requested dump tree")
    p.add_argument("--exposure-offset", type=float, default=0., help="Static diagnostic only; score unexposed HDR at the registered exposure")
    p.add_argument("--dump-root", default="Tonemap*")
    p.add_argument("--smoke", action="store_true")
    p.add_argument("--minutes", type=int, default=6)
    p.add_argument("--wait-seconds", type=int, default=0)
    p.add_argument("--package-dir", type=Path)
    for command in ("summarize", "clean", "prune", "history-check"):
        commands.add_parser(command).add_argument("label")
    p = commands.add_parser("score")
    p.add_argument("label")
    p.add_argument("--reference", type=Path, required=True)
    p.add_argument("--history")
    p.add_argument("--frames", type=int, nargs="+", help="Targeted CPU witness check; writes a separate output and preserves the full score")
    p.add_argument("--ages", type=Path)
    p.add_argument("--output-tag", type=str)
    p = commands.add_parser("history")
    p.add_argument("label")
    p.add_argument("--minutes", type=int, default=3)
    p.add_argument("--wait-seconds", type=int, default=0)
    p.add_argument("--black-background", action="store_true")
    p = commands.add_parser("reference")
    p.add_argument("label")
    p.add_argument("--track", required=True)
    p.add_argument("--frames", type=int, nargs="+", required=True)
    p.add_argument("--samples", type=int, default=1024)
    p.add_argument("--binary", type=Path, default=Path("E:/renderlab-work/m7-u0b/bin/reference-tests.exe"))
    p.add_argument("--compact-tool", type=Path, default=REPO / "scripts/m7-u0b-score.py")
    p.add_argument("--minutes", type=int, default=3)
    p.add_argument("--wait-seconds", type=int, default=0)
    p.add_argument("--timeout-seconds", type=int, default=120, help="Functional reference guard; fit inside --minutes with 15 seconds to spare")
    p = commands.add_parser("quiet")
    p.add_argument("label")
    p.add_argument("--minutes", type=int, default=8)
    p.add_argument("--wait-seconds", type=int, default=900)
    p.add_argument("--quiet-tool", type=Path, default=REPO / "scripts/m7-u0b.py")
    commands.add_parser("quiet-check").add_argument("label")
    p = commands.add_parser("black-check")
    p.add_argument("label")
    p.add_argument("--baseline", required=True)
    p.add_argument("--reference", type=Path, required=True)
    args = parser.parse_args()
    {"plan": plan, "run": run, "summarize": summarize, "clean": clean, "prune": prune, "baseline": baseline,
     "score": score, "cost": cost, "history": history, "history-check": history_check, "black-check": black_check,
     "reference": reference, "quiet": quiet, "quiet-check": quiet_check, "inspect": inspect}[args.command](args)


if __name__ == "__main__":
    main()
