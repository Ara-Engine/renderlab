"""M7-U0a: Unreal 5.7.4 Lumen/TSR configuration study on the matched quarry.

Every Unreal process here is hidden and offscreen. Each subcommand that touches the GPU or runs a
heavy Unreal build takes the shared lease itself (lock directory + owner.json, with reservation and
release lines in E:/renderlab-work/gpu-schedule.md), and releases it before returning. Launches
start only with >= 4 GiB available memory and the owned child is stopped below 2 GiB, when a game
starts, or when a foreign renderer appears.

  cook      recook the isolated project copy with mesh distance fields + path-tracer shaders,
            then stage/pak it to a new package (a new, recorded content identity)
  study     run the configuration matrix on that package (timing, readback, images, motion)
  fixture   recapture the M5 directional-black fixture on D3D12 SM6 for the glossy control (U0c)
  summarize CPU-only: per-config table from retained evidence

Raw evidence goes to E:/renderlab-work/m7-u0; nothing private enters Git.
"""
import argparse
import contextlib
import datetime as dt
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import threading
import time
import uuid

REPO = Path(__file__).resolve().parent.parent
OUT = Path("E:/renderlab-work/m7-u0")
PROJECT = OUT / "ue-project/M5Calibration.uproject"
PACKAGE_DIR = OUT / "ue-package"
PACKAGE = PACKAGE_DIR / "Windows/Engine/Binaries/Win64/UnrealGame.exe"
EDITOR = Path("C:/Program Files/Epic Games/UE_5.7/Engine/Binaries/Win64/UnrealEditor-Cmd.exe")
UAT = Path("C:/Program Files/Epic Games/UE_5.7/Engine/Build/BatchFiles/RunUAT.bat")
LOCK = Path("D:/Coding/renderlab-work/gpu-slot.lock")
SCHEDULE = Path("E:/renderlab-work/gpu-schedule.md")
OWNER = "m7-u0"
FLAGS = subprocess.CREATE_NO_WINDOW
START_GIB, ABORT_GIB = 4.0, 2.0


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), REPO / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


E2E = load("quarry-e2e")
PROFILE = load("ue-quarry-profile")


def utc():
    return dt.datetime.now(dt.timezone.utc)


def save(path, data):
    path.write_text(json.dumps(data, indent=2, allow_nan=False) + "\n", encoding="utf-8")


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def schedule_line(text):
    with SCHEDULE.open("a", encoding="utf-8") as stream:
        stream.write("\n" + text + "\n")


@contextlib.contextmanager
def lease(label, minutes):
    """Shared GPU lease for one bounded job. Never waits on, or removes, another owner's lock."""
    if not 1 <= minutes <= 45:
        raise ValueError("leases are 1-45 minutes")
    try:
        LOCK.mkdir()
    except FileExistsError:
        print("DEFERRED: GPU slot held:", (LOCK / "owner.json").read_text() if (LOCK / "owner.json").exists() else "(publishing)")
        raise SystemExit(3)
    start = utc()
    owner = dict(owner=OWNER, utc=start.isoformat(), expected_minutes=float(minutes), launcher_pid=os.getpid(),
                 token=uuid.uuid4().hex, label=label)
    (LOCK / "owner.json").write_text(json.dumps(owner, indent=2) + "\n")
    end = start + dt.timedelta(minutes=minutes)
    schedule_line(f"[{OWNER} reservation] owner={OWNER} end={end.isoformat()} label={label}")
    history = OUT / "lease-history.json"
    records = json.loads(history.read_text()) if history.exists() else []
    try:
        yield owner
    finally:
        live = [p for p in E2E.state()["foreign_renderers"] if p["Name"].lower().startswith(("unreal", "shadercompile"))]
        if live:
            print("WARNING: Unreal processes still alive at release:", live)
        current = json.loads((LOCK / "owner.json").read_text())
        if current["token"] != owner["token"]:
            raise RuntimeError("lease changed under us; refusing release")
        (LOCK / "owner.json").unlink()
        LOCK.rmdir()
        released = utc()
        schedule_line(f"{OWNER} {label} window released {released.isoformat()} (no owned GPU/build job remains).")
        records.append(dict(label=label, acquired=owner["utc"], released=released.isoformat(),
                            seconds=(released - start).total_seconds()))
        save(history, records)


def remaining_seconds(owner):
    return owner["expected_minutes"] * 60 - (utc() - dt.datetime.fromisoformat(owner["utc"])).total_seconds()


def engine_tool(p):
    """Engine helpers that render nothing (crash monitor, trace server, UAT/dotnet, UnrealPak) are not
    games or renderers. Editors, games and shader workers stay conflicts unless they are our children."""
    return p["Name"].lower().startswith(("crashreport", "unrealtraceserver", "automationtool", "dotnet", "unrealpak",
                                         "epicwebhelper", "renderlab-cook"))  # the asset cooker is CPU-only


