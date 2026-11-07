"""M7-U0b owned builds, hidden leases, quarry references and Unreal follow-through.

All generated data is on E:. Each GPU command blocks on its own bounded lease, runs no Cargo,
and uses U0's game/renderer and 4/2-GiB memory guards. Existing leases are never removed.
"""
import argparse
import contextlib
import importlib.util
import json
import math
import os
import re
from pathlib import Path
import shutil
import subprocess
import time

REPO = Path(__file__).resolve().parent.parent
OUT = Path("E:/renderlab-work/m7-u0b")
BASE = Path("E:/renderlab-work/m7-u0")
TARGET = OUT/"target"
BINARY = OUT/"bin/reference-tests.exe"


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-","_"),REPO/"scripts"/(name+".py"))
    module = importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    return module


def ue():
    u=load("m7-u0-ue")
    u.OUT=OUT;u.OWNER="m7-u0b"
    base_tool=u.engine_tool
    def engine_tool(p):
        if base_tool(p):return True
        if re.fullmatch(r"renderlab-[0-9a-f]+\.exe",p["Name"].lower()):
            # Non-ignored library tests are CPU-only. A running ignored test stays a
            # renderer conflict; a PID which exited between snapshots is no conflict.
            cmd=u.E2E.powershell(f"Get-CimInstance Win32_Process -Filter 'ProcessId={int(p['ProcessId'])}' | Select-Object CommandLine | ConvertTo-Json -Compress").strip()
            if not cmd:return True
            line=(json.loads(cmd).get("CommandLine") or "")
            return bool(line) and "--ignored" not in line and "--include-ignored" not in line
        return False
    u.engine_tool=engine_tool
    return u


def build(args):
    OUT.mkdir(parents=True,exist_ok=True)
    if (ue().LOCK/"owner.json").exists():
        lock=json.loads((ue().LOCK/"owner.json").read_text())
        if lock.get("owner")=="m7-u0b":raise RuntimeError("no Cargo in own lease")
    begin=time.perf_counter()
    with (OUT/"build.jsonl").open("w") as log,(OUT/"build.stderr.log").open("w") as err:
        p=subprocess.run(["cargo","test","-p","renderlab","--lib","-j","2","--target-dir",str(TARGET),"--message-format=json"],
                         cwd=REPO,stdout=log,stderr=err,creationflags=subprocess.CREATE_NO_WINDOW)
    if p.returncode:raise SystemExit((OUT/"build.stderr.log").read_text()[-3000:])
    records=[json.loads(l) for l in (OUT/"build.jsonl").read_text().splitlines() if l.startswith("{")]
    exe=next(x["executable"] for x in reversed(records) if x.get("reason")=="compiler-artifact" and x.get("executable") and x["target"]["name"]=="renderlab")
    BINARY.parent.mkdir(exist_ok=True)
    shutil.copyfile(exe,BINARY)
    ue().save(OUT/"build.json",dict(seconds=time.perf_counter()-begin,exit_code=p.returncode,binary_sha256=ue().sha(BINARY)))
    print((OUT/"build.json").read_text())


