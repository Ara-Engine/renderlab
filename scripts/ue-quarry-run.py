"""Guarded, bounded Unreal commandlet launch; assigned GPU slot is a prerequisite."""
import argparse
import datetime
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

ROOT = Path("D:/Coding/renderlab-work/ue-m5/Quarry")
EDITOR = Path("C:/Program Files/Epic Games/UE_5.7/Engine/Binaries/Win64/UnrealEditor-Cmd.exe")


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def run(mode, label, shadow, timeout, pose=0, packaged=None):
    if not label or any(c not in "abcdefghijklmnopqrstuvwxyz0123456789-" for c in label):
        raise ValueError("use a lowercase alphanumeric run label")
    if packaged and (mode not in ("profile", "verify") or not packaged.is_file()):
        raise ValueError("packaged runtime requires an existing executable and static profile/verify mode")
    spec = importlib.util.spec_from_file_location("ue_slot", Path(__file__).with_name("ue-quarry-slot.py"))
    slot = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(slot)
    lease, remaining = slot.read_lease()
    timeout = min(timeout, int(remaining) - 10)
    if timeout < 15:
        raise RuntimeError("GPU lease has too little time left; release it")
    launch = ROOT / "launches" / label
    launch.mkdir(parents=True, exist_ok=False)
    preflight = Path(__file__).with_name("ue-quarry-preflight.ps1")
    subprocess.run(["powershell", "-NoProfile", "-File", str(preflight), "-Output", str(launch / "preflight.json")], check=True,
                   creationflags=subprocess.CREATE_NO_WINDOW)
    script = launch / "ue-quarry-editor.py"
    shutil.copyfile(Path(__file__).with_name("ue-quarry-editor.py"), script)
    args = [str(EDITOR), "D:/Coding/renderlab-work/ue-m5/M5Calibration.uproject", "-run=pythonscript", f"-script={script}",
            "-unattended", "-nop4", "-nosound", "-ddc=RenderlabLocalDDC", f"-abslog={launch / 'ue.log'}",
            "-NoSplash", "-NoLoadingScreen", "-NoShaderWorkerCrashDialog", "-NoCrashDialog", "-corelimit=8", "-stdout", "-FullStdOutLogOutput",
            "-ini:Engine:[/Script/Engine.RendererSettings]:r.RayTracing=True",
            "-ini:Engine:[/Script/Engine.RendererSettings]:r.SkinCache.CompileShaders=True",
            "-ini:Engine:[SystemSettings]:r.ShadowQuality=5",
            "-ini:Engine:[SystemSettings]:r.Shadow.Virtual.Enable=1",
            "-ini:Engine:[SystemSettings]:r.MaxAnisotropy=16"]
    if packaged:
        args = [str(packaged.resolve()), "../../../M5Calibration/M5Calibration.uproject"] + args[2:]
    args += ["-NullRHI"] if mode in ("import", "assemble", "views", "filter", "capture-actors", "correct", "full-residency", "cook") else ["-AllowCommandletRendering", "-dx12", "-sm6", "-RenderOffscreen"]
    if mode == "cook":
        args = [a for a in args if not a.startswith(("-run=", "-script="))]
        args += ["-run=cook", "-TargetPlatform=Windows", "-Map=/Game/M6Quarry/View0+/Game/M6Quarry/View1+/Game/M6Quarry/View2",
                 "-unversioned", "-skipeditorcontent", "-cookcultures=en", "-NoZenAutoLaunch"]
    if mode == "package":
        args = [os.environ.get("COMSPEC", "C:/Windows/System32/cmd.exe"), "/d", "/c",
                "C:/Program Files/Epic Games/UE_5.7/Engine/Build/BatchFiles/RunUAT.bat", "BuildCookRun",
                "-project=D:/Coding/renderlab-work/ue-m5/M5Calibration.uproject", "-nop4", "-skipbuild", "-skipcook",
                "-stage", "-pak", "-archive", "-archivedirectory=D:/Coding/renderlab-work/ue-m5/Quarry/package",
                "-clientconfig=Development", "-targetplatform=Win64", "-unattended", "-utf8output",
                "-nocompile", "-nocompileuat", "-installed"]
    if mode in ("profile", "profile-motion", "capture-game", "verify", "environment-game"):
        args = [a for a in args if not a.startswith(("-run=", "-script=", "-AllowCommandletRendering"))]
        args += [f"/Game/M6Quarry/View{pose}", "-game", "-ResX=1920", "-ResY=1080", "-ForceRes", "-NoVSync"]
        commands = (f"r.ProfileGPU.ShowUI 0,r.VSync 0,t.MaxFPS 0,t.IdleWhenNotForeground 0,r.SetNearClipPlane 0.5,r.ShadowQuality 5,r.Shadow.Virtual.Enable 1,"
                 f"r.Shadow.Virtual.SMRT.RayCountDirectional 8,r.Shadow.Virtual.SMRT.SamplesPerRayDirectional 8,"
                 f"r.Shadow.Virtual.SMRT.AdaptiveRayCount 1,r.RayTracing.Shadows {int(shadow == 'hardware-rt-denoised')},"
                 "r.RayTracing.Shadows.SamplesPerPixel 1,r.Shadow.Denoiser 1,r.Shadow.Denoiser.TemporalAccumulation 1,"
                 "r.AntiAliasingMethod 0,r.ScreenPercentage 100,r.MaxAnisotropy 16,r.EyeAdaptation.PreExposureOverride 1,"
                 "r.RayTracing.Nanite.Mode 1,r.Nanite.MaxPixelsPerEdge 0.25,r.Nanite.Streaming.StreamingPoolSize 1024,"
                 "r.RayTracing.Nanite.Streaming 1,r.RayTracing.Nanite.Streaming.LODBias 0,"
                 "r.RayTracing.Nanite.Streaming.Offscreen.LODBias 0,r.RayTracing.Nanite.Streaming.Offscreen.MinCutError 0,"
                 "ShowFlag.EyeAdaptation 1,r.EyeAdaptationQuality 2,r.EyeAdaptation.MethodOverride 3,"
                 "r.Shadow.Virtual.Enable,r.RayTracing,r.RayTracing.Shadows,r.Nanite,r.AntiAliasingMethod,r.ScreenPercentage")
        if mode in ("profile", "profile-motion", "verify"):
            args += ["-csvGpuStats", "-csvCompression=0", "-ExitAfterCsvProfiling",
                     f"-csvExecCmds=64:ProfileGPU,72:HighResShot 1920x1080 filename={launch / 'main.png'},90:D3D12.DumpRayTracingGeometries all"]
            if mode == "verify":
                args = [a for a in args if not a.startswith("-csvExecCmds=")]
                args += [f"-csvExecCmds=32:NaniteStats,64:ProfileGPU,72:HighResShot 1920x1080 filename={launch / 'nanite-stats.png'},80:r.Nanite.ShowStats 0,81:r.ShaderPrintEnable 0,88:HighResShot 1920x1080 filename={launch / 'main.png'},90:D3D12.DumpRayTracingGeometries all"]
            if mode == "profile-motion":
                args += ["-EnablePython", "-ExecCmds=" + commands + ",py " + str(script) + ",csvprofile frames=160"]
            else:
                args += ["-ExecCmds=" + commands + ",csvprofile frames=160"]
        else:
            args += ["-EnablePython", "-ExecCmds=" + commands + ",py " + str(script)]
    csv_root = (ROOT / "package/Windows/M5Calibration/Saved/Profiling/CSV") if packaged else ROOT.parent / "Saved/Profiling/CSV"
    csv_before = set(csv_root.glob("*.csv"))
    env = os.environ.copy()
    env.update(RENDERLAB_UE_QUARRY_MODE=mode, RENDERLAB_UE_QUARRY_RUN=label, RENDERLAB_UE_QUARRY_SHADOW=shadow)
    record = dict(start_utc=now(), command=args, mode=mode, shadow=shadow, gpu_lease=lease,
                  executable_sha256=hashlib.sha256(Path(args[0]).read_bytes()).hexdigest(), packaged=bool(packaged),
                  script_sha256=hashlib.sha256(script.read_bytes()).hexdigest(), timeout_seconds=timeout)
    start = time.perf_counter()
    stop = threading.Event()
    samples = []
    def stop_owned_tree(proc):
        if proc.poll() is None:
            subprocess.run(["taskkill", "/PID", str(proc.pid), "/T", "/F"], capture_output=True,
                           creationflags=subprocess.CREATE_NO_WINDOW)
            proc.wait()
    startup = subprocess.STARTUPINFO()
    startup.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = 0
    with (launch / "stdout.log").open("w", encoding="utf-8") as log:
        proc = subprocess.Popen(args, env=env, stdout=log, stderr=subprocess.STDOUT, creationflags=subprocess.CREATE_NO_WINDOW,
                                cwd=str(packaged.parent) if packaged else None, startupinfo=startup)
        record["process_id"] = proc.pid
        def monitor():
            while not stop.wait(2):
                names = subprocess.check_output(["tasklist", "/fo", "csv", "/nh"], text=True, creationflags=subprocess.CREATE_NO_WINDOW)
                game = [line.split(',')[0].strip('"') for line in names.splitlines()
                        if re.search(r"valorant|eldenring|cyberpunk|helldivers|cs2\.exe|fortniteclient|overwatch|arcraiders|bf6\.exe|league of legends|r5apex|dota2|rocketleague|minecraft|win64-shipping|shipping\.exe", line.split(',')[0], re.I)]
                gpu = subprocess.run(["nvidia-smi", "--query-gpu=utilization.gpu,memory.used,clocks.gr,temperature.gpu,power.draw", "--format=csv,noheader"],
                                     capture_output=True, text=True, creationflags=subprocess.CREATE_NO_WINDOW, timeout=5)
                foreign = [line.split(',')[0].strip('"') for line in names.splitlines()
                           if any(g in line.lower().split(',')[0] for g in ("renderlab", "fresh-tests", "m6-shadows", "sel2shadow"))]
                samples.append(dict(seconds=time.perf_counter() - start, gpu=gpu.stdout.strip(), games=game, foreign_renderers=foreign))
                if (game or foreign) and proc.poll() is None:
                    record["stop_reason"] = "game or foreign renderer started; terminating only this owned Unreal process"
                    stop_owned_tree(proc)
                    return
        thread = threading.Thread(target=monitor)
        thread.start()
        try:
            proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            record["stop_reason"] = "bounded run timeout"
            stop_owned_tree(proc)
        finally:
            stop.set()
            thread.join()
            if proc.poll() is None:
                stop_owned_tree(proc)
    record.update(end_utc=now(), elapsed_seconds=time.perf_counter() - start, exit_code=proc.returncode, gpu_samples=samples,
                  scope="whole process, including import/shader/loading/readback/export; NOT GPU frame time")
    postflight = subprocess.run(["powershell", "-NoProfile", "-File", str(preflight), "-Output", str(launch / "postflight.json")],
        capture_output=True, text=True, creationflags=subprocess.CREATE_NO_WINDOW)
    record["postflight_exit_code"] = postflight.returncode
    before = json.loads((launch / "preflight.json").read_text(encoding="utf-8-sig"))
    if (launch / "postflight.json").is_file() and "foreground_read_only" in before:
        after = json.loads((launch / "postflight.json").read_text(encoding="utf-8-sig"))
        record["same_foreground_before_after"] = before["foreground_read_only"] == after["foreground_read_only"]
    record["csv_outputs"] = []
    for path in sorted(set(csv_root.glob("*.csv")) - csv_before):
        shutil.copyfile(path, launch / path.name)
        record["csv_outputs"].append(dict(source=str(path), copy=path.name, sha256=hashlib.sha256(path.read_bytes()).hexdigest()))
    (launch / "process.json").write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps({k: record[k] for k in ("mode", "elapsed_seconds", "exit_code")}, indent=2))
    if proc.returncode:
        raise SystemExit(proc.returncode)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("import", "assemble", "capture", "capture-game", "capture-actors", "views", "filter", "profile", "profile-motion", "correct", "verify", "full-residency", "environment-game", "cook", "package"))
    parser.add_argument("label")
    parser.add_argument("--shadow", choices=("vsm-smrt", "hardware-rt-denoised"), default="vsm-smrt")
    parser.add_argument("--timeout", type=int, default=900)
    parser.add_argument("--pose", type=int, choices=(0, 1, 2), default=0)
    parser.add_argument("--packaged", type=Path, help="existing private staged Development executable; no build or cook implied")
    options = parser.parse_args()
    run(options.mode, options.label, options.shadow, options.timeout, options.pose, options.packaged)