def preflight(path, wait_seconds=180):
    """Defer at once on a game; wait (bounded) for another worker's renderer or test binary to finish."""
    deadline = time.monotonic() + wait_seconds
    while True:
        record = E2E.state()
        games = [p for p in record["games"] if not engine_tool(p)]
        foreign = [p for p in record["foreign_renderers"] if not engine_tool(p)]
        if games or not foreign or time.monotonic() > deadline:
            break
        time.sleep(5)
    save(path, record)
    if games or foreign or record["available_gib"] < START_GIB:
        print("DEFERRED", json.dumps({k: record[k] for k in ("games", "foreign_renderers", "available_gib")}), flush=True)
        raise SystemExit(3)
    return record


def guarded(command, out, timeout, cwd=None, env=None):
    """Run one hidden owned process; stop it on a game, foreign renderer or low memory."""
    out.mkdir(parents=True, exist_ok=True)
    preflight(out / "preflight.json")
    startup = subprocess.STARTUPINFO()
    startup.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = 0
    stop, samples, failures = threading.Event(), [], []
    record = dict(start_utc=utc().isoformat(), command=[str(c) for c in command], timeout_seconds=timeout)
    begin = time.perf_counter()
    with (out / "stdout.log").open("w", encoding="utf-8") as log:
        proc = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, creationflags=FLAGS,
                                startupinfo=startup, cwd=cwd, env=env)

        def kill(reason):
            failures.append(reason)
            subprocess.run(["taskkill", "/PID", str(proc.pid), "/T", "/F"], capture_output=True, creationflags=FLAGS)

        def watch_memory():
            while not stop.wait(0.25):
                value = E2E.available_gib()
                samples.append(round(value, 3))
                if value < ABORT_GIB and proc.poll() is None:
                    kill(dict(reason="available memory below 2 GiB", available_gib=value))
                    return

        def watch_conflicts():
            while not stop.wait(2):
                snapshot = E2E.state()
                ours = {proc.pid}
                foreign = [p for p in snapshot["foreign_renderers"] if p["ProcessId"] not in ours
                           and not engine_tool(p)]
                games = [p for p in snapshot["games"] if not engine_tool(p)]
                if (games or foreign) and proc.poll() is None:
                    kill(dict(reason="game or foreign renderer", games=games, foreign=foreign))
                    return

        threads = [threading.Thread(target=watch_memory), threading.Thread(target=watch_conflicts)]
        for thread in threads:
            thread.start()
        try:
            proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            kill("timeout")
            proc.wait()
        finally:
            stop.set()
            for thread in threads:
                thread.join()
    record.update(end_utc=utc().isoformat(), elapsed_seconds=time.perf_counter() - begin, exit_code=proc.returncode,
                  failures=failures, minimum_available_gib=min(samples) if samples else None,
                  scope="whole process wall; not GPU frame time")
    save(out / "process.json", record)
    if failures or proc.returncode:
        raise RuntimeError(f"owned process failed or was stopped: {out} {failures} exit={proc.returncode}")
    return record


COMMON_INI = ["-ini:Engine:[/Script/Engine.RendererSettings]:r.RayTracing=True",
              "-ini:Engine:[/Script/Engine.RendererSettings]:r.SkinCache.CompileShaders=True",
              "-ini:Engine:[/Script/Engine.RendererSettings]:r.GenerateMeshDistanceFields=True",
              "-ini:Engine:[/Script/Engine.RendererSettings]:r.PathTracing=True",
              "-ini:Engine:[SystemSettings]:r.ShadowQuality=5",
              "-ini:Engine:[SystemSettings]:r.Shadow.Virtual.Enable=1",
              "-ini:Engine:[SystemSettings]:r.MaxAnisotropy=16"]
UE_FLAGS = ["-unattended", "-nop4", "-nosound", "-ddc=RenderlabLocalDDC", "-NoSplash", "-NoLoadingScreen",
            "-NoShaderWorkerCrashDialog", "-NoCrashDialog", "-corelimit=8", "-stdout", "-FullStdOutLogOutput"]


def cook(args):
    out = OUT / "cook" / args.label
    if out.exists():
        raise SystemExit("label exists: " + str(out))
    with lease("cook-" + args.label, 45) as owner:
        cooked = guarded([EDITOR, PROJECT, *UE_FLAGS, f"-abslog={out / 'cook' / 'ue.log'}", *COMMON_INI, "-NullRHI",
                          "-run=cook", "-TargetPlatform=Windows",
                          "-Map=/Game/M6Quarry/View0+/Game/M6Quarry/View1+/Game/M6Quarry/View2",
                          "-unversioned", "-skipeditorcontent", "-cookcultures=en", "-NoZenAutoLaunch"],
                         out / "cook", timeout=max(60, int(remaining_seconds(owner)) - 600))
        packaged = guarded([os.environ.get("COMSPEC", "C:/Windows/System32/cmd.exe"), "/d", "/c", UAT, "BuildCookRun",
                            f"-project={PROJECT}", "-nop4", "-skipbuild", "-skipcook", "-stage", "-pak", "-archive",
                            f"-archivedirectory={PACKAGE_DIR}", "-clientconfig=Development", "-targetplatform=Win64",
                            "-unattended", "-utf8output", "-nocompile", "-nocompileuat", "-installed"],
                           out / "package", timeout=max(60, int(remaining_seconds(owner)) - 30))
    identity = dict(package_exe_sha256=sha(PACKAGE), cook_seconds=cooked["elapsed_seconds"],
                    package_seconds=packaged["elapsed_seconds"],
                    paks={str(p.relative_to(PACKAGE_DIR)): sha(p) for p in PACKAGE_DIR.rglob("*.pak")},
                    utocs={str(p.relative_to(PACKAGE_DIR)): sha(p) for p in PACKAGE_DIR.rglob("*.utoc")},
                    config_sha256=sha(PROJECT.parent / "Config/DefaultEngine.ini"))
    save(out / "identity.json", identity)
    print(json.dumps(identity, indent=2))


