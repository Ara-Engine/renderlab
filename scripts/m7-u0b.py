"""M7-U0b owned builds, hidden leases, quarry references and Unreal follow-through.

All generated data is on E:. Each GPU command blocks on its own bounded lease, runs no Cargo,
and uses U0's game/renderer and 4/2-GiB memory guards. Existing leases are never removed.
"""
import argparse
import contextlib
import ctypes
from ctypes import wintypes
import datetime as dt
import hashlib
import importlib.util
import json
import math
import os
import re
from pathlib import Path
import shutil
import subprocess
import threading
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


def game_state():
    state="released"
    for line in Path("E:/renderlab-work/gpu-schedule.md").read_text(encoding="utf-8").splitlines():
        if line.startswith("[GAME HOLD RELEASED]"):state="released"
        elif line.startswith("[GAME HOLD RELAXED]"):state="relaxed"
        elif line.startswith("[GAME HOLD]"):state="hold"
    return state


def native_process_inventory():
    """Win32 snapshot with the same PID/parent/name/path fields as the WMI guard.

    Protected or exited processes may have no readable image path, just as in WMI;
    their names/PIDs remain in the mandatory game/renderer inventory. Other API
    errors reject the snapshot. No process is opened with mutation rights.
    """
    class Entry(ctypes.Structure):
        _fields_=[("size",wintypes.DWORD),("usage",wintypes.DWORD),("pid",wintypes.DWORD),
            ("heap",ctypes.c_size_t),("module",wintypes.DWORD),("threads",wintypes.DWORD),
            ("parent",wintypes.DWORD),("priority",wintypes.LONG),("flags",wintypes.DWORD),
            ("name",wintypes.WCHAR*260)]
    api=ctypes.WinDLL("kernel32",use_last_error=True)
    signatures={"CreateToolhelp32Snapshot":([wintypes.DWORD,wintypes.DWORD],wintypes.HANDLE),
        "Process32FirstW":([wintypes.HANDLE,ctypes.POINTER(Entry)],wintypes.BOOL),
        "Process32NextW":([wintypes.HANDLE,ctypes.POINTER(Entry)],wintypes.BOOL),
        "OpenProcess":([wintypes.DWORD,wintypes.BOOL,wintypes.DWORD],wintypes.HANDLE),
        "QueryFullProcessImageNameW":([wintypes.HANDLE,wintypes.DWORD,wintypes.LPWSTR,ctypes.POINTER(wintypes.DWORD)],wintypes.BOOL),
        "GetExitCodeProcess":([wintypes.HANDLE,ctypes.POINTER(wintypes.DWORD)],wintypes.BOOL),
        "CloseHandle":([wintypes.HANDLE],wintypes.BOOL)}
    for name,(arguments,result) in signatures.items():
        function=getattr(api,name);function.argtypes=arguments;function.restype=result
    def close(handle):
        if not api.CloseHandle(handle):raise ctypes.WinError(ctypes.get_last_error())
    snapshot=api.CreateToolhelp32Snapshot(2,0)  # TH32CS_SNAPPROCESS
    if not snapshot or snapshot==ctypes.c_void_p(-1).value:raise ctypes.WinError(ctypes.get_last_error())
    rows=[]
    try:
        entry=Entry();entry.size=ctypes.sizeof(entry)
        more=api.Process32FirstW(snapshot,ctypes.byref(entry))
        while more:
            row=dict(Name=entry.name,ProcessId=int(entry.pid),ParentProcessId=int(entry.parent),ExecutablePath=None)
            process=api.OpenProcess(0x1000,False,entry.pid)  # PROCESS_QUERY_LIMITED_INFORMATION
            if process:
                try:
                    path=ctypes.create_unicode_buffer(32768);length=wintypes.DWORD(len(path))
                    if api.QueryFullProcessImageNameW(process,0,path,ctypes.byref(length)):
                        row["ExecutablePath"]=path.value
                    else:
                        row["image_query_error"]=ctypes.get_last_error()
                        if row["image_query_error"]==31:
                            status=wintypes.DWORD()
                            if not api.GetExitCodeProcess(process,ctypes.byref(status)):
                                raise ctypes.WinError(ctypes.get_last_error())
                            row["exited_during_image_query"]=status.value!=259  # STILL_ACTIVE
                finally:close(process)
            else:row["image_query_error"]=ctypes.get_last_error()
            if row.get("image_query_error",0) not in (0,5,87,1168) and not row.get("exited_during_image_query"):
                raise RuntimeError("process image query failed: "+json.dumps(row))
            rows.append(row)
            more=api.Process32NextW(snapshot,ctypes.byref(entry))
        if ctypes.get_last_error()!=18:raise ctypes.WinError(ctypes.get_last_error())  # ERROR_NO_MORE_FILES
    finally:close(snapshot)
    if not any(p["ProcessId"]==os.getpid() for p in rows):raise RuntimeError("process snapshot omitted this monitor")
    return rows


def cpu_readiness(u,timing=False):
    """Compiler-free valve for timing only; correctness retains the RAM start guard."""
    pause=OUT/"resource-pause.json"
    started=time.monotonic();clear_since=None;next_notice=0
    while True:
        inventory_ok=True
        try:
            processes=sorted({p["Name"].lower() for p in native_process_inventory()
                if p["Name"].lower() in ("cargo.exe","rustc.exe","clippy-driver.exe","link.exe","unrealbuildtool.exe")})
        except Exception as error:
            inventory_ok=False;processes=["process-inventory-error: "+repr(error)]
        ram=u.E2E.available_gib();now=time.monotonic()
        if processes:clear_since=None
        elif clear_since is None:clear_since=now
        waiting=not inventory_ok or ram<u.START_GIB or (timing and (processes or now-clear_since<60))
        record=dict(utc=dt.datetime.now(dt.timezone.utc).isoformat(),foreign_compilers=processes,
            available_gib=ram,timing_eligible_for_claim=False if processes else None,
            compiler_free_seconds=0 if clear_since is None else now-clear_since,
            timing_requested=timing,cargo_free_required=timing,paused_for_memory=ram<u.START_GIB,
            waiting=bool(waiting),seconds=now-started)
        if not waiting:
            if pause.exists():
                u.save(OUT/"resource-pause-released.json",dict(json.loads(pause.read_text()),released=record));pause.unlink()
            with (OUT/"cpu-readiness.jsonl").open("a") as log:log.write(json.dumps(record)+"\n")
            return record
        if now>=next_notice:
            print("Waiting outside GPU lease for CPU/memory readiness",json.dumps(record),flush=True);next_notice=now+30
        time.sleep(5)


