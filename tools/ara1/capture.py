"""Shared recording implementation, executed only inside ARA1's guarded lease."""
import argparse
import csv
import ctypes
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import threading
import time

from lease import check_order

REPO=Path(__file__).resolve().parents[2]
FLAGS=getattr(subprocess,"CREATE_NO_WINDOW",0)


class WorkingSet:
    """Read only this child process; Windows maintains the cumulative working-set peak."""
    class Counters(ctypes.Structure):
        _fields_=[("cb",ctypes.c_ulong),("PageFaultCount",ctypes.c_ulong)]+[(name,ctypes.c_size_t) for name in (
            "PeakWorkingSetSize","WorkingSetSize","QuotaPeakPagedPoolUsage","QuotaPagedPoolUsage",
            "QuotaPeakNonPagedPoolUsage","QuotaNonPagedPoolUsage","PagefileUsage","PeakPagefileUsage","PrivateUsage")]

    def __init__(self,pid):
        self.kernel=ctypes.WinDLL("kernel32",use_last_error=True)
        self.kernel.OpenProcess.restype=ctypes.c_void_p
        self.kernel.OpenProcess.argtypes=[ctypes.c_ulong,ctypes.c_int,ctypes.c_ulong]
        self.kernel.CloseHandle.argtypes=[ctypes.c_void_p]
        self.psapi=ctypes.WinDLL("psapi",use_last_error=True)
        self.psapi.GetProcessMemoryInfo.argtypes=[ctypes.c_void_p,ctypes.POINTER(self.Counters),ctypes.c_ulong]
        self.handle=self.kernel.OpenProcess(0x0400|0x0010,False,pid)
        if not self.handle:raise ctypes.WinError(ctypes.get_last_error())

    def sample(self):
        value=self.Counters();value.cb=ctypes.sizeof(value)
        if not self.psapi.GetProcessMemoryInfo(self.handle,ctypes.byref(value),value.cb):
            return dict(error=str(ctypes.WinError(ctypes.get_last_error())))
        return dict(working_set_bytes=value.WorkingSetSize,peak_working_set_bytes=value.PeakWorkingSetSize,
                    private_commit_bytes=value.PrivateUsage,peak_pagefile_bytes=value.PeakPagefileUsage)

    def close(self):
        if self.handle:self.kernel.CloseHandle(self.handle);self.handle=None


def digest(path):
    with path.open("rb") as f:return hashlib.file_digest(f,"sha256").hexdigest()


def command_output(command):
    p=subprocess.run(command,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,creationflags=FLAGS,timeout=20)
    return dict(command=command,exit_code=p.returncode,output=p.stdout.strip())


def ara_process_check():
    result=command_output(["tasklist","/FO","CSV","/NH"])
    if result["exit_code"]:raise RuntimeError("cannot check Ara renderer ownership")
    processes=[row for row in csv.reader(io.StringIO(result["output"])) if row and row[0].lower().startswith(("ara","snengine")) and row[0].lower().endswith(".exe")]
    if processes:raise RuntimeError("another Ara/snengine process is running: "+repr(processes))
    return processes


def identity(source):
    def git(*args):return subprocess.check_output(["git","-C",str(source),*args],text=True).strip()
    # A live edit to this measurement harness cannot alter a frozen child binary.
    # Ara is stricter: every tracked path in its checkout remains read-only.
    paths=['--','src','shaders','crates','Cargo.toml','Cargo.lock','rust-toolchain.toml'] if source.resolve()==REPO.resolve() else []
    return dict(source=str(source),sha=git("rev-parse","HEAD"),tracked_status=git("status","--porcelain","--untracked-files=no",*paths),
                mutation_scope='renderer/build inputs; harness sources separately hashed' if paths else 'entire Ara tracked checkout')


def require_lease():
    expected=os.environ.get("ARA1_EXPECTED_ORDER")
    if not expected:raise RuntimeError("run through tools/ara1/lease.py; no lease order token")
    check_order(expected)
    owner=json.loads(Path("D:/Coding/renderlab-work/gpu-slot.lock/owner.json").read_text())
    # P-SHC reuses devloop under its own n8 lease; keep ARA1's default owner.
    expected_owner=os.environ.get("ARA1_LEASE_OWNER","m7-ara1")
    if owner["owner"]!=expected_owner:raise RuntimeError("not inside "+expected_owner+" lease")
    return expected,owner


