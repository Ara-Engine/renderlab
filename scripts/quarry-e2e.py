"""Matched offscreen frame-loop runs, using the existing packaged UE quarry.

build/session hold the shared GPU mkdir lease for <=45 minutes, preflight each
launch, use hidden child startup, and retain raw evidence outside Git. Exit 3
means deferred: do CPU work and retry. Never stops foreign processes.
"""
import argparse
import contextlib
import csv
import ctypes
import datetime as dt
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import re
import shutil
import statistics
import subprocess
import threading
import time

REPO = Path(__file__).resolve().parent.parent
PRIVATE = Path("D:/Coding/renderlab-work/quarry-e2e")
UE = Path("D:/Coding/renderlab-work/ue-m5/Quarry")
PACKAGE = UE / "package/Windows/Engine/Binaries/Win64/UnrealGame.exe"
BASE = Path("D:/Coding/renderlab-work/m6-interactive-climb/runs/timing-c11-1.manifest.json")
TEST = "renderer::fresh_shadow_tests::quarry_frame_loop_tests::gpu_quarry_frame_loop"
FLAGS = subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0
GAMES = ("eldenring", "nightreign", "cyberpunk", "helldivers", "cs2.exe", "valorant",
         "fortniteclient", "overwatch", "arcraiders", "bf6.exe", "league of legends",
         "r5apex", "dota2", "rocketleague", "minecraft", "blackops", "cod.exe",
         "witcher", "baldur", "starfield", "roblox", "destiny2", "escapefromtarkov",
         "factorio", "win64-shipping", "shipping.exe")


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def utc():
    return dt.datetime.now(dt.timezone.utc).isoformat()


def save(path, data):
    path.write_text(json.dumps(data, indent=2, allow_nan=False) + "\n", encoding="utf-8")


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def powershell(command):
    proc = subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-Command", command],
                          capture_output=True, text=True, creationflags=FLAGS, timeout=30)
    if proc.returncode:
        raise RuntimeError(proc.stderr)
    return proc.stdout.strip()


def available_gib():
    """Read the English PDH gauge \\Memory\\Available MBytes, including standby."""
    class Value(ctypes.Structure):
        _fields_ = [("status", ctypes.c_uint32), ("number", ctypes.c_double)]
    pdh = ctypes.WinDLL("pdh")
    query, counter, value = ctypes.c_void_p(), ctypes.c_void_p(), Value()
    def check(status):
        if status:
            raise OSError(f"PDH available-memory counter failed: {status:#x}")
    check(pdh.PdhOpenQueryW(None, 0, ctypes.byref(query)))
    try:
        check(pdh.PdhAddEnglishCounterW(query, ctypes.c_wchar_p(r"\Memory\Available MBytes"), 0, ctypes.byref(counter)))
        check(pdh.PdhCollectQueryData(query))
        check(pdh.PdhGetFormattedCounterValue(counter, 0x200, None, ctypes.byref(value)))
        if value.status not in (0, 1) or not math.isfinite(value.number):
            raise OSError("invalid PDH available-memory sample")
        return value.number / 1024
    finally:
        pdh.PdhCloseQuery(query)


