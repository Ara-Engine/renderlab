"""Background UE supervisor and bounded streaming decoder; called under motion-run."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import threading
import time
import traceback
import uuid


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("phase",choices=["prepare","render"]);parser.add_argument("--probe-first",action="store_true");args=parser.parse_args()
    root=Path(os.environ["RENDERLAB_MOTION_ROOT"]);scene=json.loads((root/"fixture/scene.json").read_text())
    script=Path(__file__).with_name("bench-motion-ue.py").resolve()
    executable="C:/Program Files/Epic Games/UE_5.7/Engine/Binaries/Win64/UnrealEditor-Cmd.exe"
    command=[executable,"D:/Coding/renderlab-work/ue-m5/M5Calibration.uproject"]
    if args.phase=="prepare":command += ["-run=pythonscript","-script="+str(script),"-NullRHI"]
    else:command += ["/Game/BenchMotion"+scene["bench"]+"/Bench","-game","-EnablePython","-ExecCmds=py "+str(script),
                    "-dx12","-sm6","-RenderOffscreen","-ResX=1920","-ResY=1080","-ForceRes","-NoVSync","-csvGpuStats","-csvCompression=0","-UseFixedTimeStep","-FPS=60"]
    command += ["-unattended","-nop4","-nosound","-NoSplash","-NoLoadingScreen","-NoShaderWorkerCrashDialog","-NoCrashDialog",
                "-corelimit=2","-stdout","-FullStdOutLogOutput","-ddc=RenderlabLocalDDC","-abslog="+(root/("unreal-"+args.phase+".log")).as_posix()]
    env=os.environ.copy();env["RENDERLAB_MOTION_PHASE"]=args.phase;env["RENDERLAB_MOTION_SESSION"]=uuid.uuid4().hex[:12]
    if args.probe_first:env["RENDERLAB_MOTION_PROBE"]="1"
    startup=subprocess.STARTUPINFO();startup.dwFlags|=subprocess.STARTF_USESHOWWINDOW;startup.wShowWindow=0
    spec=importlib.util.spec_from_file_location("motion_score",Path(__file__).with_name("bench-motion-native-batch.py"))
    scoring=importlib.util.module_from_spec(spec);spec.loader.exec_module(scoring)
    sources=[script,Path(__file__),Path(__file__).with_name("bench-motion-native-batch.py"),Path(__file__).with_name("bench-motion-score.py")]
    source_identity={p.name:hashlib.file_digest(p.open("rb"),"sha256").hexdigest() for p in sources}
    (root/"unreal-launch.json").write_text(json.dumps(dict(session=env["RENDERLAB_MOTION_SESSION"],command=command,source_sha256=source_identity),indent=2))
    stop=threading.Event();errors=[];decoded=set();pending={}
    def consume():
        while not stop.is_set():
            try:
                directories=sorted([*(root/"unreal").glob("*/stage-depth"),*(root/"unreal").glob("*/*/stage-*")])
                for directory in directories:
                    if directory in decoded:continue
                    if (directory/"complete.done").exists():decoded.add(directory);continue
                    pending.setdefault(directory,time.monotonic())
                    try:
                        scoring.consume(directory,scene);decoded.add(directory);pending.pop(directory,None)
                    except (FileNotFoundError,StopIteration,json.JSONDecodeError) as error:
                        if time.monotonic()-pending[directory]>600:raise RuntimeError(str(directory)+": "+str(error)) from error
            except Exception as error:
                (root/"unreal-decode-failure.json").write_text(json.dumps(dict(error=repr(error),traceback=traceback.format_exc()),indent=2))
                errors.append(repr(error));return
            stop.wait(.15)
    start=time.monotonic()
    with (root/("unreal-"+args.phase+"-stdout.log")).open("w",encoding="utf-8") as log:
        child=subprocess.Popen(command,env=env,stdout=log,stderr=subprocess.STDOUT,creationflags=subprocess.CREATE_NO_WINDOW,startupinfo=startup)
        thread=threading.Thread(target=consume) if args.phase=="render" else None
        if thread:thread.start()
        while child.poll() is None:
            if errors or (root/"unreal/failure.json").exists():
                subprocess.run(["taskkill","/PID",str(child.pid),"/T","/F"],capture_output=True,creationflags=subprocess.CREATE_NO_WINDOW)
                break
            time.sleep(.25)
        code=child.wait()
        if thread:
            # Give asynchronous native writes/decoder a bounded drain without
            # introducing extra simulated frames or slowing capture poses.
            deadline=time.monotonic()+(60 if code==0 and not (root/"unreal/failure.json").exists() else 0)
            expected=len(scene["tracks"])*len(scene["radii"])*(13 if scene["bench"]=="S3" else 8)
            while not errors and len(decoded)<expected and time.monotonic()<deadline:time.sleep(.25)
            stop.set();thread.join()
            if len(decoded)!=expected:errors.append(f"decoded {len(decoded)} of {expected} frames")
    record=dict(command=command,session=env["RENDERLAB_MOTION_SESSION"],exit=code,errors=errors,seconds=time.monotonic()-start,decoded_stages=len(decoded),
                source_sha256=source_identity)
    log_text=(root/"unreal-render.log").read_text(encoding="utf-8-sig",errors="replace") if args.phase=="render" else ""
    blocks=log_text.split("MOTION_VIEW ")[1:]
    for completed in (root/"unreal").glob("*/*/config-complete.json"):
        info=json.loads(completed.read_text())
        if info["session"]!=env["RENDERLAB_MOTION_SESSION"]:continue
        lines=[]
        for block in blocks:
            directory=Path(block.splitlines()[0].strip())
            if directory.parent!=completed.parent:continue
            camera=next((line for line in block.splitlines() if "CameraCachePrivate =" in line),None)
            if camera:lines.append("MOTION_VIEW "+str(directory)+"\n"+camera+"\n")
        if lines:(completed.parent/"camera.log").write_text("".join(lines),encoding="utf-8")
    (root/("unreal-"+args.phase+"-process.json")).write_text(json.dumps(record,indent=2))
    sessions=root/"unreal-sessions";sessions.mkdir(exist_ok=True)
    (sessions/(env["RENDERLAB_MOTION_SESSION"]+".json")).write_text(json.dumps(record,indent=2))
    print(json.dumps(record),flush=True)
    if (root/"unreal/failure.json").exists():raise RuntimeError((root/"unreal/failure.json").read_text())
    if code or errors:raise SystemExit(1)


if __name__=="__main__":main()