def sample_resources(stop,samples,process):
    while not stop.is_set():
        sample=dict(unix_seconds=time.time(),elapsed_seconds=time.monotonic()-samples[0]["monotonic_start"],pid=process.pid)
        sample["gpu"]=command_output(["nvidia-smi","--query-gpu=name,uuid,driver_version,memory.total,memory.used,utilization.gpu,temperature.gpu","--format=csv,noheader,nounits"])
        # WDDM often reports N/A through nvidia-smi. Keep that value, and query the
        # process-specific Windows dedicated-memory counter rather than calling board delta VRAM.
        ps="(Get-Counter '\\GPU Process Memory(*)\\Dedicated Usage' -ErrorAction SilentlyContinue).CounterSamples | Where-Object {$_.InstanceName -like ('pid_'+$env:ARA1_PID+'_*')} | Select-Object InstanceName,CookedValue | ConvertTo-Json -Compress"
        try:
            value=subprocess.run(["powershell","-NoProfile","-NonInteractive","-Command",ps],env=os.environ|{"ARA1_PID":str(process.pid)},capture_output=True,text=True,creationflags=FLAGS,timeout=15)
            sample["process_dedicated_counter"]=dict(exit_code=value.returncode,output=value.stdout.strip(),error=value.stderr.strip())
        except subprocess.TimeoutExpired:
            sample["process_dedicated_counter"]=dict(error="counter query timed out; VRAM unavailable for this sample")
        samples.append(sample)
        stop.wait(1)