def state():
    value = json.loads(powershell("Get-CimInstance Win32_Process | Select-Object Name,ProcessId,ParentProcessId,ExecutablePath | ConvertTo-Json -Compress"))
    owned = {os.getpid()}
    for _ in range(8):
        owned.update(p["ProcessId"] for p in value if p["ParentProcessId"] in owned)
    games, renderers, builds = [], [], []
    for p in value:
        if p["ProcessId"] in owned:
            continue
        name = p["Name"].lower()
        path = (p.get("ExecutablePath") or "").lower().replace("/", "\\")
        launcher = name.startswith(("steam", "riotclient", "epicgameslauncher")) or "\\riot client\\" in path
        if not launcher and (any(g in name for g in GAMES) or any(d in path for d in
                ("\\steamapps\\common\\", "\\epic games\\", "\\riot games\\", "\\xboxgames\\"))):
            # Unreal's installed engine is a renderer, not a foreground game.
            if not name.startswith(("unreal", "shadercompile")) and "wallpaper" not in path:
                games.append(p)
        if (name.startswith(("renderlab", "unreal", "shadercompile", "fresh-tests", "primary-tests", "climb-tests",
                             "e2e-tests", "m6-shadows", "screen-", "sel2shadow", "quarry-resets", "virtual-stream"))
                or ("\\renderlab-work\\" in path and name.endswith(".exe") and name not in
                    ("cargo.exe", "rustc.exe", "clippy-driver.exe", "python.exe", "py.exe", "powershell.exe", "pwsh.exe", "rust-analyzer.exe"))):
            renderers.append(p)
        if name in ("cargo.exe", "rustc.exe", "clippy-driver.exe", "link.exe", "unrealbuildtool.exe"):
            builds.append(p)
    memory = json.loads(powershell("Get-CimInstance Win32_OperatingSystem | Select-Object FreePhysicalMemory,TotalVisibleMemorySize | ConvertTo-Json -Compress"))
    gpu = subprocess.run(["nvidia-smi", "--query-gpu=name,driver_version,utilization.gpu,memory.used,clocks.gr,temperature.gpu,power.draw",
                          "--format=csv,noheader,nounits"], capture_output=True, text=True, creationflags=FLAGS, timeout=10)
    if gpu.returncode:
        raise RuntimeError("GPU inventory failed: " + gpu.stderr)
    return dict(utc=utc(), games=games, foreign_renderers=renderers, builds=builds,
                free_gib=memory["FreePhysicalMemory"] / 1024**2, available_gib=available_gib(),
                available_counter=r"\Memory\Available MBytes", gpu=gpu.stdout.strip())


def preflight(path, memory_pressure=False):
    record = state()
    record.update(memory_pressure=memory_pressure, launch_floor_gib=6 if memory_pressure else 8,
                  launch_memory_metric="available_gib" if memory_pressure else "free_gib")
    save(path, record)
    if record["games"] or record["foreign_renderers"] or record[record["launch_memory_metric"]] < record["launch_floor_gib"]:
        print("DEFERRED", json.dumps(record), flush=True)
        raise SystemExit(3)
    return record


def await_memory(out, slot, memory_pressure=False):
    """Bounded retry for RAM-only deferrals; never launches during a conflict."""
    for attempt in range(13):
        _, remaining = slot.read_lease()
        if remaining < 190:
            raise RuntimeError("release lease before another launch")
        path = out / f"preflight-{attempt:02d}.json"
        try:
            record = preflight(path, memory_pressure)
        except SystemExit:
            record = json.loads(path.read_text())
            if record["games"] or record["foreign_renderers"] or attempt == 12:
                raise
            time.sleep(10)
        else:
            save(out / "preflight.json", record)
            return record


@contextlib.contextmanager
def lease(out):
    slot = load("ue-quarry-slot")
    history_path = out / "lease-history.json"
    history = json.loads(history_path.read_text()) if history_path.exists() else []
    previous = out / "lease-released.json"
    if previous.exists():
        record = json.loads(previous.read_text())
        if not any(r["acquired"]["token"] == record["acquired"]["token"] for r in history):
            history.append(record)
            save(history_path, history)
    if slot.LOCK.exists():
        owner = (slot.LOCK / "owner.json").read_text()
        save(out / "deferred-lock.json", dict(utc=utc(), owner=owner))
        print("DEFERRED: GPU slot held", owner, flush=True)
        raise SystemExit(3)
    if slot.acquire(45):
        raise SystemExit(3)
    owner, _ = slot.read_lease()
    save(out / "lease-acquired.json", owner)
    try:
        yield slot
    finally:
        current = json.loads((slot.LOCK / "owner.json").read_text())
        if current["token"] != owner["token"]:
            raise RuntimeError("lease changed; refusing release")
        slot.release()
        record = dict(acquired=owner, released_utc=utc())
        save(out / "lease-released.json", record)
        history.append(record)
        save(history_path, history)