# Baseline commands: the matched e2e VSM static workload (docs/quarry-end-to-end-comparison.md), except that
# pre-exposure follows the manual -14 EV exposure (override 0). The e2e override 1 keeps scene colour unexposed,
# which makes Lumen's MaxRayIntensity firefly clamps (10-40, meant for pre-exposed values) clip nearly all
# traced light: study-a measured near-black Lumen images that way.
BASE_COMMANDS = [
    "r.ProfileGPU.ShowUI 0", "r.VSync 0", "t.MaxFPS 0", "t.IdleWhenNotForeground 0", "r.SetNearClipPlane 0.5",
    "r.ShadowQuality 5", "r.Shadow.Virtual.Enable 1", "r.Shadow.Virtual.SMRT.RayCountDirectional 8",
    "r.Shadow.Virtual.SMRT.SamplesPerRayDirectional 8", "r.Shadow.Virtual.SMRT.AdaptiveRayCount 1",
    "r.RayTracing.Shadows 0", "r.RayTracing.Shadows.SamplesPerPixel 1", "r.Shadow.Denoiser 1",
    "r.Shadow.Denoiser.TemporalAccumulation 1", "r.ScreenPercentage 100", "r.MaxAnisotropy 16",
    "r.EyeAdaptation.PreExposureOverride 0", "r.RayTracing.Nanite.Mode 1", "r.Nanite.MaxPixelsPerEdge 0.25",
    "r.Nanite.Streaming.StreamingPoolSize 1024", "r.RayTracing.Nanite.Streaming 1",
    "r.RayTracing.Nanite.Streaming.LODBias 0", "r.RayTracing.Nanite.Streaming.Offscreen.LODBias 0",
    "r.RayTracing.Nanite.Streaming.Offscreen.MinCutError 0", "ShowFlag.EyeAdaptation 1",
    "r.EyeAdaptationQuality 2", "r.EyeAdaptation.MethodOverride 3"]
HRT_SHADOWS = ["r.RayTracing.Shadows 1", "set DirectionalLightComponent LightSourceAngle 4.583662361",
               "set DirectionalLightComponent ShadowSourceAngleFactor 1", "r.Shadow.CacheWPOPrimitives 1",
               "r.Shadow.CacheWPOPrimitives 0"]  # the matched e2e HRT mode, unchanged
QUALITY = {2: "High", 3: "Epic", 4: "Cinematic"}


def spec(lumen=None, quality=3, mode=0, shadows="vsm", aa="tsr", reflections="lumen", mesh_sdfs=None, screen_traces=True,
         ini=(), workloads=("static", "camera", "sun")):
    """One Unreal configuration. lumen: None, "sw" or "hw"; mode: r.Lumen.HardwareRayTracing.LightingMode."""
    commands, required = [], [("r.EyeAdaptation.PreExposureOverride", "0")]
    if lumen:
        # Scalability groups first, so explicit overrides after them win; the readback proves the result.
        commands += [f"sg.GlobalIlluminationQuality {quality}", f"sg.ReflectionQuality {quality}"]
        required += [("sg.GlobalIlluminationQuality", str(quality)), ("sg.ReflectionQuality", str(quality))]
    if aa == "tsr":
        commands += ["sg.AntiAliasingQuality 3", "r.AntiAliasingMethod 4", "r.TSR.History.ScreenPercentage 200",
                     "r.ScreenPercentage 100"]
        required += [("r.AntiAliasingMethod", "4"), ("r.TSR.History.ScreenPercentage", "200"), ("r.ScreenPercentage", "100")]
    else:
        commands += ["r.AntiAliasingMethod 0"]
        required += [("r.AntiAliasingMethod", "0")]
    gi = "1" if lumen else "0"
    refl = "1" if lumen and reflections == "lumen" else "0"
    commands += [f"r.DynamicGlobalIlluminationMethod {gi}", f"r.ReflectionMethod {refl}"]
    required += [("r.DynamicGlobalIlluminationMethod", gi), ("r.ReflectionMethod", refl)]
    if lumen:
        hw = "1" if lumen == "hw" else "0"
        commands += [f"r.Lumen.HardwareRayTracing {hw}", f"r.Lumen.HardwareRayTracing.LightingMode {mode}"]
        required += [("r.Lumen.HardwareRayTracing", hw), ("r.Lumen.HardwareRayTracing.LightingMode", str(mode))]
        if mesh_sdfs is not None:
            commands.append(f"r.Lumen.TraceMeshSDFs {mesh_sdfs}")
            required.append(("r.Lumen.TraceMeshSDFs", str(mesh_sdfs)))
        if not screen_traces:
            commands += ["r.Lumen.ScreenProbeGather.ScreenTraces 0", "r.Lumen.Reflections.ScreenTraces 0"]
            required += [("r.Lumen.ScreenProbeGather.ScreenTraces", "0"), ("r.Lumen.Reflections.ScreenTraces", "0")]
    if shadows == "hrt":
        commands += HRT_SHADOWS
        required.append(("r.RayTracing.Shadows", "1"))
    else:
        required.append(("r.RayTracing.Shadows", "0"))
    return dict(commands=commands, required=required, lumen=lumen, quality=quality if lumen else None, mode=mode,
                shadows=shadows, aa=aa, reflections=reflections if lumen else "none", ini=list(ini), workloads=list(workloads))


