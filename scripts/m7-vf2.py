"""VF2: two-job CPU compilation; lib/images/timing inside n8's guarded GPU lease.
Evidence, staged builds and the disposable target are under D:/Coding/renderlab-work/vf2.
"""
import argparse
import importlib.util
import os
from pathlib import Path
import subprocess
import time
import datetime as dt
import json
import sys

REPO = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("vf1",REPO/"scripts/m7-vf1.py")
g = importlib.util.module_from_spec(spec)
spec.loader.exec_module(g)
g.OUT=Path("D:/Coding/renderlab-work/vf2")
g.OWNER="m7-vf2"
g.BIN=g.OUT/"bin"
SCHEDULE=Path("E:/renderlab-work/gpu-schedule.md")

def order_ready():
    """Every acquisition/step rereads root orders; new orders require explicit review."""
    schedule=SCHEDULE
    while True:
        try: lines=schedule.read_text(encoding="utf-8").splitlines();break
        except PermissionError: time.sleep(.05)
    orders=[(i,line) for i,line in enumerate(lines) if line.startswith("[root order]")]
    if not orders:return False
    position,order=orders[-1]
    config=g.OUT/"gpu-after.json"
    if not config.exists():return False
    gates=json.loads(config.read_text())
    if gates.get("acknowledged_order")!=order:return False
    later=lines[position+1:]
    def released(gate):
        if "line_prefix" in gate:
            return any(line.startswith(gate["line_prefix"]) for line in later)
        return sum(line.startswith(gate["owner"]+" ") and "window released" in line
                   and gate.get("label", "") in line for line in later)>=gate.get("count",1)
    return all(released(gate) for gate in gates["after"])


def gpu(label):
    checks,host=g.schedule()
    memory=g.load("m6-film-run")
    minimum=int(os.environ.get("VF2_MIN_RAM_MB","4096"))
    assert minimum>=4096
    g.OUT.mkdir(parents=True,exist_ok=True)
    with host.SCHEDULE.open("a",encoding="utf-8") as f:
        f.write(f"\n[m7-vf2 request] owner=m7-vf2 label={label} <=15 min headless; reads root orders before every lease/step, honours gpu-after.json. {dt.datetime.now(dt.timezone.utc).isoformat()}\n")
    deadline=time.monotonic()+5400
    attempt=0
    while time.monotonic()<deadline:
        if host.LOCK.exists():
            time.sleep(.05);continue
        if not order_ready() or memory.resource_sample()["available_mbytes"]<minimum or checks.hold_state(host)=="hold" or checks.quiet_active(host,any_lease=False):
            time.sleep(.5);continue
        attempt+=1
        os.environ.update(N8_OWNER="m7-vf2",N8_OUT=str(g.OUT))
        guard=g.load("n8-run")
        guard.s1.runner.PREFLIGHT_MEMORY_MBYTES=minimum
        guard.s1.runner.RUNTIME_MEMORY_MBYTES=2048
        original=guard.s1.runner.state
        def state(owned=()):
            if checks.hold_state(host)=="hold" or checks.quiet_active(host,any_lease=False):
                raise RuntimeError("game hold or foreign QUIET window")
            if not owned and not order_ready():
                raise RuntimeError("root order changed before launch")
            return original(owned)
        guard.s1.runner.state=state
        sys.argv=["n8-run","--label",label+"-"+str(attempt),"--minutes","15","--record-resources","--",
                  sys.executable,"-B",str(Path(__file__).resolve()),"batch",label]
        if not order_ready():
            time.sleep(.5);continue
        try:code=guard.s1.runner.main()
        finally:
            if guard.s1.claimed:
                with host.SCHEDULE.open("a",encoding="utf-8") as f:
                    f.write(f"\nm7-vf2 {label}-{attempt} window released {guard.s1.runner.utc()} (no owned GPU/build job remains).\n")
        if code!=3:raise SystemExit(code)
        time.sleep(.05)
    raise SystemExit("GPU wait deadline reached; no job remains")

def build():
    runner=g.load("m7-g2-run");runner.UNIT=g.OWNER;runner.OUT=g.OUT
    target=g.OUT/"target"
    command=["cargo","test","-p","renderlab","--lib","--no-run","--release","-j","2","--target-dir",str(target),"--message-format=json"]
    start=time.monotonic();code=runner.cargo(None,command,"build-"+os.environ.get("VF2_TAG","retained"))
    print("compile seconds",time.monotonic()-start,flush=True)
    if code:raise SystemExit((g.OUT/("build-"+os.environ.get("VF2_TAG","retained")+".err")).read_text(errors="replace")[-6000:])
    rows=[json.loads(l) for l in (g.OUT/("build-"+os.environ.get("VF2_TAG","retained")+".out")).read_text().splitlines() if l.startswith("{")]
    executable=next(r["executable"] for r in reversed(rows) if r.get("executable"))
    import shutil,hashlib
    g.BIN.mkdir(parents=True,exist_ok=True);dest=g.BIN/(os.environ.get("VF2_TAG","retained")+".exe");shutil.copy2(executable,dest)
    (dest.with_suffix(".json")).write_text(json.dumps({"sha256":hashlib.sha256(dest.read_bytes()).hexdigest(),"head":subprocess.check_output(["git","rev-parse","HEAD"],text=True).strip(),"diff":subprocess.check_output(["git","diff"],text=True),"compile_seconds":time.monotonic()-start,"source_sha256":{str(f):hashlib.sha256(f.read_bytes()).hexdigest() for d in ["src","shaders"] for f in (REPO/d).rglob("*") if f.is_file()}}))