@contextlib.contextmanager
def monitor(out, memory_abort_gib=None):
    samples, stopped = [], threading.Event()
    failure, memory_samples = [], []

    def stop_owned_gpu():
        children = json.loads(powershell("Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name | ConvertTo-Json -Compress"))
        for p in children:
            if p["ParentProcessId"] == os.getpid() and p["Name"].lower() in ("unrealgame.exe", "e2e-tests.exe"):
                subprocess.run(["taskkill", "/PID", str(p["ProcessId"]), "/T", "/F"], capture_output=True, creationflags=FLAGS)

    def watch_memory():
        while not stopped.wait(.25):
            try:
                value = available_gib()
                memory_samples.append(dict(utc=utc(), available_gib=value))
                if value < memory_abort_gib:
                    failure.append(dict(reason="available memory below abort floor", available_gib=value))
                    stop_owned_gpu()
                    return
            except Exception as error:
                failure.append(dict(memory_monitor_error=str(error)))
                stop_owned_gpu()
                return

    def watch():
        while not stopped.wait(1):
            try:
                snapshot = state()
                samples.append(snapshot)
                if snapshot["games"] or snapshot["foreign_renderers"]:
                    failure.append(snapshot)
                    # Only processes parented to this launcher are owned.
                    stop_owned_gpu()
                    return
            except Exception as error:
                failure.append(dict(error=str(error)))
                return

    thread = threading.Thread(target=watch)
    thread.start()
    memory_thread = threading.Thread(target=watch_memory) if memory_abort_gib is not None else None
    if memory_thread is not None:
        memory_thread.start()
    try:
        yield
    finally:
        stopped.set()
        thread.join()
        if memory_thread is not None:
            memory_thread.join()
        save(out / "contention.json", dict(samples=samples, failures=failure, memory_samples=memory_samples,
                                          memory_abort_gib=memory_abort_gib, memory_counter=r"\Memory\Available MBytes"))
    if failure:
        raise RuntimeError("contention monitor failed; retained evidence is not accepted")


def hidden_run(command, log, **kwargs):
    startup = subprocess.STARTUPINFO()
    startup.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = 0
    with log.open("w", encoding="utf-8") as stream:
        return subprocess.run(command, cwd=REPO, stdout=stream, stderr=subprocess.STDOUT,
                              creationflags=FLAGS, startupinfo=startup, **kwargs)


def build(out):
    with lease(out):
        preflight(out / "preflight.json")
        command = ["cargo", "test", "-p", "renderlab", "--lib", "--release", "--no-run", "-j", "2",
                   "--target-dir", str(PRIVATE / "target"), "--message-format=json"]
        start = time.perf_counter()
        proc = hidden_run(command, out / "build.jsonl", timeout=1200)
        record = dict(command=command, seconds=time.perf_counter() - start, exit_code=proc.returncode,
                      head=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip())
        save(out / "build.json", record)
        if proc.returncode:
            raise RuntimeError("build failed; see " + str(out / "build.jsonl"))
        artifacts = [json.loads(line) for line in (out / "build.jsonl").read_text().splitlines() if line.startswith("{")]
        binary = next(a["executable"] for a in reversed(artifacts) if a.get("reason") == "compiler-artifact"
                      and a.get("executable") and a["target"]["name"] == "renderlab")
        shutil.copy2(binary, out / "e2e-tests.exe")
        record["binary_sha256"] = sha(out / "e2e-tests.exe")
        save(out / "build.json", record)
        (out / "source.diff").write_bytes(subprocess.check_output(["git", "diff", "HEAD"], cwd=REPO))
        save(out / "source-hashes.json", {str(p.relative_to(REPO)): sha(p) for p in REPO.glob("src/*.rs")})
        print("BUILD", record["seconds"], record["binary_sha256"], flush=True)


def motion_files(out, sequence):
    files = []
    for pose in range(2):
        file = out / f"{sequence}-{pose}.txt"
        if sequence == "camera":
            command = f'ke CameraActor K2_SetActorLocation "(X=-620,Y={520+pose},Z=340)" false () false'
        else:
            pitch = math.degrees(math.asin(-.8 / math.sqrt(.89)))
            yaw = math.degrees(math.atan2(-.4, .3)) + pose
            command = f'ke DirectionalLight K2_SetActorRotation "(Pitch={pitch:.9f},Yaw={yaw:.9f},Roll=0)" false'
        file.write_text(command + "\n", encoding="utf-8")
        files.append(file)
    return files


def ue_events(out, sequence, verify):
    events = [(64, "ProfileGPU"), (90, "D3D12.DumpRayTracingGeometries all")]
    if sequence != "static":
        files = motion_files(out, sequence)
        # ULocalPlayer::ExecMacro prefixes ../../Binaries even for absolute paths.
        # Resolve relative to the packaged Engine/Binaries directory explicitly.
        names = [Path(os.path.relpath(file, PACKAGE.parent.parent)).as_posix() for file in files]
        events += [(i, "exec " + names[i % 2]) for i in range(160)]
        if sequence == "sun":
            events += [(92, "getall SceneComponent RelativeRotation"), (93, "getall SceneComponent RelativeRotation")]
    if verify:
        events += [(94, "getall PlayerCameraManager CameraCachePrivate"),
                   (95, "getall PlayerCameraManager CameraCachePrivate")]
    return sorted(events, key=lambda e: e[0])