ALL = ("static", "camera", "sun")
SETTLE = ("sun-step", "yaw-step")
CONFIGS = {
    # Baselines: Lumen's increment is UE-x minus the TSR baseline with the same shadows.
    "ue-0": spec(aa="none", workloads=ALL + ("pan",)),
    "ue-0-hrt": spec(aa="none", shadows="hrt"),
    "ue-tsr": spec(workloads=ALL + ("pan",) + SETTLE),
    "ue-tsr-hrt": spec(shadows="hrt"),
    # Non-RT comparators (software Lumen, VSM). Epic is the floor; High is the cost curve; Cinematic the ceiling.
    "ue-sw-h": spec("sw", 2),
    "ue-sw-e": spec("sw", 3, workloads=ALL + ("pan",) + SETTLE),
    "ue-sw-c": spec("sw", 4, workloads=ALL + ("pan",)),
    "ue-sw-e-sdf1": spec("sw", 3, mesh_sdfs=1, workloads=("static", "camera")),
    "ue-sw-e-gi": spec("sw", 3, reflections="none", workloads=("static", "camera")),
    "ue-sw-e-noscreen": spec("sw", 3, screen_traces=False, workloads=("static",)),
    # RT comparators (hardware Lumen, HRT as matched). Surface cache is the floor; hit lighting the ceiling.
    "ue-hw-h": spec("hw", 2, shadows="hrt"),
    "ue-hw-e": spec("hw", 3, shadows="hrt", workloads=ALL + ("pan",) + SETTLE),
    "ue-hw-e-hit": spec("hw", 3, 2, shadows="hrt", workloads=ALL + ("pan",) + SETTLE),
    "ue-hw-e-hit1": spec("hw", 3, 1, shadows="hrt", workloads=("static", "camera")),
    "ue-hw-c": spec("hw", 4, shadows="hrt"),
    "ue-hw-c-hit": spec("hw", 4, 2, shadows="hrt", workloads=ALL + ("pan",)),
    "ue-hw-e-vsm": spec("hw", 3, shadows="vsm", workloads=("static", "camera")),
    "ue-hw-e-gi": spec("hw", 3, reflections="none", shadows="hrt", workloads=("static", "camera")),
    # Memory only: no duplicate SDF memory beside the BVH (gi.md's recommended RT project setting).
    "ue-hw-e-nodf": spec("hw", 3, shadows="hrt", workloads=("static",),
                         ini=["-ini:Engine:[/Script/Engine.RendererSettings]:r.DistanceFields.SupportEvenIfHardwareRayTracingSupported=0"]),
}
READBACK = ["getall CameraComponent FieldOfView", "getall DirectionalLightComponent Intensity",
            "getall DirectionalLightComponent LightSourceAngle", "getall SkyLightComponent Intensity",
            "getall SkyLightComponent SourceType", "getall SkyLightComponent SourceCubemapAngle", "r.Shadow.Virtual.Cache"]
# gi.md "Effective cvars to log", plus reflection, TSR and tracing state.
LOGGED = ("r.Lumen.", "r.LumenScene.", "r.TSR.", "r.AntiAliasingMethod", "r.ScreenPercentage", "sg.", "r.DynamicGlobalIllumination",
          "r.ReflectionMethod", "r.DistanceFields.", "r.GenerateMeshDistanceFields", "r.RayTracing", "r.Shadow.Virtual.Enable",
          "r.PathTracing", "r.Nanite.MaxPixelsPerEdge", "r.MeshCardRepresentation", "r.RDG.AsyncCompute", "r.SSR.Quality",
          "r.EyeAdaptation.")
SUN_PITCH, SUN_YAW = -57.994617, -53.130102
STEP_FRAME, STEP_OFFSETS = 24, (0, 1, 2, 4, 8, 16, 32, 64, 128)


def macro(out, name, lines):
    file = out / f"{name}.txt"
    file.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return "exec " + Path(os.path.relpath(file, PACKAGE.parent.parent)).as_posix()