def reference(args):
    u=ue();out=OUT/"reference"/args.label;out.mkdir(parents=True,exist_ok=True)
    env=dict(os.environ,RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
             RENDERLAB_M7_REFERENCE_OUT=str(out),VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation")
    env.pop("VK_INSTANCE_LAYERS",None)
    test="reference::tests::gpu_reference_analytic"
    if args.benches:
        env["RENDERLAB_M7_BENCHES"]=";".join(str(BASE/"scenes"/x.split("@")[0])+"@"+x.split("@")[1] for x in args.benches)
        test="reference::tests::gpu_reference_benches"
    if args.manifest:
        env["RENDERLAB_M7_QUARRY_MANIFEST"]=str(args.manifest)
        test="reference::quarry_tests::gpu_reference_quarry"
    with u.lease("reference-"+args.label,args.minutes) as owner:
        u.guarded([BINARY,test,"--exact","--ignored","--nocapture","--test-threads=1"],out/"process",
                  timeout=max(30,int(u.remaining_seconds(owner))-10),cwd=REPO,env=env)


def prepare(args):
    u=ue();out=OUT/"import";out.mkdir(parents=True,exist_ok=True)
    env=dict(os.environ,RENDERLAB_U0B_ROOT=str(OUT))
    with u.lease("bench-import",35) as owner:
        for attempt in range(12):
            run=out/f"attempt-{attempt}"
            if run.exists():continue
            try:
                u.guarded([u.EDITOR,u.PROJECT,*u.UE_FLAGS,*u.COMMON_INI,"-NullRHI","-run=pythonscript",
                           "-script="+str(REPO/"scripts/m7-u0b-ue-prepare.py"),"-abslog="+str(run/"ue.log")],run,
                          timeout=int(u.remaining_seconds(owner))-15,env=env)
                break
            except RuntimeError:
                log=(run/"stdout.log").read_text(errors="replace")
                if "Assertion failed: CurrentApplication.IsValid()" not in log or (out/"failure.txt").exists():raise
                print("Saved Interchange assets before known commandlet Slate assertion; resuming",flush=True)
        if not all((out/(b+".json")).exists() for b in ("SV","S4","S5","S2G")):raise RuntimeError("imports incomplete")
    if (out/"failure.txt").exists():raise RuntimeError((out/"failure.txt").read_text())


def cook(args):
    u=ue();out=OUT/"cook"/args.label;out.mkdir(parents=True,exist_ok=True)
    maps=[f"/Game/M6Quarry/View{i}" for i in range(3)]+[f"/Game/M7U0b/{b}/Bench" for b in ("SV","S4","S5","S2G")]
    with u.lease("cook-"+args.label,45) as owner:
        u.guarded([u.EDITOR,u.PROJECT,*u.UE_FLAGS,*u.COMMON_INI,"-NullRHI","-run=cook","-TargetPlatform=Windows",
                   "-Map="+"+".join(maps),"-unversioned","-skipeditorcontent","-cookcultures=en","-NoZenAutoLaunch",
                   "-abslog="+str(out/"cook/ue.log")],out/"cook",timeout=int(u.remaining_seconds(owner))-300)
        u.guarded([os.environ["COMSPEC"],"/d","/c",u.UAT,"BuildCookRun",f"-project={u.PROJECT}","-nop4","-skipbuild",
                   "-skipcook","-stage","-pak","-archive",f"-archivedirectory={OUT/'ue-package'}","-clientconfig=Development",
                   "-targetplatform=Win64","-unattended","-utf8output","-nocompile","-nocompileuat","-installed"],
                  out/"package",timeout=int(u.remaining_seconds(owner))-10)
    u.save(out/"identity.json",dict(executable_sha256=u.sha(u.PACKAGE),maps=maps,
        paks={str(p.relative_to(OUT/"ue-package")):u.sha(p) for p in (OUT/"ue-package").rglob("*.pak")},
        utocs={str(p.relative_to(OUT/"ue-package")):u.sha(p) for p in (OUT/"ue-package").rglob("*.utoc")}))


@contextlib.contextmanager
def quiet_desktop(u,root):
    """Pause the live wallpaper through its supported watched setting, then restore only that
    setting. Do not stop applications or overwrite concurrent changes to other preferences."""
    path=Path(os.environ["APPDATA"])/"mypaper/config.json"
    config=json.loads(path.read_text(encoding="utf-8-sig")) if path.exists() else None
    original=config.get("paused",False) if config is not None else None
    u.save(root/"wallpaper-state.json",dict(path=str(path),original_paused=original,method="supported watched config.paused; restored in finally"))
    if config is not None:
        config["paused"]=True;path.write_text(json.dumps(config,indent=2)+"\n",encoding="utf-8")
    try:
        samples=[]
        for _ in range(15):
            state=u.E2E.state();samples.append(state)
            if state["games"]:raise RuntimeError("game present")
            if float(state["gpu"].split(",")[2])<=3:break
            time.sleep(2)
        u.save(root/"quiet-start.json",samples)
        if float(samples[-1]["gpu"].split(",")[2])>3:raise RuntimeError("no quiet GPU start after wallpaper pause")
        yield
    finally:
        if original is not None:
            current=json.loads(path.read_text(encoding="utf-8-sig"))
            current["paused"]=original;path.write_text(json.dumps(current,indent=2)+"\n",encoding="utf-8")


def study(args):
    u=ue();root=OUT/"study"/args.label;root.mkdir(parents=True,exist_ok=True)
    configs=args.configs or list(u.CONFIGS)
    benches=args.benches or ["quarry"]
    u.schedule_line(f"[m7-u0b gap request] quiet={args.quiet} asks for an exclusive {args.minutes}-minute gap: {configs}, benches={benches}; no Cargo, hidden/offscreen, wallpaper paused only for quiet timings.")
    with u.lease("study-"+args.label,args.minutes) as owner:
        with quiet_desktop(u,root) if args.quiet else contextlib.nullcontext():
            for visit in range(args.repeats):
                for bench in benches:
                    u.PACKAGE_DIR=(BASE if bench=="quarry" else OUT)/"ue-package"
                    u.PACKAGE=u.PACKAGE_DIR/"Windows/Engine/Binaries/Win64/UnrealGame.exe"
                    for config in configs if visit%2==0 else reversed(configs):
                        if u.remaining_seconds(owner)<100:raise RuntimeError("lease nearly over; resume unfinished runs")
                        out=root/f"{bench}-{config}-{args.workload}-{visit}";out.mkdir(exist_ok=True)
                        if (out/"readback.json").exists() and json.loads((out/"readback.json").read_text())["all_ok"]:continue
                        extra=[];events=[]
                        if args.hdr:
                            extra=["r.DumpGPU.Upload 0","r.DumpGPU.Explore 0","r.DumpGPU.Stream 0","r.DumpGPU.Texture 2",
                                   "r.DumpGPU.Buffer 2","r.DumpGPU.PassParameters 1","r.DumpGPU.FrameCount 1","r.DumpGPU.Root Tonemap*",
                                   f"r.DumpGPU.Directory {(out/'hdr').as_posix()}"]
                            events=[(178,"getall PlayerCameraManager CameraCachePrivate"),(180,"DumpGPU")]
                        if args.workload!="static":
                            events=[e for e in u.workload_events(out,args.workload) if not e[1].startswith("HighResShot")]
                            if args.hdr and args.workload=="pan":
                                for f in (97,112,128,159):
                                    events += [(f-2,f"r.DumpGPU.Directory {(out/f'hdr-{f:03}').as_posix()}"),(f-1,"DumpGPU"),(f+1,"getall PlayerCameraManager CameraCachePrivate")]
                        u.launch_ue(out,config,args.workload,extra=extra,events=events,frames=220 if args.hdr else 160,
                                    level=None if bench=="quarry" else f"/Game/M7U0b/{bench}/Bench")
                        ok=u.readback(out,config,u.scalability_expectations())
                        summary=u.PROFILE.summarize(out/"profile.csv");summary["pass_costs_async_off"]=u.pass_costs(out)
                        u.save(out/"summary.json",summary)
                        print(json.dumps(dict(case=out.name,readback_ok=ok,gpu=summary["statistics"]["GPUTime"])),flush=True)
                        if not ok:raise RuntimeError("readback failed "+str(out))


def main():
    p=argparse.ArgumentParser(description=__doc__);sub=p.add_subparsers(dest="cmd",required=True)
    sub.add_parser("build");sub.add_parser("prepare")
    q=sub.add_parser("cook");q.add_argument("label")
    q=sub.add_parser("reference");q.add_argument("label");q.add_argument("--minutes",type=int,default=15)
    q.add_argument("--benches",nargs="+");q.add_argument("--manifest",type=Path)
    q=sub.add_parser("study");q.add_argument("label");q.add_argument("--minutes",type=int,default=30)
    q.add_argument("--configs",nargs="+");q.add_argument("--benches",nargs="+");q.add_argument("--repeats",type=int,default=1)
    q.add_argument("--quiet",action="store_true");q.add_argument("--hdr",action="store_true")
    q.add_argument("--workload",choices=("static","camera","sun","pan"),default="static")
    args=p.parse_args();globals()[args.cmd](args)


if __name__=="__main__":main()