def csv_rows(path):
    with path.open(encoding="utf-8-sig", newline="") as stream:
        rows = list(csv.reader(stream))
    header = next(row for row in reversed(rows) if row and row[0] == "EVENTS")
    result = []
    for row in rows[1:]:
        if not row or row[0].startswith("[") or row[0] == "EVENTS" or len(row) > len(header):
            continue
        try:
            result.append({key: float(value) for key, value in zip(header[1:], (row + ["0"] * len(header))[1:])})
        except ValueError:
            continue
    return result


def verify_motion(path, sequence):
    rows = csv_rows(path)[96:160]
    if len(rows) != 64:
        raise ValueError("expected fixed profiler block")
    ys = [r["View/PosY"] for r in rows]
    if sequence == "camera":
        if not (set(ys) == {520., 521.} and all(a != b for a, b in zip(ys, ys[1:]))):
            raise ValueError("packaged camera did not alternate every frame: " + repr(ys))
    return dict(camera_y=ys, camera_x=sorted({r["View/PosX"] for r in rows}),
                camera_z=sorted({r["View/PosZ"] for r in rows}), sequence=sequence)


def verify_sun(log):
    text = log.read_text(encoding="utf-8-sig")
    rotations = re.findall(r"DirectionalLight_0\.LightComponent0\.RelativeRotation = \(Pitch=([-\d.]+),Yaw=([-\d.]+),Roll=([-\d.]+)\)", text)
    yaws = sorted({round(float(r[1]), 4) for r in rotations})
    if len(yaws) != 2 or abs(yaws[1] - yaws[0] - 1) > .0002:
        raise ValueError("packaged sun rotation did not alternate: " + repr(rotations))
    if "call failed" in text or "Bad or missing property" in text:
        raise ValueError("packaged motion call failed")
    return dict(rotations=rotations, yaw_degrees=yaws)