def workload_events(out, workload):
    """Per-frame console macro files (the e2e mechanism, which needs files for comma-bearing commands)."""
    events = []
    shot = lambda frame, name: (frame, f"HighResShot 1920x1080 filename={(out / name).as_posix()}")
    for frame in range(160):
        if workload == "camera":
            events.append((frame, macro(out, f"m{frame:03d}", [f'ke CameraActor K2_SetActorLocation "(X=-620,Y={520 + frame % 2},Z=340)" false () false'])))
        elif workload == "pan":  # 2 cm per frame from frame 96
            y = 520 + 2 * max(0, frame - 96)
            events.append((frame, macro(out, f"m{frame:03d}", [f'ke CameraActor K2_SetActorLocation "(X=-620,Y={y},Z=340)" false () false'])))
        elif workload == "sun":
            events.append((frame, macro(out, f"m{frame:03d}", [f'ke DirectionalLight K2_SetActorRotation "(Pitch={SUN_PITCH:.6f},Yaw={SUN_YAW + frame % 2:.6f},Roll=0)" false'])))
    if workload == "pan":
        events += [shot(f, f"pan-{f:03d}.png") for f in (97, 112, 128, 159)]
    if workload == "sun-step":
        events.append((STEP_FRAME, macro(out, "step", [f'ke DirectionalLight K2_SetActorRotation "(Pitch={SUN_PITCH:.6f},Yaw={SUN_YAW + 10:.6f},Roll=0)" false'])))
    if workload == "yaw-step":
        events.append((STEP_FRAME, macro(out, "step", ["set SkyLightComponent SourceCubemapAngle 10", "r.SkylightRecapture"])))
    if workload in SETTLE:
        events += [shot(STEP_FRAME - 1, "step-pre.png")] + [shot(STEP_FRAME + k, f"step-{k:03d}.png") for k in STEP_OFFSETS[1:]]
    return events


def launch_ue(out, config, workload, pose=0, extra=(), events=None, frames=160):
    c = CONFIGS[config]
    commands = BASE_COMMANDS + c["commands"] + READBACK + list(extra)
    settle = workload in SETTLE
    csv_events = [(40, f"DumpCVars -csv={(out / 'cvars.csv').as_posix()}"),
                  # Pass attribution with async off around the single ProfileGPU frame (gi.md); both defaults are 1.
                  (56, "r.RDG.Events 3"), (60, "r.RDG.AsyncCompute 0"), (61, "r.Lumen.AsyncCompute 0"), (64, "ProfileGPU"),
                  (70, "r.RDG.Events 1"), (68, "r.RDG.AsyncCompute 1"), (69, "r.Lumen.AsyncCompute 1"),
                  (86, "r.DistanceFields.LogAtlasStats 2"), (87, "r.LumenScene.DumpStats 1"),
                  (90, "D3D12.DumpRayTracingGeometries all"),
                  (95, f"DumpCVars -csv={(out / 'cvars-timed.csv').as_posix()}")]
    if not settle:
        csv_events.append((72, f"HighResShot 1920x1080 filename={(out / 'main.png').as_posix()}"))
    csv_events += workload_events(out, workload) if events is None else list(events)
    if settle:  # the step and its captures own frames 23-152; the mode readback profiles frame 156
        csv_events = [e for e in csv_events if e[1].startswith(("exec", "HighResShot", "DumpCVars"))]
        csv_events += [(153, "r.RDG.Events 3"), (156, "ProfileGPU")]
    csv_events.sort(key=lambda e: e[0])
    command = [PACKAGE, "../../../M5Calibration/M5Calibration.uproject", *UE_FLAGS,
               f"-abslog={out / 'ue.log'}", *COMMON_INI, *c["ini"], "-dx12", "-sm6", "-RenderOffscreen",
               f"/Game/M6Quarry/View{pose}", "-game", "-ResX=1920", "-ResY=1080", "-ForceRes", "-NoVSync",
               "-csvGpuStats", "-csvCompression=0", "-ExitAfterCsvProfiling",
               "-csvExecCmds=" + ",".join(f"{f}:{cmd}" for f, cmd in csv_events),
               "-ExecCmds=" + ",".join(commands + [f"csvprofile frames={frames}"])]
    csv_root = PACKAGE_DIR / "Windows/M5Calibration/Saved/Profiling/CSV"
    before = set(csv_root.glob("*.csv")) if csv_root.exists() else set()
    record = guarded(command, out, timeout=240 if frames <= 160 else 600, cwd=PACKAGE.parent)
    new = sorted(set(csv_root.glob("*.csv")) - before)
    if len(new) != 1:
        raise RuntimeError(f"expected one new CSV, found {new}")
    shutil.copyfile(new[0], out / "profile.csv")
    new[0].unlink()
    return record


def scalability_expectations():
    """Values each sg.* level sets, read from the installed BaseScalability.ini (configuration only)."""
    text = Path("C:/Program Files/Epic Games/UE_5.7/Engine/Config/BaseScalability.ini").read_text(encoding="utf-8", errors="replace")
    sections, current = {}, None
    for raw in text.splitlines():
        line = raw.split(";")[0].strip()
        if line.startswith("[") and line.endswith("]"):
            current = sections.setdefault(line[1:-1], {})
        elif current is not None and "=" in line:
            key, value = line.split("=", 1)
            current[key.strip()] = value.strip()
    return sections


def cvar_rows(path):
    rows = {}
    for line in path.read_text(encoding="utf-8-sig", errors="replace").splitlines()[1:]:
        parts = line.split(",")
        if len(parts) >= 3:
            rows[parts[0]] = dict(value=parts[1].strip('"'), set_by=parts[2])
    return rows


def same(a, b):
    try:
        return abs(float(a) - float(b)) <= 1e-6 * max(1, abs(float(b)))
    except ValueError:
        return a.lower() == b.lower()