def main(engine):
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--binary",type=Path,required=True)
    p.add_argument("--source",type=Path,required=True)
    p.add_argument("--scene",type=Path,required=True)
    p.add_argument("--mode",required=True)
    p.add_argument("--out",type=Path,required=True)
    p.add_argument("--bake-cache",type=Path)
    p.add_argument("--cache-from",type=Path,help="Ara only: seed the new GPU cache with a completed prior run; record exact payload identity")
    p.add_argument("--profile",choices=["dev","release"],required=True)
    p.add_argument("--validate",action="store_true",help="retain inherited Vulkan validation for diagnostic runs; release measurements normally remove it")
    p.add_argument("--contention",required=True,help="explicit context, e.g. shared-GPU functional dry-run")
    a=p.parse_args()
    expected,owner=require_lease()
    if engine=="ara":ara_process_check()
    if a.out.exists():raise RuntimeError("refusing to overwrite a capture directory")
    a.out.mkdir(parents=True)
    cache_payload=None
    if a.cache_from:
        if engine!='ara':raise ValueError('--cache-from applies only to Ara GPU caches')
        cache_payload={str(f.relative_to(a.cache_from)):dict(bytes=f.stat().st_size,sha256=digest(f)) for f in a.cache_from.rglob('*') if f.is_file()}
        shutil.copytree(a.cache_from,a.out/'gpu-cache')
    scene=json.loads(a.scene.read_text());asset=Path(scene["asset"])
    if not asset.is_absolute():asset=a.scene.parent/asset
    source=identity(a.source)
    record=dict(engine=engine,mode=a.mode,source=source,binary=str(a.binary),binary_sha256=digest(a.binary),
                harness_sources={str(f.relative_to(REPO)):digest(f) for f in sorted([*(REPO/"tools/ara1").glob("*"),*(REPO/"examples").glob("ara1_*.rs")]) if f.is_file()},
                scene=str(a.scene),scene_sha256=digest(a.scene),asset=str(asset),asset_sha256=digest(asset),
                resolution=scene["size"],profile=a.profile,contention=a.contention,platform=platform.platform(),
                cpu=platform.processor(),lease=owner,root_order=check_order(expected)["line"],
                hardware=command_output(["nvidia-smi","--query-gpu=name,uuid,driver_version,memory.total,power.limit","--format=csv,noheader,nounits"]),
                memory_scope="GPU board memory is separate from the per-process WDDM Dedicated Usage counter; unavailable counters stay unavailable")
    record['seeded_gpu_cache']=dict(source=str(a.cache_from),payload=cache_payload) if a.cache_from else None
    command=[str(a.binary),str(a.scene),a.mode,str(a.out)]
    if engine=="ara":
        if a.bake_cache is None:p.error("Ara needs --bake-cache (raster/hybrid require an acquired cache)")
        command.append(str(a.bake_cache))
    record["command"]=command
    samples=[dict(monotonic_start=time.monotonic(),unix_seconds=time.time(),gpu=command_output(["nvidia-smi","--query-gpu=memory.used,utilization.gpu","--format=csv,noheader,nounits"]))]
    stop=threading.Event()
    environment=os.environ|{"ARA_CONFIG_DIR":str(a.out/"profile"),"ARA_CRASH_DIR":str(a.out/"crashes")}
    removed={}
    if a.profile=='release' and not a.validate:
        for key in ('VK_INSTANCE_LAYERS','VK_LAYER_ENABLES'):
            if key in environment:removed[key]=environment.pop(key)
    record['gpu_environment']=dict(release_validation_requested=a.validate,removed_inherited=removed,
        effective={k:v for k,v in environment.items() if k.startswith(('VK_','WGPU_')) or k in (
            'ARA_CONFIG_DIR','ARA_CRASH_DIR','ARA_UPSCALER','ARA_REFLECTION_CAPTURES',
            'RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK','RENDERLAB_M7_SHADER_OVERRIDE')})
    start=time.perf_counter();ready=None
    with (a.out/"process.log").open("w",encoding="utf-8") as log:
        child=subprocess.Popen(command,cwd=REPO,env=environment,stdout=log,stderr=subprocess.STDOUT,creationflags=FLAGS)
        memory=WorkingSet(child.pid)
        startup_memory=dict(status="startup incomplete; last observation is a lower bound if guard terminates the process",pid=child.pid,
                            scope="child working set/private commit, excludes driver allocations in other processes and the Python guard")
        last_memory_write=0
        monitor=threading.Thread(target=sample_resources,args=(stop,samples,child));monitor.start()
        try:
            while child.poll() is None:
                if ready is None:
                    startup_memory.update(memory.sample(),elapsed_ms=(time.perf_counter()-start)*1000)
                    if (a.out/"first-image.ready").exists():
                        ready=(time.perf_counter()-start)*1000
                        startup_memory["status"]="first GPU readback observed; startup working-set peak captured"
                    if ready is not None or time.perf_counter()-last_memory_write>.2:
                        # Persist during startup so a guard-killed process retains a useful bound.
                        staged=a.out/"startup-memory.tmp"
                        staged.write_text(json.dumps(startup_memory,indent=2)+"\n")
                        staged.replace(a.out/"startup-memory.json");last_memory_write=time.perf_counter()
                # The enclosing N8 runner enforces games/memory/lease deadline.
                stop.wait(.025)
            if ready is None and (a.out/"first-image.ready").exists():ready=(time.perf_counter()-start)*1000
            record.update(exit_code=child.returncode,process_seconds=time.perf_counter()-start,launch_to_ready_file_ms=ready)
            if a.mode=="bake" and child.returncode==0:
                startup_memory.update(memory.sample(),status="probe acquisition completed; whole acquisition-process peak",
                                      elapsed_ms=(time.perf_counter()-start)*1000)
            elif ready is not None and startup_memory["status"].startswith("startup incomplete"):
                startup_memory.update(memory.sample(),status="first-image marker observed after exit; peak covers the short process, not startup alone")
            staged=a.out/"startup-memory.tmp"
            staged.write_text(json.dumps(startup_memory,indent=2)+"\n")
            staged.replace(a.out/"startup-memory.json")
        finally:stop.set();monitor.join();memory.close()
    record["startup_memory"]=startup_memory
    record["resources"]=samples
    counters=[]
    for sample in samples:
        raw=sample.get("process_dedicated_counter",{}).get("output","")
        if raw:
            values=json.loads(raw)
            values=values if isinstance(values,list) else [values]
            counters.append(sum(v["CookedValue"] for v in values))
    record["peak_process_dedicated_bytes"]=max(counters) if counters else None
    record["source_status_unchanged"]=source==identity(a.source)
    record["images"]={f.name:digest(f) for f in a.out.glob("frame-*.png")}
    if not record["exit_code"] and a.mode!="bake":
        expected={f"frame-{i:04}.png" for i,f in enumerate(scene["frames"]) if f["capture"]}
        if a.mode=="reference" and os.environ.get("ARA1_REFERENCE_INDICES"):
            selected={int(i) for i in os.environ["ARA1_REFERENCE_INDICES"].split(',')}
            expected={f"frame-{i:04}.png" for i in selected if scene["frames"][i]["capture"]}
        if set(record["images"])!=expected or not (a.out/"engine.json").exists():
            raise RuntimeError("successful process did not produce the registered frames and timing record")
    (a.out/"process.json").write_text(json.dumps(record,indent=2)+"\n")
    if not record["source_status_unchanged"]:raise RuntimeError("Ara/source changed during capture; do not compare this run")
    if record["exit_code"]:
        print((a.out/"process.log").read_text()[-6000:]);return record["exit_code"]
    print(json.dumps({k:record[k] for k in ("engine","mode","process_seconds","launch_to_ready_file_ms","images")}))
    return 0