def session(out, binary, sequences, repeats, verify, resume=False, memory_pressure=False):
    base = json.loads(BASE.read_text())
    runner, profiler = load("ue-quarry-run"), load("ue-quarry-profile")
    identity = dict(start_utc=utc(), binary=str(binary), binary_sha256=sha(binary),
                    package=str(PACKAGE), package_sha256=sha(PACKAGE),
                    input_hashes={k: sha(Path(base[k])) for k in ("asset", "limits", "cameras", "lighting", "environment")},
                    scripts={p.name: sha(p) for p in Path(__file__).parent.glob("*quarry*.py")})
    if resume:
        original = json.loads((out / "identity.json").read_text())
        for key in ("binary_sha256", "package_sha256", "input_hashes"):
            if original[key] != identity[key]:
                raise ValueError("resume changed immutable input: " + key)
        save(out / f"resume-{time.time_ns()}.json", identity)
    else:
        save(out / "identity.json", identity)
    schedule = []
    for sequence in sequences:
        for block in range(repeats):
            modes = ("renderlab", "vsm", "hrt") if block % 2 == 0 else ("hrt", "vsm", "renderlab")
            schedule += [dict(sequence=sequence, block=block, mode=mode, pose=(block % 2 if sequence == "static" else 0)) for mode in modes]
    if resume:
        if json.loads((out / "schedule.json").read_text()) != schedule:
            raise ValueError("resume changed the registered schedule")
    else:
        save(out / "schedule.json", schedule)
    if memory_pressure and (not resume or len(schedule) != 27):
        raise ValueError("the available-memory exception requires the original 27-run schedule")
    with lease(out) as slot:
        for index, entry in enumerate(schedule):
            _, remaining = slot.read_lease()
            if remaining < 190:
                raise RuntimeError("less than 190 seconds remain; release before another bounded launch")
            label = f'e2e-{out.name}-{index:02d}-{entry["mode"]}-{entry["sequence"]}'
            destination = out / f"{index:02d}"
            if resume:
                if (destination / "run.json").exists():
                    continue
                label += f"-r{len(list(out.glob('resume-*.json')))}"
                if destination.exists():
                    retained = out / f"deferred-{index:02d}-{time.time_ns()}"
                    if destination.resolve().parent != out.resolve() or retained.resolve().parent != out.resolve():
                        raise ValueError("evidence rename escaped its session")
                    destination.rename(retained)
            if memory_pressure and (index not in range(22, 27) or entry["sequence"] != "sun"):
                raise ValueError("the 6 GiB exception is authorized only for the last five resumed sun runs")
            destination.mkdir()
            await_memory(destination, slot, memory_pressure)
            start = time.perf_counter()
            if entry["mode"] == "renderlab":
                manifest = dict(base, output=str(destination), sequence=entry["sequence"], pose=entry["pose"],
                                warmup=512, frames=2048, validate=verify,
                                accepted_interactive="D:/Coding/renderlab-work/m6-interactive-climb/runs/accept-c11")
                save(destination / "manifest.json", manifest)
                env = os.environ.copy()
                implicit = PRIVATE / "empty-implicit"
                implicit.mkdir(exist_ok=True)
                env.update(RENDERLAB_PRIMARY_MANIFEST=str(destination / "manifest.json"),
                           RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
                           VK_IMPLICIT_LAYER_PATH=str(implicit), VK_LAYER_PATH="C:/VulkanSDK/1.4.363.0/Bin")
                env.pop("VK_INSTANCE_LAYERS", None)
                if verify:
                    env["VK_INSTANCE_LAYERS"] = "VK_LAYER_KHRONOS_validation"
                    env.pop("VK_LOADER_LAYERS_DISABLE", None)
                else:
                    env["VK_LOADER_LAYERS_DISABLE"] = "VK_LAYER_KHRONOS_validation"
                command = [str(binary), TEST, "--ignored", "--exact", "--nocapture", "--test-threads=1"]
                with monitor(destination, 3 if memory_pressure else None):
                    proc = hidden_run(command, destination / "stdout.log", env=env, timeout=120)
                if proc.returncode:
                    raise RuntimeError("Renderlab run failed: " + str(destination))
            else:
                commands = ["r.ProfileGPU.ShowUI 0", "getall CameraComponent FieldOfView", "getall CameraComponent PostProcessSettings",
                            "getall DirectionalLightComponent Intensity", "r.RayTracing.Shadows", "r.Shadow.Virtual.Cache"]
                if entry["mode"] == "hrt":
                    commands += ["set DirectionalLightComponent LightSourceAngle 4.583662361",
                                 "set DirectionalLightComponent ShadowSourceAngleFactor 1",
                                 "r.Shadow.CacheWPOPrimitives 1", "r.Shadow.CacheWPOPrimitives 0"]
                with monitor(destination, 3 if memory_pressure else None):
                    runner.run("profile", label, "hardware-rt-denoised" if entry["mode"] == "hrt" else "vsm-smrt", 180,
                               entry["pose"], PACKAGE, commands, ue_events(destination, entry["sequence"], verify),
                               minimum_available_gib=6 if memory_pressure else None)
                launch = UE / "launches" / label
                csv_path = next(launch.glob("*.csv"))
                summary = profiler.summarize(csv_path)
                save(destination / "ue-summary.json", summary)
                motion = verify_motion(csv_path, entry["sequence"])
                if entry["sequence"] == "sun":
                    motion["sun"] = verify_sun(launch / "ue.log")
                save(destination / "motion-check.json", motion)
                save(destination / "ue-evidence.json", dict(launch=str(launch), csv=str(csv_path)))
            save(destination / "run.json", dict(entry, label=label, memory_pressure=memory_pressure,
                                               memory_launch_floor_gib=6 if memory_pressure else 8,
                                               memory_abort_floor_gib=3 if memory_pressure else None, start_seconds=start,
                                               elapsed_seconds=time.perf_counter() - start, end_utc=utc()))
            print(index, entry, "seconds", round(time.perf_counter() - start, 2), flush=True)
    save(out / "finished.json", dict(end_utc=utc()))


def stats(values):
    values = sorted(values)
    n = len(values)
    def quantile(q):
        x = (n - 1) * q
        low = math.floor(x)
        return values[low] + (values[math.ceil(x)] - values[low]) * (x - low)
    return dict(n=n, median=statistics.median(values), mean=statistics.mean(values), minimum=values[0],
                p05=quantile(.05), p95=quantile(.95), maximum=values[-1])