ONE_SHOT = ("r.RDG.AsyncCompute", "r.Lumen.AsyncCompute", "r.RDG.Events", "r.DistanceFields.LogAtlasStats",
            "r.LumenScene.DumpStats")
# Project [SystemSettings] values that outrank scalability groups in the matched package. Both are
# inactive in the Lumen configurations: Lumen GI replaces DFAO, and SSR runs only when Lumen reflections are off.
PROJECT_OVERRIDES = {"r.DistanceFieldAO": "project SystemSettings 0 outranks the group; unused with Lumen GI",
                     "r.SSR.Quality": "project SystemSettings 0 outranks the group; SSR is unused with r.ReflectionMethod 1 or 0"}


def gpu_tree(out):
    """ProfileGPU rows with their ancestry, rebuilt from the printed indentation."""
    rows, stack = [], []
    for row in PROFILE.log_evidence(out / "ue.log")["gpu_tree"]:
        name = row["name"]
        depth = len(name) - len(name.lstrip())
        while stack and stack[-1][0] >= depth:
            stack.pop()
        path = [s[1] for s in stack] + [name.strip()]
        stack.append((depth, name.strip()))
        rows.append(dict(row, name=name.strip(), path=path))
    return rows


def effective_mode(out):
    """The tracing mode the engine actually ran, from Lumen's RDG pass names (not from the requested cvars)."""
    rows = gpu_tree(out)
    lumen = [r for r in rows if any("Lumen" in p or "ScreenProbe" in p or "Radiosity" in p for p in r["path"])]
    names = sorted({r["name"] for r in lumen})
    find = lambda *keys: [n for n in names if any(k.lower() in n.lower() for k in keys)]
    hardware = find("HardwareRayTracing", "HWRT", "RayTracing")
    software = find("SDF", "Voxel", "SoftwareRayTracing", "Software", "DistanceField")
    hit = find("HitLighting", "Hit Lighting", "hit-lighting", "Retrace")
    if not lumen:
        tracing = "lumen off"
    elif hardware and not software:
        tracing = "hardware"
    elif software and not hardware:
        tracing = "software"
    else:
        tracing = "mixed" if software else "unresolved"
    return dict(lumen_rows=len(lumen), lumen_pass_names=names, hardware_passes=hardware, software_passes=software,
                hit_lighting_passes=hit, tracing=tracing, hit_lighting=bool(hit),
                scope="ProfileGPU frame 64, async off, r.RDG.Events 3; Lumen subtree pass names")


PASS_GROUPS = dict(
    gi=("LumenSceneUpdate", "LumenSceneLighting", "LumenScreenProbeGather", "TranslucencyVolumeLighting", "StochasticLighting",
        "Submit Lumen surface cache feedback", "UpdateLumenSceneBuffers", "LumenShortRangeAO", "Lumen Front Layer"),
    reflections=("LumenReflections",),
    tsr=("TemporalSuperResolution",),
    shadows=("ShadowDepths", "VirtualShadowMapMarkPages", "ShadowProjectionOnOpaque", "ShadowBatch", "RayTracedShadow"),
    path_tracer=("PathTracing", "Path Tracing"),
)


def pass_costs(out):
    """Async-off attribution from the ProfileGPU tree: each group sums its outermost matching rows."""
    rows = gpu_tree(out)
    result = {}
    for group, keys in PASS_GROUPS.items():
        match = lambda name: any(name.startswith(k) for k in keys)
        top = [r for r in rows if match(r["name"]) and not any(match(p) for p in r["path"][:-1])]
        result[group] = dict(ms=round(sum(r["inclusive_ms"] for r in top), 4), rows=[(r["name"], r["inclusive_ms"]) for r in top])
    roots = [r["inclusive_ms"] for r in rows if r["name"] == "<root>"]
    result["frame_async_off_ms"] = max(roots) if roots else None
    return result


def readback(out, config, scalability):
    """Requested settings, scalability-group values and the effective tracing mode, all from the running engine."""
    c = CONFIGS[config]
    rows, timed = cvar_rows(out / "cvars.csv"), cvar_rows(out / "cvars-timed.csv")
    checks = [dict(name=n, expected=v, actual=rows.get(n), ok=n in rows and same(rows[n]["value"], v)) for n, v in c["required"]]
    explicit = {n for n, _ in c["required"]} | {"r.TSR.History.ScreenPercentage", "r.Lumen.TraceMeshSDFs"}
    groups = []
    if c["lumen"]:
        level = "Cine" if c["quality"] == 4 else str(c["quality"])
        groups += [f"GlobalIlluminationQuality@{level}", f"ReflectionQuality@{level}"]
    if c["aa"] == "tsr":
        groups.append("AntiAliasingQuality@3")
    for group in groups:
        for name, value in scalability.get(group, {}).items():
            if name in explicit or name.startswith(("+", "-")):
                continue
            actual = rows.get(name)
            ok = actual is not None and same(actual["value"], value)
            exception = PROJECT_OVERRIDES.get(name) if not ok and actual and actual["set_by"] == "SystemSettingsIni" else None
            checks.append(dict(name=name, expected=value, actual=actual, group=group, ok=ok or exception is not None,
                               exception=exception))
    drift = [n for n in rows if n.startswith(LOGGED) and n not in ONE_SHOT
             and timed.get(n, {}).get("value") != rows[n]["value"]]
    mode = effective_mode(out)
    expected_tracing = {"hw": "hardware", "sw": "software", None: "lumen off"}[c["lumen"]]
    mode_ok = mode["tracing"] == expected_tracing and (not c["lumen"] or mode["hit_lighting"] == (c["mode"] != 0))
    record = dict(config=config, checks=checks, all_ok=all(x["ok"] for x in checks) and not drift and mode_ok,
                  failed=[x for x in checks if not x["ok"]], drift_frame_40_to_95=drift, effective_mode=mode,
                  effective_mode_ok=mode_ok, logged={n: rows[n] for n in rows if n.startswith(LOGGED)},
                  total_cvars=len(rows), source="DumpCVars -csv at CSV frames 40 and 95 (NAME,VALUE,SETBY); BaseScalability.ini groups")
    save(out / "readback.json", record)
    return record["all_ok"]