def ue(allow_game=False):
    u=load("m7-u0-ue")
    u.OUT=OUT;u.OWNER="m7-u0b"
    original_state=u.E2E.state
    original_powershell=u.E2E.powershell
    original_available=u.E2E.available_gib
    monitor_context=threading.local()
    class MonitorCommands:
        """TR0 79bcfc5: GPU telemetry is preflight-only; process checks stay live."""
        def __getattr__(self,name):return getattr(subprocess,name)
        def run(self,command,*a,**kw):
            if getattr(monitor_context,"active",False) and Path(str(command[0])).stem.lower()=="nvidia-smi":
                return subprocess.CompletedProcess(command,0,"not sampled by live conflict guard","")
            return subprocess.run(command,*a,**kw)
    u.E2E.subprocess=MonitorCommands()
    # Reuse TR0 b9760bc's native-memory and fail-closed monitor controls.
    def monitor_powershell(command):
        if command.startswith("Get-CimInstance Win32_Process | Select-Object Name,ProcessId,ParentProcessId,ExecutablePath"):
            return json.dumps(native_process_inventory())
        if command.startswith("Get-CimInstance Win32_OperatingSystem"):
            class Memory(ctypes.Structure):
                _fields_=[("length",ctypes.c_ulong),("load",ctypes.c_ulong)]+[
                    (name,ctypes.c_ulonglong) for name in
                    ("total_physical","available_physical","total_page","available_page",
                     "total_virtual","available_virtual","extended_virtual")]
            value=Memory();value.length=ctypes.sizeof(value)
            if not ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(value)):raise ctypes.WinError()
            return json.dumps(dict(FreePhysicalMemory=value.available_physical//1024,
                TotalVisibleMemorySize=value.total_physical//1024))
        return original_powershell(command)
    u.E2E.powershell=monitor_powershell
    def available():
        try:
            value=original_available()
            return value if math.isfinite(value) and value>=0 else 0.
        except Exception:return 0.
    u.E2E.available_gib=available
    def game_filter(snapshot):
        mode=game_state()
        snapshot["game_protocol"]=mode
        if allow_game and mode=="relaxed":
            snapshot["permitted_game_processes"]=snapshot["games"]
            snapshot["games"]=[]
            snapshot["timings_contended"]=True
        return snapshot
    def state():
        try:
            snapshot=original_state()
            snapshot["monitor_memory_source"]="TR0 b9760bc GlobalMemoryStatusEx; launch/live floor uses PDH"
            snapshot["monitor_process_source"]="Win32 Toolhelp snapshot + QueryFullProcessImageNameW; mandatory at preflight/live"
            snapshot["monitor_gpu_telemetry"]="TR0 79bcfc5: preflight only; live process/game checks remain mandatory"
        except Exception as error:
            sentinel=dict(Name="u0b-monitor-error",ProcessId=-1,ParentProcessId=-1,
                ExecutablePath="",monitor_error=repr(error))
            snapshot=dict(games=[],foreign_renderers=[sentinel],builds=[],free_gib=0,
                available_gib=0,gpu="unavailable",monitor_error=repr(error))
        return game_filter(snapshot)
    u.E2E.state=state
    def monitor_state():
        monitor_context.active=True
        try:
            snapshot=state()
            if getattr(u,"WAIT_FOR_FOREIGN_CPU_CLEAR",False) and snapshot.get("builds"):
                snapshot["monitor_error"]="foreign compiler/build began during quiet timing"
            return snapshot
        finally:monitor_context.active=False
    u.MONITOR_STATE=monitor_state
    original_preflight=u.preflight
    u.preflight=lambda path,wait_seconds=0:original_preflight(path,wait_seconds=0)
    def abort_requested():
        mode=game_state()
        return "GAME HOLD requested" if mode=="hold" or (mode=="relaxed" and not allow_game) else None
    u.ABORT_REQUESTED=abort_requested
    if allow_game and game_state()=="relaxed":
        u.START_GIB=6.0;u.ABORT_GIB=3.0
        u.UE_FLAGS=["-corelimit=2" if f=="-corelimit=8" else f for f in u.UE_FLAGS]
    base_tool=u.engine_tool
    def engine_tool(p):
        if base_tool(p):return True
        path=(p.get("ExecutablePath") or "").lower().replace("/","\\")
        if p["Name"].lower()=="build-script-build.exe" and "\\target\\" in path and "\\build\\crossbeam-" in path:return True
        if re.fullmatch(r"renderlab-[0-9a-f]+\.exe",p["Name"].lower()):
            # Non-ignored library tests are CPU-only. A running ignored test stays a
            # renderer conflict; a PID which exited between snapshots is no conflict.
            cmd=u.E2E.powershell(f"Get-CimInstance Win32_Process -Filter 'ProcessId={int(p['ProcessId'])}' | Select-Object CommandLine | ConvertTo-Json -Compress").strip()
            if not cmd:return True
            line=(json.loads(cmd).get("CommandLine") or "")
            return bool(line) and "--ignored" not in line and "--include-ignored" not in line
        return False
    u.engine_tool=engine_tool
    original_lease=u.lease
    original_guarded=u.guarded
    active_owner=None
    def bounded_guarded(command,*a,**kw):
        if Path(str(command[0])).name.lower()=="unrealgame.exe":
            length=len(subprocess.list2cmdline([str(c) for c in command]))
            if length>=16000:raise ValueError(f"UE's 16-KiB FCommandLine bound: {length}")
        if active_owner is not None:
            remaining=int(u.remaining_seconds(active_owner))-5
            if remaining<1:raise RuntimeError("owned GPU lease is ending")
            kw["timeout"]=min(kw.get("timeout",remaining),remaining)
        return original_guarded(command,*a,**kw)
    u.guarded=bounded_guarded
    @contextlib.contextmanager
    def queued_lease(label,minutes):
        nonlocal active_owner
        mode=game_state()
        if mode=="hold" or (mode=="relaxed" and not allow_game):raise SystemExit("Queued until GAME HOLD RELEASED: "+label)
        if mode=="relaxed":minutes=min(minutes,3)
        next_notice=0
        while True:
            cpu=cpu_readiness(u,timing=getattr(u,"WAIT_FOR_FOREIGN_CPU_CLEAR",False))
            queued_at=time.monotonic()
            while u.LOCK.exists():
                mode=game_state()
                if mode=="hold" or (mode=="relaxed" and not allow_game):raise SystemExit("Queued until GAME HOLD RELEASED: "+label)
                if time.monotonic()>=next_notice:
                    print("Waiting for shared GPU slot; requesting",label,flush=True)
                    next_notice=time.monotonic()+120
                time.sleep(.5)
            mode=game_state()
            if mode=="hold" or (mode=="relaxed" and not allow_game):raise SystemExit("Queued until GAME HOLD RELEASED: "+label)
            if time.monotonic()-queued_at>5:cpu=cpu_readiness(u,timing=getattr(u,"WAIT_FOR_FOREIGN_CPU_CLEAR",False))
            acquired=False
            try:
                with original_lease(label,minutes) as owner:
                    acquired=True;active_owner=owner
                    owner["foreign_cpu_readiness"]=cpu
                    try:yield owner
                    finally:active_owner=None
                return
            except SystemExit as error:
                # Readiness can take time: another worker may win mkdir after the
                # unlocked observation. Retry only an unacquired lock conflict.
                if acquired or error.code!=3:raise
                time.sleep(.5)
    u.lease=queued_lease
    return u


def build(args):
    OUT.mkdir(parents=True,exist_ok=True)
    if (ue().LOCK/"owner.json").exists():
        lock=json.loads((ue().LOCK/"owner.json").read_text())
        if lock.get("owner")=="m7-u0b":raise RuntimeError("no Cargo in own lease")
    begin=time.perf_counter()
    def foreign_quiet():
        path=ue().LOCK/"owner.json"
        try:owner=json.loads(path.read_text())
        except (FileNotFoundError,json.JSONDecodeError):return None
        # Reservation labels do not reliably contain "quiet". Every foreign GPU
        # lease excludes our build, including functional jobs with little RAM.
        if owner.get("owner")!="m7-u0b":return owner
        return None
    if foreign_quiet():raise SystemExit("CPU build deferred for another worker's measurement lease")
    with (OUT/"build.jsonl").open("w") as log,(OUT/"build.stderr.log").open("w") as err:
        p=subprocess.Popen(["cargo","test","-p","renderlab","--lib","-j","2","--target-dir",str(TARGET),"--message-format=json"],
                         cwd=REPO,stdout=log,stderr=err,creationflags=subprocess.CREATE_NO_WINDOW)
        while p.poll() is None:
            owner=foreign_quiet()
            if owner:
                subprocess.run(["taskkill","/PID",str(p.pid),"/T","/F"],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,
                               creationflags=subprocess.CREATE_NO_WINDOW)
                p.wait()
                raise SystemExit("Owned CPU build stopped for quiet owner "+owner["owner"])
            time.sleep(.25)
    if p.returncode:raise SystemExit((OUT/"build.stderr.log").read_text()[-3000:])
    records=[json.loads(l) for l in (OUT/"build.jsonl").read_text().splitlines() if l.startswith("{")]
    exe=next(x["executable"] for x in reversed(records) if x.get("reason")=="compiler-artifact" and x.get("executable") and x["target"]["name"]=="renderlab")
    BINARY.parent.mkdir(exist_ok=True)
    shutil.copyfile(exe,BINARY)
    inputs=("src/reference.rs","src/reference_inputs.rs","src/reference_tests.rs","src/reference_quarry_tests.rs","shaders/reference_path_tracer.wgsl")
    ue().save(OUT/"build.json",dict(seconds=time.perf_counter()-begin,exit_code=p.returncode,binary_sha256=ue().sha(BINARY),
        inputs_sha256={name:ue().sha(REPO/name) for name in inputs}))
    print((OUT/"build.json").read_text())


def build_gap(args):
    """The explicitly coordinated, lease-free CPU gap immediately after G1b-q2."""
    u=ue();next_notice=0
    while True:
        released=any(line.startswith("m7-g1b g1b-q2 window released")
            for line in u.SCHEDULE.read_text().splitlines())
        if released and not u.LOCK.exists():break
        if time.monotonic()>=next_notice:
            print("CPU build queued until G1b-q2 release; no GPU/build starts",flush=True);next_notice=time.monotonic()+120
        time.sleep(.5)
    if game_state()!="released":raise SystemExit("CPU measurement build deferred during game hold")
    started=dt.datetime.now(dt.timezone.utc).isoformat();status="failed"
    u.schedule_line("[m7-u0b CPU build gap START] "+started+" no GPU lease may start; max2 Cargo jobs; strict quarry replay harness")
    try:
        build(args);status="passed"
    finally:
        ended=dt.datetime.now(dt.timezone.utc).isoformat()
        u.schedule_line("[m7-u0b CPU build gap END] "+ended+" status="+status+"; no owned Cargo remains; one-state alternation may resume")
        u.save(OUT/"build-gap.json",dict(start=started,end=ended,status=status))
    gate=OUT/"gpu-defer.txt"
    if gate.exists() and gate.read_text().startswith("Diagnosing a reference conflict-monitor timeout"):
        gate.unlink()
    finish(args)


def reference(args):
    u=ue(allow_game=True);out=OUT/"reference"/args.label;out.mkdir(parents=True,exist_ok=True)
    env=dict(os.environ,RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
             RENDERLAB_M7_REFERENCE_OUT=str(out),VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation")
    env.pop("VK_INSTANCE_LAYERS",None)
    env.pop("RENDERLAB_M7_SEED",None)
    if args.seed is not None:
        if not 0<=args.seed<2**32:raise ValueError("reference seed must fit u32")
        env["RENDERLAB_M7_SEED"]=str(args.seed)
    env.pop("RENDERLAB_M7_SAMPLES_PER_DISPATCH",None)
    if args.dispatch_samples is not None:
        if args.dispatch_samples<1:raise ValueError("dispatch samples must be positive")
        env["RENDERLAB_M7_SAMPLES_PER_DISPATCH"]=str(args.dispatch_samples)
    env.pop("RENDERLAB_M7_MAX_DEPTH",None)
    if getattr(args,"depth",None) is not None:
        if not 1<=args.depth<=1024:raise ValueError("reference depth must be 1-1024")
        env["RENDERLAB_M7_MAX_DEPTH"]=str(args.depth)
    test="reference::tests::gpu_reference_analytic"
    if args.benches:
        env["RENDERLAB_M7_BENCHES"]=";".join(str((OUT if (OUT/"scenes"/x.split("@")[0]/"scene.json").exists() else BASE)/"scenes"/x.split("@")[0])+"@"+x.split("@")[1] for x in args.benches)
        test="reference::tests::gpu_reference_benches"
    if args.geometry:env["RENDERLAB_M7_CAPTURE_GEOMETRY"]="1"
    if args.replay_loaded:
        if not args.benches:raise ValueError("loaded replay requires --benches")
        test="reference::tests::gpu_reference_replay_loaded"
    if getattr(args,"s2g_regression",False):
        if not args.benches:raise ValueError("S2G regression requires --benches")
        test="reference::tests::gpu_reference_s2g_replay_regression"
    if args.manifest:
        env["RENDERLAB_M7_QUARRY_REPLAY_POLICY"]=getattr(args,"replay_policy","strict")
        if getattr(args,"canonical",False):
            for key in ("RENDERLAB_M7_QUARRY_REPLAY_MODE","RENDERLAB_M7_QUARRY_REPLAY_COUNT","RENDERLAB_M7_SHADER_OVERRIDE"):
                env.pop(key,None)
        env["RENDERLAB_M7_QUARRY_MANIFEST"]=str(args.manifest)
        env["RENDERLAB_M7_REFERENCE_BATCH"]=str(min(args.batch_frames,3) if game_state()=="relaxed" else args.batch_frames)
        if all("alias_of" not in f for f in json.loads(args.manifest.read_text())["frames"]):
            env["RENDERLAB_M7_DEFER_REFERENCE_COMPACT"]="1"
        if game_state()=="relaxed":env["RENDERLAB_M7_REFERENCE_MAX_FRAME_SECONDS"]="45"
        test="reference::quarry_tests::gpu_reference_quarry"
    with u.lease("reference-"+args.label,args.minutes) as owner:
        process=out/("process-"+owner["token"][:8]);process.mkdir(exist_ok=True)
        u.save(process/"reference-recipe.json",dict(binary_sha256=u.sha(BINARY),build=json.loads((OUT/"build.json").read_text()),
            environment={k:v for k,v in env.items() if k.startswith("RENDERLAB_M7_")},
            manifest_sha256=u.sha(args.manifest) if args.manifest else None,game_protocol=game_state(),
            foreign_cpu_at_acquire=owner.get("foreign_cpu_readiness"),timing_eligible_for_claim=False))
        try:
            u.guarded([BINARY,test,"--exact","--ignored","--nocapture","--test-threads=1"],process,
                timeout=min(600,max(30,int(u.remaining_seconds(owner))-15)),cwd=REPO,env=env)
        except (RuntimeError,SystemExit):
            path=process/"process.json";preflight=process/"preflight.json"
            record=json.loads(path.read_text()) if path.exists() else {}
            initial=json.loads(preflight.read_text()) if preflight.exists() else {}
            minimum=record.get("minimum_available_gib",initial.get("available_gib"))
            problems=json.dumps([record.get("failures",[]),initial.get("monitor_error")]).lower()
            if (minimum is not None and minimum<4) or "monitor" in problems or "timeout" in problems:
                u.save(OUT/"resource-pause.json",dict(reason="reference resource/monitor failure",run=str(process),
                    utc=dt.datetime.now(dt.timezone.utc).isoformat(),policy="TR0 b9760bc: wait outside lock for 60 Cargo-free seconds"))
            raise
    if args.manifest:
        # Match TR0's bench flow: GPU child/context ends before CPU archival.
        for receipt in out.glob("*.compact-pending.json"):
            name=receipt.name.removesuffix(".compact-pending.json")
            compact_env=dict(os.environ);compact_env.pop("RENDERLAB_M7_DEFER_REFERENCE_COMPACT",None)
            subprocess.run(["py","-3","-B",str(REPO/"scripts/m7-u0b-score.py"),"compact",str(out),name],
                cwd=REPO,env=compact_env,check=True,creationflags=subprocess.CREATE_NO_WINDOW)


def s4(args):
    """Bound process lifetime while preserving the full 65,536-spp production budget."""
    combined=OUT/"reference"/args.label/"combined/S4-interior.npz"
    if combined.exists():
        metadata=json.loads(combined.with_suffix(".json").read_text())
        if metadata["samples"]!=65536 or ue().sha(combined)!=metadata["archive_sha256"]:
            raise ValueError("existing S4 production identity/budget mismatch")
        print("S4 combined production already verified",flush=True);return
    guard=OUT/"scenes/SV-seed-guard";guard.mkdir(parents=True,exist_ok=True)
    scene=json.loads((OUT/"scenes/SV/scene.json").read_text());scene["size"]=[8,8]
    (guard/"scene.json").write_text(json.dumps(scene,separators=(",",":")))
    inputs=[]
    for i in range(16):
        label=args.label+f"/seed-{i:02}";root=OUT/"reference"/label;archive=root/"S4-interior.npz"
        if not archive.exists():
            reference(argparse.Namespace(label=label,benches=["SV-seed-guard@16","S4@4096"],
                minutes=3,geometry=False,replay_loaded=False,manifest=None,batch_frames=1,
                dispatch_samples=1,
                seed=(0x4d375530+i*7919)%(2**32)))
            processes=list(root.glob("process-*/process.json"))
            process=max(processes,key=lambda p:p.stat().st_mtime)
            proof=json.loads(process.read_text())
            if proof["exit_code"] or proof["failures"]:raise RuntimeError("S4 batch process did not pass")
            metadata=root/"S4-interior.json";value=json.loads(metadata.read_text())
            value|=dict(production_process=str(process),production_exit_code=0)
            metadata.write_text(json.dumps(value,indent=2))
            subprocess.run(["py","-3","-B",str(REPO/"scripts/m7-u0b-score.py"),"compact",str(root),"S4-interior"],
                           cwd=REPO,check=True,creationflags=subprocess.CREATE_NO_WINDOW)
            # Other workers poll every two seconds. Leave an explicit unlocked gap.
            time.sleep(3)
        inputs.append(str(archive))
    subprocess.run(["py","-3","-B",str(REPO/"scripts/m7-u0b-score.py"),"combine",
                    str(OUT/"reference"/args.label/"combined"),"S4-interior",*inputs],
                   cwd=REPO,check=True,creationflags=subprocess.CREATE_NO_WINDOW)


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


def prepare_sv(args):
    u=ue();out=OUT/"import/sv-sky";out.mkdir(parents=True,exist_ok=True)
    with u.lease("sv-sky-import",4):
        u.guarded([u.EDITOR,u.PROJECT,*u.UE_FLAGS,*u.COMMON_INI,"-NullRHI","-run=pythonscript",
            "-script="+str(REPO/"scripts/m7-u0b-sv-prepare.py"),"-abslog="+str(out/"ue.log")],
            out,timeout=220,env=dict(os.environ,RENDERLAB_U0B_ROOT=str(OUT)))


def cook(args):
    u=ue();out=OUT/"cook"/args.label;out.mkdir(parents=True,exist_ok=True)
    maps=args.maps or [f"/Game/M6Quarry/View{i}" for i in range(3)]+[f"/Game/M7U0b/{b}/Bench" for b in ("SV","S4","S5","S2G")]
    archive=OUT/(args.archive or "ue-package")
    with u.lease("cook-"+args.label,args.minutes) as owner:
        u.guarded([u.EDITOR,u.PROJECT,*u.UE_FLAGS,*u.COMMON_INI,"-NullRHI","-run=cook","-TargetPlatform=Windows",
                   "-Map="+"+".join(maps),"-unversioned","-skipeditorcontent","-cookcultures=en","-NoZenAutoLaunch",
                   "-abslog="+str(out/"cook/ue.log")],out/"cook",timeout=int(u.remaining_seconds(owner))-300)
        u.guarded([os.environ["COMSPEC"],"/d","/c",u.UAT,"BuildCookRun",f"-project={u.PROJECT}","-nop4","-skipbuild",
                   "-skipcook","-stage","-pak","-archive",f"-archivedirectory={archive}","-clientconfig=Development",
                   "-targetplatform=Win64","-unattended","-utf8output","-nocompile","-nocompileuat","-installed"],
                  out/"package",timeout=int(u.remaining_seconds(owner))-10)
    u.save(out/"identity.json",dict(executable_sha256=u.sha(archive/"Windows/Engine/Binaries/Win64/UnrealGame.exe"),maps=maps,
        paks={str(p.relative_to(archive)):u.sha(p) for p in archive.rglob("*.pak")},
        utocs={str(p.relative_to(archive)):u.sha(p) for p in archive.rglob("*.utoc")}))


@contextlib.contextmanager
def quiet_desktop(u,root):
    """Pause the live wallpaper through its supported watched setting, then restore only that
    setting. Do not stop applications or overwrite concurrent changes to other preferences."""
    root=root/"windows"/dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    root.mkdir(parents=True)
    u.preflight(root/"quiet-preflight.json",wait_seconds=0)
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
        yield root
    finally:
        u.save(root/"quiet-end.json",dict(utc=dt.datetime.now(dt.timezone.utc).isoformat(),game_protocol=game_state()))
        if original is not None:
            current=json.loads(path.read_text(encoding="utf-8-sig"))
            current["paused"]=original;path.write_text(json.dumps(current,indent=2)+"\n",encoding="utf-8")


def study(args):
    u=ue(allow_game=not args.quiet);root=OUT/"study"/args.label;root.mkdir(parents=True,exist_ok=True)
    u.WAIT_FOR_FOREIGN_CPU_CLEAR=args.quiet
    capture_sync=args.hdr and not args.quiet
    if capture_sync:
        u.UE_FLAGS += ["-UseFixedTimeStep","-FPS=60"]
        for c in u.CONFIGS.values():
            c["commands"] += ["r.OneFrameThreadLag 0"]
            c["required"] += [("r.OneFrameThreadLag","0")]
    configs=args.configs or list(u.CONFIGS)
    benches=args.benches or ["quarry"]
    u.schedule_line(f"[m7-u0b gap request] quiet={args.quiet} asks for an exclusive {args.minutes}-minute gap: {configs}, benches={benches}; no Cargo, hidden/offscreen, wallpaper paused only for quiet timings.")
    with u.lease("study-"+args.label,args.minutes) as owner:
        if args.quiet:u.schedule_line(f"[m7-u0b QUIET window] label={args.label} token={owner['token']} expected_minutes={args.minutes}; exclusive GPU, wallpaper paused, no other lease/compiler/heavy build until owned release.")
        with (quiet_desktop(u,root) if args.quiet else contextlib.nullcontext()) as quiet_window:
            for visit in range(args.repeats):
                for bench in benches:
                    u.PACKAGE_DIR=(BASE if bench=="quarry" else OUT)/"ue-package"
                    u.PACKAGE=u.PACKAGE_DIR/"Windows/Engine/Binaries/Win64/UnrealGame.exe"
                    for config in configs if visit%2==0 else reversed(configs):
                        if u.remaining_seconds(owner)<100:
                            print("Short lease complete; resume unfinished cases with the same command",flush=True);return
                        bench_label=bench+("-"+args.bench_pose if args.bench_pose else "")
                        out=root/f"{bench_label}-{config}-{args.workload}-{visit}"
                        attempts=[out,*root.glob("retry*-"+out.name)]
                        if any((p/"summary.json").exists() and (p/"readback.json").exists() and (p/"process.json").exists() and
                               json.loads((p/"process.json").read_text())["exit_code"]==0 and
                               not json.loads((p/"process.json").read_text()).get("failures") and
                               json.loads((p/"readback.json").read_text())["all_ok"] and
                               (not args.hdr or (p/"camera-readback.json").exists()) and
                               (not args.hdr or args.workload!="static" or (p/"native.json").exists()) for p in attempts):continue
                        if out.exists() and any(out.iterdir()):
                            number=1
                            while (root/f"retry{number}-{out.name}").exists():number+=1
                            out=root/f"retry{number}-{out.name}"
                        out.mkdir(exist_ok=True)
                        extra=[];events=[]
                        if args.hdr:
                            extra=["r.DumpGPU.Upload 0","r.DumpGPU.Explore 0","r.DumpGPU.Stream 0","r.DumpGPU.Texture 2",
                                   "r.DumpGPU.Buffer 2","r.DumpGPU.PassParameters 1","r.DumpGPU.FrameCount 1","r.DumpGPU.Root Tonemap*",
                                   f"r.DumpGPU.Directory {(out/'hdr').as_posix()}"]
                            events=[(178,"getall PlayerCameraManager CameraCachePrivate"),(180,"DumpGPU")]
                        if args.workload!="static":
                            # UE's own command line is limited to 16 KiB. Keep the 160
                            # per-frame exec paths short even for long study labels.
                            macros=OUT/"m"/hashlib.sha256(str(out).encode()).hexdigest()[:10]
                            macros.mkdir(parents=True,exist_ok=True)
                            events=[e for e in u.workload_events(macros,args.workload) if not e[1].startswith("HighResShot")]
                            if args.hdr and args.workload=="pan":
                                for f in (97,112,128,159):
                                    events += [(f-2,f"r.DumpGPU.Directory {(out/f'hdr-{f:03}').as_posix()}"),(f-1,"DumpGPU"),(f+1,"getall PlayerCameraManager CameraCachePrivate")]
                        scene=None
                        if bench!="quarry":
                            scene=json.loads((BASE/"scenes"/bench/"scene.json").read_text())
                            pose=next(p for p in scene["poses"] if p["name"]==args.bench_pose) if args.bench_pose else scene["poses"][0]
                            if args.bench_pose:extra.append(u.macro(out,"registered-pose",pose_commands(pose)))
                            # U0's HRT configuration sets the quarry's 4.58-degree disk. These
                            # benches register a 0.5329-degree disk, so restore it after the preset.
                            angle=math.degrees(2*scene["sun"]["angular_radius"]) if scene["sun"] else 0
                            extra += [f"set DirectionalLightComponent LightSourceAngle {angle:.9f}",
                                      "r.Shadow.CacheWPOPrimitives 1","r.Shadow.CacheWPOPrimitives 0",*u.READBACK]
                        u.launch_ue(out,config,args.workload,extra=extra,events=events,frames=220 if args.hdr else 160,
                                    level=None if bench=="quarry" else f"/Game/M7U0b/{bench}/Bench")
                        ok=u.readback(out,config,u.scalability_expectations())
                        if scene is not None:
                            log=(out/"ue.log").read_text(encoding="utf-8-sig",errors="replace")
                            actual=[float(v) for v in re.findall(r"\.LightSourceAngle = ([0-9.eE+-]+)",log)]
                            proof=dict(scene_sha256=u.sha(BASE/"scenes"/bench/"scene.json"),expected_sun_angle_degrees=angle,
                                       actual_sun_angle_degrees=actual[-1] if actual else None,
                                       sun_angle_ok=bool(actual) and abs(actual[-1]-angle)<2e-5)
                            u.save(out/"scene-readback.json",proof);ok=ok and proof["sun_angle_ok"]
                        if args.hdr and args.workload=="static":
                            if scene is None:
                                expected=json.loads((OUT/"quarry-static.json").read_text())["frames"][0]
                                expected=dict(expected["open"],fov_y_radians=expected["fov_y"])
                            else:
                                expected=dict(pose,fov_y_radians=math.radians(scene["fov_y_degrees"]))
                            camera=check_camera((out/"ue.log").read_text(encoding="utf-8-sig",errors="replace"),[expected])
                            u.save(out/"camera-readback.json",dict(frames=camera,all_ok=True))
                        summary=u.PROFILE.summarize(out/"profile.csv");summary["pass_costs_async_off"]=u.pass_costs(out)
                        summary["timing_eligible_for_claim"]=bool(args.quiet and game_state()=="released")
                        summary["game_protocol"]=game_state()
                        summary["quiet_window"]=str(quiet_window) if quiet_window else None
                        summary["capture_only_synchronization"]=capture_sync
                        summary["foreign_cpu_at_acquire"]=owner.get("foreign_cpu_readiness")
                        u.save(out/"summary.json",summary)
                        print(json.dumps(dict(case=out.name,readback_ok=ok,gpu=summary["statistics"]["GPUTime"])),flush=True)
                        if not ok:raise RuntimeError("readback failed "+str(out))
                        if args.hdr and args.workload=="static":
                            # Compact each bench immediately so a full matrix never
                            # accumulates dozens of large regenerable GPU dumps.
                            with (out/"decode.log").open("w") as log:
                                subprocess.run(["py","-3","-B",str(REPO/"scripts/m7-u0b-score.py"),"decode",
                                    str(out/"hdr"),str(out/"native.npz"),"--stops",str(int(scene["exposure_stops"]) if scene else -14)],
                                    cwd=REPO,stdout=log,check=True,creationflags=subprocess.CREATE_NO_WINDOW)


def matrix(args):
    configs=list(ue().CONFIGS)
    for bench in ("SV","S4","S5","S2G"):
        for i in range(0,len(configs),3):
            root=OUT/"study"/args.label
            def complete(config):
                name=f"{bench}-{config}-static-0"
                return any((p/"summary.json").exists() and (p/"native.json").exists() and
                    (p/"camera-readback.json").exists() and (p/"readback.json").exists() and
                    json.loads((p/"readback.json").read_text())["all_ok"]
                    for p in [root/name,*root.glob("retry*-"+name)])
            while not all(complete(c) for c in configs[i:i+3]):
                before=sum(complete(c) for c in configs[i:i+3])
                study(argparse.Namespace(label=args.label,configs=configs[i:i+3],benches=[bench],
                    repeats=1,quiet=False,hdr=True,workload="static",bench_pose=None,minutes=5))
                if sum(complete(c) for c in configs[i:i+3])==before:raise RuntimeError("bench chunk made no progress")
                time.sleep(3)


def sequence(args):
    """Complete a registered canonical manifest in bounded, alternating leases."""
    manifest=json.loads(args.manifest.read_text());root=OUT/"reference"/args.label
    names=[f["name"] for f in manifest["frames"] if "alias_of" not in f]
    while True:
        before={name for name in names if (root/(name+".npz")).exists()}
        if len(before)==len(names):break
        reference(argparse.Namespace(label=args.label,manifest=args.manifest,batch_frames=1,
            minutes=11,benches=None,geometry=False,replay_loaded=False,seed=None,dispatch_samples=None,depth=None,
            replay_policy="bounded-as-v1",canonical=True))
        for name in names:
            if name in before or not (root/(name+".json")).exists():continue
            if not (root/(name+".npz")).exists():
                env=dict(os.environ);env.pop("RENDERLAB_M7_DEFER_REFERENCE_COMPACT",None)
                subprocess.run(["py","-3","-B",str(REPO/"scripts/m7-u0b-score.py"),"compact",str(root),name],
                    cwd=REPO,env=env,check=True,creationflags=subprocess.CREATE_NO_WINDOW)
        after={name for name in names if (root/(name+".npz")).exists()}
        if after==before:raise RuntimeError("reference batch made no progress")
        for name in after-before:
            meta=json.loads((root/(name+".json")).read_text())
            if not load("m7-u0b-score").qualified_replay(meta):
                raise RuntimeError("new quarry reference did not pass its replay policy: "+name)
        print(f"sequence references {len(after)}/{len(names)}",flush=True)
        time.sleep(3)


def cleanup_native(case):
    """Score summaries retain identities; keep inspected PNGs instead of large raw HDR."""
    ledger=case/"native-cleanup.json"
    deleted=json.loads(ledger.read_text()) if ledger.exists() else {}
    for path in case.glob("native*.npz"):
        if path.stat().st_size>5_000_000:
            deleted[path.name]=dict(bytes=path.stat().st_size,sha256=ue().sha(path));path.unlink()
    ue().save(ledger,deleted)


def floors(args):
    """Complete U0's static matrix and bounded, per-config native motion captures."""
    configs=list(ue().CONFIGS);static_root=OUT/"study/floor-static"
    def qualified():
        result={}
        if not static_root.exists():return result
        for case in sorted(static_root.iterdir()):
            if not case.is_dir() or any(not (case/f).exists() for f in
                    ("native.json","camera-readback.json","readback.json","process.json")):continue
            runtime=json.loads((case/"readback.json").read_text())
            process=json.loads((case/"process.json").read_text())
            if (runtime["all_ok"] and json.loads((case/"camera-readback.json").read_text())["all_ok"]
                    and process["exit_code"]==0 and not process.get("failures")):
                result[runtime["config"]]=case
        return result
    while True:
        cases=qualified();missing=[c for c in configs if c not in cases]
        if not missing:break
        before=len(cases)
        study(argparse.Namespace(label="floor-static",configs=missing[:3],benches=["quarry"],
            repeats=1,quiet=False,hdr=True,workload="static",bench_pose=None,minutes=3))
        if len(qualified())<=before:raise RuntimeError("static floor chunk made no progress")
        time.sleep(3)
    score=load("m7-u0b-score")
    summary=static_root/"physical-static-floor.json"
    if not summary.exists() or {r["config"] for r in json.loads(summary.read_text())["frames"]}!=set(configs):
        score.static_floor(argparse.Namespace(root=static_root,cases=list(qualified().values()),
            reference=OUT/"reference/quarry-final/quarry-p0.npz",stops=-14))
    if {r["config"] for r in json.loads(summary.read_text())["frames"]}!=set(configs):
        raise RuntimeError("static physical floor does not cover every U0 configuration")
    for case in static_root.iterdir():
        if case.is_dir() and (case/"native.json").exists():cleanup_native(case)
    for sequence_name,label,selected in (("q-walk","lumen",["ue-sw-e","ue-hw-e","ue-sw-h","ue-hw-h"]),
            ("alternation","variants",[c for c in configs if ue().CONFIGS[c]["lumen"]])):
        root=OUT/"walk"/label
        for config in selected:
            case=root/config
            if (case/"physical-floor.json").exists():continue
            walk(argparse.Namespace(label=label,configs=[config],sequence=sequence_name))
            score.floor(argparse.Namespace(root=root,reference=OUT/"reference/quarry-final",stops=-14,
                configs=[config],selection=OUT/"quarry-t1b-witnesses.json" if sequence_name=="q-walk" else None))
            result=json.loads((case/"physical-floor.json").read_text())
            if not result["complete"]:raise RuntimeError("missing registered physical references")
            cleanup_native(case)


def finish(args):
    """One sequential controller; no competing U0b acquisitions or hidden GPU children."""
    sv=load("m7-u0b-sv")
    sv.capture(argparse.Namespace(label="b",configs=["sw-mesh","off"],timing=False,async_off=False))
    sv.audit(argparse.Namespace(label="b"))
    if not (OUT/"sv-sky/b/g1-comparator.json").exists():sv.score(argparse.Namespace(label="b"))
    sv.costs(argparse.Namespace(label="b"))
    for label,async_off in (("timing",False),("timing-async-off",True)):
        sv.capture(argparse.Namespace(label=label,configs=["sw-mesh","hw"],timing=True,async_off=async_off))
        sv.costs(argparse.Namespace(label=label))
    matrix(argparse.Namespace(label="bench-matrix"))
    bench_summary=OUT/"bench-captures.json"
    if not bench_summary.exists() or not json.loads(bench_summary.read_text())["complete"]:
        load("m7-u0b-report").benches(argparse.Namespace(root=OUT/"study/bench-matrix"))
    sequence(argparse.Namespace(label="quarry-final",manifest=OUT/"quarry-unit-sequence.json"))
    load("m7-u0b-report").aliases(argparse.Namespace(manifest=OUT/"quarry.json"))
    # These two original attribution processes overlapped stillkit's foreign build.
    # Timing readiness waits for 60 observed seconds without its Cargo/rustc jobs.
    sv.capture(argparse.Namespace(label="timing-async-off-retry",configs=["hw"],timing=True,async_off=True))
    sv.costs(argparse.Namespace(label="timing-async-off-retry"))
    floors(args)


def pose_commands(p):
    x,y,z=p["eye"];dx,dy,dz=[b-a for a,b in zip(p["eye"],p["target"])]
    pitch=math.degrees(math.atan2(dy,math.hypot(dx,dz)));yaw=math.degrees(math.atan2(dx,-dz))
    return [f'ke CameraActor K2_SetActorLocation "(X={-100*z:.9f},Y={100*x:.9f},Z={100*y:.9f})" false () false',
            f'ke CameraActor K2_SetActorRotation "(Pitch={pitch:.9f},Yaw={yaw:.9f},Roll=0)" false']


def check_camera(log,frames):
    rows=re.findall(r"CameraCachePrivate = .*?Location=\(X=([^,]+),Y=([^,]+),Z=([^\)]+)\),Rotation=\(Pitch=([^,]+),Yaw=([^,]+),Roll=([^\)]+)\),FOV=([^,]+)",log)
    if len(rows)!=len(frames):raise RuntimeError(f"camera readback coverage {len(rows)} != {len(frames)}")
    checks=[]
    for i,(row,p) in enumerate(zip(rows,frames)):
        x,y,z,pitch,yaw,roll,fov=map(float,row);a,b=map(math.radians,(pitch,yaw))
        eye=[y/100,z/100,-x/100]
        actual=[math.cos(a)*math.sin(b),math.sin(a),-math.cos(a)*math.cos(b)]
        d=[v-e for v,e in zip(p["target"],p["eye"])];length=math.sqrt(sum(v*v for v in d));expected=[v/length for v in d]
        error=max(abs(e-v) for e,v in zip(eye,p["eye"]))
        direction_error=max(abs(e-v) for e,v in zip(expected,actual))
        expected_fov=math.degrees(2*math.atan(math.tan(p["fov_y_radians"]/2)*1920/1080))
        if error>1e-6 or direction_error>1e-6 or abs(roll)>1e-5 or abs(fov-expected_fov)>2e-4:
            raise RuntimeError(f"camera mismatch {i}: eye{error}, direction{direction_error}, FOV{fov}/{expected_fov}")
        checks.append(dict(frame=i,eye_error_m=error,direction_error=direction_error,fov_degrees=fov,expected_fov_degrees=expected_fov))
    return checks


def walk(args):
    """Contiguous native Lumen capture on the exact registered Q-walk. The GPU lease is
    released before CPU HDR decoding, so other workers can use that time."""
    u=ue(allow_game=True);root=OUT/"walk"/args.label;root.mkdir(parents=True,exist_ok=True)
    u.UE_FLAGS += ["-UseFixedTimeStep","-FPS=60"]
    for config in args.configs:
        u.CONFIGS[config]["commands"] += ["r.OneFrameThreadLag 0"]
        u.CONFIGS[config]["required"] += [("r.OneFrameThreadLag","0")]
    source=Path("E:/renderlab-work/t1-gpu/sequences/q-walk.json") if args.sequence=="q-walk" else OUT/"quarry-static.json"
    registered=json.loads(source.read_text())["frames"]
    if args.sequence=="q-walk":frames=registered
    else:
        poses=[dict(f["open"],fov_y_radians=f["fov_y"],phase="camera-alternation")
               for f in registered if not f["sky_only"]]
        frames=[dict(poses[i%2],frame=i) for i in range(64)]
    macros=OUT/"m"/hashlib.sha256(str(root).encode()).hexdigest()[:8];macros.mkdir(parents=True,exist_ok=True)
    events=[]
    # Alternation has 96 moving warm-up frames, matching U0's cost workload.
    for i in range(-96 if args.sequence=="alternation" else 0,len(frames)):
        p=frames[i%2] if i<0 else frames[i]
        text=(["getall PlayerCameraManager CameraCachePrivate"] if i else [])+pose_commands(p)
        if i<0:text=pose_commands(p)
        events.append((96+i,u.macro(macros,f"m{i}",text)))
    arm=(["r.ResetViewState"] if args.sequence=="q-walk" else [])+["DumpGPU"]
    events += [(95,u.macro(macros,"arm",arm)),(96+len(frames),"getall PlayerCameraManager CameraCachePrivate")]
    for config in args.configs:
        out=root/config;out.mkdir(exist_ok=True)
        if (out/"native.json").exists():continue
        extra=["r.DumpGPU.Upload 0","r.DumpGPU.Explore 0","r.DumpGPU.Stream 0","r.DumpGPU.Texture 2","r.DumpGPU.Buffer 2",
               "r.DumpGPU.PassParameters 1",f"r.DumpGPU.FrameCount {len(frames)}","r.DumpGPU.Root Tonemap*",f"r.DumpGPU.Directory {(out/'hdr').as_posix()}"]
        original_guarded=u.guarded
        def bounded_command(command,*a,**kw):
            length=len(subprocess.list2cmdline([str(c) for c in command]))
            if length>=16000:raise ValueError(f"UE's 16-KiB FCommandLine bound: {length}")
            return original_guarded(command,*a,**kw)
        u.guarded=bounded_command
        with u.lease("walk-"+args.label+"-"+config,8):
            u.launch_ue(out,config,"static",extra=extra,events=events,frames=96+len(frames)+80)
            if not u.readback(out,config,u.scalability_expectations()):raise RuntimeError("walk readback failed")
        u.guarded=original_guarded
        log=(out/"ue.log").read_text(encoding="utf-8-sig",errors="replace")
        checks=check_camera(log,frames)
        u.save(out/"camera-readback.json",dict(sequence_sha256=u.sha(source),frames=checks,all_ok=True))
        u.save(out/"recipe.json",dict(sequence=str(source),sequence_name=args.sequence,sequence_sha256=u.sha(source),frames=frames,
                shutter="zero",capture_only=True,synchronous_render=True,fixed_delta_seconds=1/60,
                reset_before_arm=args.sequence=="q-walk",first_csv_frame=96,arm_csv_frame=95,
                package_executable_sha256=u.sha(u.PACKAGE),config=config))
        subprocess.run(["py","-3","-B",str(REPO/"scripts/m7-u0b-score.py"),"decode",str(out/"hdr"),str(out/"native.npz")],check=True,
                       cwd=REPO,stdout=(out/"decode.log").open("w"),creationflags=subprocess.CREATE_NO_WINDOW)
        print("decoded native walk",config,flush=True)
        time.sleep(3)


def main():
    p=argparse.ArgumentParser(description=__doc__);sub=p.add_subparsers(dest="cmd",required=True)
    sub.add_parser("build");sub.add_parser("build_gap");sub.add_parser("prepare");sub.add_parser("prepare_sv")
    q=sub.add_parser("matrix");q.add_argument("label",default="bench-matrix",nargs="?")
    q=sub.add_parser("sequence");q.add_argument("label");q.add_argument("manifest",type=Path)
    sub.add_parser("floors");sub.add_parser("finish")
    q=sub.add_parser("s4");q.add_argument("label",default="s4-64k",nargs="?")
    q=sub.add_parser("cook");q.add_argument("label");q.add_argument("--maps",nargs="+")
    q.add_argument("--archive");q.add_argument("--minutes",type=int,default=45)
    q=sub.add_parser("reference");q.add_argument("label");q.add_argument("--minutes",type=float,default=15)
    q.add_argument("--seed",type=int)
    q.add_argument("--dispatch-samples",type=int)
    q.add_argument("--depth",type=int)
    q.add_argument("--benches",nargs="+");q.add_argument("--manifest",type=Path)
    q.add_argument("--batch-frames",type=int,default=16)
    q.add_argument("--replay-policy",choices=("strict","bounded-as-v1"),default="strict")
    q.add_argument("--geometry",action="store_true");q.add_argument("--replay-loaded",action="store_true")
    q.add_argument("--s2g-regression",action="store_true")
    q=sub.add_parser("study");q.add_argument("label");q.add_argument("--minutes",type=int,default=30)
    q.add_argument("--configs",nargs="+");q.add_argument("--benches",nargs="+");q.add_argument("--repeats",type=int,default=1)
    q.add_argument("--bench-pose")
    q.add_argument("--quiet",action="store_true");q.add_argument("--hdr",action="store_true")
    q.add_argument("--workload",choices=("static","camera","sun","pan"),default="static")
    q=sub.add_parser("walk");q.add_argument("label");q.add_argument("--configs",nargs="+",default=["ue-sw-e","ue-hw-e","ue-sw-h","ue-hw-h"])
    q.add_argument("--sequence",choices=("q-walk","alternation"),default="q-walk")
    args=p.parse_args();globals()[args.cmd](args)


if __name__=="__main__":main()