def summarize(out):
    records = []
    for directory in sorted(out.glob("[0-9][0-9]")):
        if not (directory / "run.json").exists():
            continue
        record = json.loads((directory / "run.json").read_text())
        if record["mode"] == "renderlab":
            frames = json.loads((directory / "frames.json").read_text())
            record["statistics"] = {key: stats([r[key] for r in frames["rows"] if key != "frame_to_frame_ms" or r["index"] > 0]) for key in
                ("gpu_ms", "cpu_frame_ms", "cpu_calls_ms", "submit_ms", "update_ms", "draw_preparation_ms", "collection_ms",
                 "preparation_ms", "shadow_preparation_ms", "upload_ms", "encode_ms", "frame_to_frame_ms", "completion_latency_ms")}
            record["completed_ms_per_frame"] = frames["completed_ms_per_frame"]
            record["statistics"]["cpu_critical_proxy_ms"] = stats([
                r["cpu_frame_ms"] + r["collection_ms"] for r in frames["rows"]])
            record["startup_to_first_completed_image_ms"] = frames["startup_to_first_completed_image_ms"]
        else:
            summary = json.loads((directory / "ue-summary.json").read_text())
            record["statistics"] = {key: summary["statistics"][key] for key in
                ("GPUTime", "FrameTime", "GameThreadTime", "RenderThreadTime", "GameThreadTime_CriticalPath", "RenderThreadTime_CriticalPath", "RHIThreadTime")}
            native = csv_rows(Path(summary["csv"]))[96:160]
            record["statistics"]["cpu_critical_proxy_ms"] = stats([max(r[k] for k in
                ("GameThreadTime_CriticalPath", "RenderThreadTime_CriticalPath", "RHIThreadTime")) for r in native])
            record["completed_ms_per_frame"] = statistics.mean(r["FrameTime"] for r in native)
        records.append(record)
    ratios = []
    for ue in (r for r in records if r["mode"] != "renderlab"):
        rl = next(r for r in records if r["mode"] == "renderlab" and r["sequence"] == ue["sequence"] and r["block"] == ue["block"])
        ratios.append(dict(mode=ue["mode"], sequence=ue["sequence"], block=ue["block"],
            gpu=ue["statistics"]["GPUTime"]["median"] / rl["statistics"]["gpu_ms"]["median"],
            cpu_critical_proxy=ue["statistics"]["cpu_critical_proxy_ms"]["median"] / rl["statistics"]["cpu_critical_proxy_ms"]["median"],
            cpu_gt=ue["statistics"]["GameThreadTime_CriticalPath"]["median"] / rl["statistics"]["cpu_critical_proxy_ms"]["median"],
            cpu_rt=ue["statistics"]["RenderThreadTime_CriticalPath"]["median"] / rl["statistics"]["cpu_critical_proxy_ms"]["median"],
            frame_median=ue["statistics"]["FrameTime"]["median"] / rl["statistics"]["frame_to_frame_ms"]["median"],
            throughput=ue["completed_ms_per_frame"] / rl["completed_ms_per_frame"]))
    save(out / "summary.json", dict(records=records, ratios=ratios,
        analysis_script_sha256=sha(Path(__file__)),
        ratio_scope="UE/Renderlab, larger favors Renderlab; same-block process medians, not simultaneous paired frames; CPU native critical fields include waits, serial Renderlab admission includes backpressure; throughput uses measured complete block means, not max(CPU,GPU)"))
    print(json.dumps(ratios, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("build", "session", "summarize", "preflight"))
    parser.add_argument("label")
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--sequences", nargs="+", choices=("static", "camera", "sun"), default=["static", "camera", "sun"])
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("--resume", action="store_true", help="keep completed runs and resume the identical schedule/inputs")
    parser.add_argument("--memory-pressure", action="store_true", help="user-authorized final five sun runs only: available RAM >=6 GiB, abort <3 GiB")
    args = parser.parse_args()
    if not args.label or any(c not in "abcdefghijklmnopqrstuvwxyz0123456789-" for c in args.label):
        parser.error("label must be lowercase alphanumeric/hyphens")
    out = PRIVATE / args.label
    if args.action == "summarize":
        summarize(out)
        return
    out.mkdir(parents=True, exist_ok=args.resume and args.action == "session")
    if args.action == "build":
        build(out)
    elif args.action == "session":
        if args.binary is None or args.repeats < 1:
            parser.error("session requires --binary and positive repeats")
        session(out, args.binary.resolve(), args.sequences, args.repeats, args.verify, args.resume, args.memory_pressure)
    else:
        preflight(out / "preflight.json")


if __name__ == "__main__":
    main()