def study(args):
    if not PACKAGE.is_file():
        raise SystemExit("package missing; run cook first")
    root = OUT / "study" / args.label
    root.mkdir(parents=True, exist_ok=False)
    configs = args.configs or list(CONFIGS)
    plan = ([tuple(r.split(":")) for r in args.runs] if args.runs else
            [(c, w) for c in configs for w in (args.workloads or CONFIGS[c]["workloads"])])
    scalability = scalability_expectations()
    save(root / "identity.json", dict(package_sha256=sha(PACKAGE), configs={c: CONFIGS[c] for c in configs}, plan=plan,
                                      base_commands=BASE_COMMANDS, script_sha256=sha(__file__),
                                      scalability={k: v for k, v in scalability.items() if k.startswith(("GlobalIlluminationQuality@", "ReflectionQuality@", "AntiAliasingQuality@"))}))
    results = []
    with lease("study-" + args.label, args.minutes) as owner:
        if args.discover:
            out = root / "discover"
            out.mkdir()
            launch_ue(out, "ue-0", "static", events=[(41, f"DumpCVars -showhelp -csv={(out / 'cvars-help.csv').as_posix()}"),
                                                     (42, f"DumpCCmds -csv={(out / 'ccmds.csv').as_posix()}")])
        for config, workload in plan:
            if remaining_seconds(owner) < 150:
                results.append(dict(config=config, workload=workload, skipped="lease nearly over"))
                continue
            out = root / f"{config}-{workload}"
            out.mkdir()
            start = time.perf_counter()
            try:
                try:
                    launch_ue(out, config, workload)
                except RuntimeError as error:
                    if "foreign renderer" not in str(error):
                        raise
                    shutil.move(out, out.with_name(out.name + "-stopped"))
                    out.mkdir()
                    launch_ue(out, config, workload)
                ok = readback(out, config, scalability)
                summary = PROFILE.summarize(out / "profile.csv")
                summary["pass_costs_async_off"] = pass_costs(out)
                save(out / "summary.json", summary)
                results.append(dict(config=config, workload=workload, readback_ok=ok, seconds=time.perf_counter() - start))
            except Exception as error:  # the failure is evidence; keep going
                results.append(dict(config=config, workload=workload, error=str(error)[:2000]))
            save(root / "results.json", results)
            print(json.dumps(results[-1]), flush=True)
        if args.path_tracer and remaining_seconds(owner) > 240:
            out = root / "ue-pt-static"
            out.mkdir()
            CONFIGS["ue-pt"] = dict(spec(aa="none"), commands=spec(aa="none")["commands"] + [
                "ShowFlag.PathTracing 1", f"r.PathTracing.SamplesPerPixel {args.pt_spp}", "r.PathTracing.MaxBounces 32",
                "r.PathTracing.ProgressDisplay 0"], required=[("r.PathTracing.SamplesPerPixel", str(args.pt_spp))])
            try:
                launch_ue(out, "ue-pt", "static", frames=args.pt_spp + 80,
                          events=[(40, f"DumpCVars -csv={(out / 'cvars.csv').as_posix()}"), (95, f"DumpCVars -csv={(out / 'cvars-timed.csv').as_posix()}"),
                                  (args.pt_spp + 60, f"HighResShot 1920x1080 filename={(out / 'pt.png').as_posix()}")])
                results.append(dict(config="ue-pt", workload="static", readback_ok=readback(out, "ue-pt", scalability)))
            except Exception as error:
                results.append(dict(config="ue-pt", workload="static", error=str(error)[:2000]))
            save(root / "results.json", results)
    print(json.dumps(results, indent=1))


def fixture(args):
    """D3D12 SM6 SceneCapture of the retained directional-black fixture (glossy control)."""
    out = OUT / "fixture" / args.label
    out.mkdir(parents=True, exist_ok=False)
    script = out / "m7-u0-ue-fixture.py"
    shutil.copyfile(REPO / "scripts/m7-u0-ue-fixture.py", script)
    env = dict(os.environ, RENDERLAB_M7_FIXTURE_OUT=str(out))
    with lease("fixture-" + args.label, 25) as owner:
        guarded([EDITOR, PROJECT, "-run=pythonscript", f"-script={script}",
                 *UE_FLAGS, f"-abslog={out / 'ue.log'}", "-AllowCommandletRendering", "-dx12", "-sm6", "-RenderOffscreen"],
                out, timeout=max(60, int(remaining_seconds(owner)) - 30), env=env)