def batch(label):
    owner=json.loads(Path("D:/Coding/renderlab-work/gpu-slot.lock/owner.json").read_text())
    assert owner["owner"]=="m7-vf2" and owner["label"].startswith(label+"-"), "VF2's owned n8 lease required"
    root=g.OUT/"images"/label
    root.mkdir(parents=True,exist_ok=True)
    import hashlib
    executable=g.BIN/(os.environ.get("VF2_TAG","retained")+".exe")
    identity=json.loads(executable.with_suffix(".json").read_text())
    assert hashlib.sha256(executable.read_bytes()).hexdigest()==identity["sha256"], "staged executable changed"
    (root/"build-identity.json").write_text(json.dumps(identity,indent=2))
    if os.environ.get("VF2_BASELINE_QUARRY")=="1":
        control=g.BIN/"baseline.exe"
        control_identity=json.loads(control.with_suffix(".json").read_text())
        assert hashlib.sha256(control.read_bytes()).hexdigest()==control_identity["sha256"], "baseline executable changed"
        (root/"baseline-identity.json").write_text(json.dumps(control_identity,indent=2))
    durations={}
    memory=g.load("m6-film-run")
    def run(name,command,extra=None,allow_failure=False,minimum=4096):
        if not order_ready():
            print("root order holds VF2 before step",name,flush=True)
            raise SystemExit(75)
        if memory.resource_sample()["available_mbytes"]<minimum:
            print("releasing before",name,"for RAM headroom",flush=True)
            raise SystemExit(75)
        env=dict(os.environ,RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",**(extra or {}))
        start=time.monotonic()
        with (root/(name+".log")).open("w") as log:
            code=subprocess.run(command,cwd=REPO,env=env,stdout=log,stderr=subprocess.STDOUT).returncode
        if code==0 and "--exact" in command and "\ntest result: ok. 1 passed; 0 failed;" not in (root/(name+".log")).read_text(errors="replace"):
            print("focused test did not execute exactly one passing test:",name,flush=True)
            code=1
        durations[name]={"exit":code,"seconds":time.monotonic()-start,"command":command,"settings":extra or {}}
        (root/"steps.json").write_text(json.dumps(durations,indent=2))
        print(name,"exit",code,flush=True)
        if code and not allow_failure:
            print((root/(name+".log")).read_text(errors="replace")[-6000:],flush=True)
            raise SystemExit(code)
    tests=str(g.BIN/(os.environ.get("VF2_TAG","retained")+".exe"))
    if os.environ.get("VF2_LIB", "1") != "0":
        run("lib",[tests,"--test-threads=2"],allow_failure=True)
    if os.environ.get("VF2_GROUND","1")!="0":
        run("ground",[tests,"atmosphere::tests::vf2::gpu_ground_limit","--ignored","--exact","--nocapture"])
    for path in filter(None,os.environ.get("VF2_PATHS","nonrt,rt").split(",")):
        for scene in os.environ.get("VF2_SCENES","slats,windows").split(","):
            name="fixture-"+path+("-windows" if scene=="windows" else "")
            run(name,[tests,"atmosphere::tests::vf2::gpu_shafts","--ignored","--exact","--nocapture"],dict(VF2_PATH=path,VF2_WINDOWS="1" if scene=="windows" else "0",VF2_OUT=str(root/name)))
    if os.environ.get("VF2_QUARRY","1")!="0":
        for path in filter(None,os.environ.get("VF2_QUARRY_PATHS","nonrt,traced").split(",")):
            if os.environ.get("VF2_BASELINE_QUARRY")=="1":
                run("quarry-baseline-"+path,[str(g.BIN/"baseline.exe"),"atmosphere::tests::gpu_at1_quarry","--ignored","--exact","--nocapture"],dict(AT1_QUARRY="E:/renderlab-work/m7-g1c/quarry/manifest.json",AT1_OUT=str(root/("baseline-"+path)),AT1_PATH=path,VF2="1"),minimum=6144)
            run("quarry-"+path,[tests,"atmosphere::tests::gpu_at1_quarry","--ignored","--exact","--nocapture"],dict(AT1_QUARRY="E:/renderlab-work/m7-g1c/quarry/manifest.json",AT1_OUT=str(root/path),AT1_PATH=path,VF2="1"),minimum=6144)
    if durations.get("lib",{}).get("exit",0):raise SystemExit(1)

if __name__=="__main__":
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("action",choices=["build","gpu","batch"])
    p.add_argument("label",nargs="?",default="a")
    a=p.parse_args()
    if a.action=="build":build()
    elif a.action=="gpu":gpu(a.label)
    else:batch(a.label)