def settle_curve(out):
    """Display-domain settle after a step: each capture against the last (+128 frames)."""
    import numpy as np
    from PIL import Image
    load = lambda name: np.asarray(Image.open(out / name).convert("RGB"), dtype=np.float64)
    last = load(f"step-{STEP_OFFSETS[-1]:03d}.png")
    pre = load("step-pre.png")
    curve = []
    for k in STEP_OFFSETS[1:]:
        diff = np.abs(load(f"step-{k:03d}.png") - last)
        curve.append(dict(frames_after=k, mean_abs_8bit=float(diff.mean()), p99_abs_8bit=float(np.quantile(diff, .99))))
    settled = next((c["frames_after"] for c in curve if c["mean_abs_8bit"] < .5 and c["p99_abs_8bit"] < 4), None)
    step = np.abs(pre - last)
    return dict(curve=curve, step_size=dict(mean_abs_8bit=float(step.mean()), p99_abs_8bit=float(np.quantile(step, .99))),
                frames_to_settle=settled,
                rule="first capture with mean |d| < 0.5/255 and p99 < 4/255 against the +128-frame capture (display PNG, fixed exposure)")


def summarize(args):
    rows = []
    for root in args.roots:
        for out in sorted(p for p in Path(root).iterdir() if p.is_dir() and (p / "summary.json").exists()):
            if out.name.endswith("-stopped"):
                continue
            config, workload = next(((c, out.name[len(c) + 1:]) for c in sorted(CONFIGS, key=len, reverse=True)
                                     if out.name.startswith(c + "-")), (out.name, ""))
            if not workload:
                continue
            s = json.loads((out / "summary.json").read_text())
            r = json.loads((out / "readback.json").read_text()) if (out / "readback.json").exists() else {}
            stat = s["statistics"]
            med = lambda k: round(stat[k]["median"], 4) if k in stat else None
            p95 = lambda k: round(stat[k]["p95"], 4) if k in stat else None
            costs = s.get("pass_costs_async_off") or (pass_costs(out) if (out / "ue.log").exists() else {})
            row = dict(config=config, workload=workload, run=str(out),
                       gpu_ms=med("GPUTime"), gpu_p95_ms=p95("GPUTime"), frame_ms=med("FrameTime"),
                       local_used_mib=med("GPUMem/LocalUsedMB"),
                       csv_lumen_ms=round(sum(stat[k]["median"] for k in stat if k.startswith("GPU/Lumen")), 4),
                       csv_tsr_ms=med("GPU/TemporalSuperResolution"),
                       async_off={k: v["ms"] for k, v in costs.items() if isinstance(v, dict)},
                       async_off_frame_ms=costs.get("frame_async_off_ms"),
                       tracing=r.get("effective_mode", {}).get("tracing"), hit_lighting=r.get("effective_mode", {}).get("hit_lighting"),
                       readback_ok=r.get("all_ok"), failed=[f["name"] for f in r.get("failed", [])],
                       exceptions=sorted({c["name"] for c in r.get("checks", []) if c.get("exception")}))
            if workload in SETTLE and (out / "step-pre.png").exists():
                row["settle"] = settle_curve(out)
            rows.append(row)
    save(Path(args.output), rows)
    lines = ["| Config | Workload | GPU ms (p50 / p95) | GI ms | Refl. ms | TSR ms | Shadows ms | Frame async-off | LocalUsed MiB | Tracing | Readback |",
             "|---|---|---|---|---|---|---|---|---|---|---|"]
    for r in rows:
        a = r["async_off"]
        lines.append(f"| {r['config']} | {r['workload']} | {r['gpu_ms']} / {r['gpu_p95_ms']} | {a.get('gi')} | {a.get('reflections')} | "
                     f"{a.get('tsr')} | {a.get('shadows')} | {r['async_off_frame_ms']} | {r['local_used_mib']} | "
                     f"{r['tracing']}{' + hit' if r['hit_lighting'] else ''} | {'ok' if r['readback_ok'] else 'FAIL ' + ','.join(r['failed'])} |")
    Path(args.output).with_suffix(".md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines))


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("cook")
    p.add_argument("label")
    p = sub.add_parser("study")
    p.add_argument("label")
    p.add_argument("--configs", nargs="*")
    p.add_argument("--workloads", nargs="*")
    p.add_argument("--minutes", type=int, default=40)
    p.add_argument("--discover", action="store_true")
    p.add_argument("--path-tracer", action="store_true")
    p.add_argument("--pt-spp", type=int, default=1024)
    p.add_argument("--runs", nargs="*", help="explicit config:workload pairs (reruns)")
    p = sub.add_parser("fixture")
    p.add_argument("label")
    p = sub.add_parser("summarize")
    p.add_argument("output")
    p.add_argument("roots", nargs="+")
    args = parser.parse_args()
    dict(cook=cook, study=study, fixture=fixture, summarize=summarize)[args.command](args)


if __name__ == "__main__":
    main()
