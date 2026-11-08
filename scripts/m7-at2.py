"""AT2: two-job CPU compilation; lib/images/timing inside n8's guarded GPU lease.
Evidence, staged builds and the disposable target are under D:/Coding/renderlab-work/at2.
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
g.OUT=Path("D:/Coding/renderlab-work/at2")
g.OWNER="m7-at2"
g.BIN=g.OUT/"bin"

def order_ready():
    """Every acquisition/step rereads root orders; new orders require explicit review."""
    schedule=Path("E:/renderlab-work/gpu-schedule.md")
    while True:
        try: lines=schedule.read_text(encoding="utf-8").splitlines();break
        except PermissionError: time.sleep(.05)
    orders=[(i,line) for i,line in enumerate(lines) if line.startswith("[root order]")]
    if not orders:return True
    position,order=orders[-1]
    config=g.OUT/"gpu-after.json"
    if not config.exists():return False
    gates=json.loads(config.read_text())
    if gates.get("acknowledged_order")!=order:return False
    later=lines[position+1:]
    return all(any(line.startswith(gate["owner"]+" ") and "window released" in line
                   and gate.get("label", "") in line for line in later) for gate in gates["after"])


def gpu(label):
    checks,host=g.schedule()
    g.OUT.mkdir(parents=True,exist_ok=True)
    with host.SCHEDULE.open("a",encoding="utf-8") as f:
        f.write(f"\n[m7-at2 request] owner=m7-at2 label={label} <=15 min headless; reads root orders before every lease/step, honours gpu-after.json. {dt.datetime.now(dt.timezone.utc).isoformat()}\n")
    deadline=time.monotonic()+5400
    attempt=0
    while time.monotonic()<deadline:
        if host.LOCK.exists() or not order_ready() or checks.hold_state(host)=="hold" or checks.quiet_active(host,any_lease=False):
            time.sleep(.05);continue
        attempt+=1
        os.environ.update(N8_OWNER="m7-at2",N8_OUT=str(g.OUT))
        guard=g.load("n8-run")
        original=guard.s1.runner.state
        def state(owned=()):
            if checks.hold_state(host)=="hold" or checks.quiet_active(host,any_lease=False):
                raise RuntimeError("game hold or foreign QUIET window")
            if not owned and not order_ready():
                raise RuntimeError("root order changed before launch")
            return original(owned)
        guard.s1.runner.state=state
        sys.argv=["n8-run","--label",label+"-"+str(attempt),"--minutes","15","--fast-check","--",
                  sys.executable,"-B",str(Path(__file__).resolve()),"batch",label]
        try:code=guard.s1.runner.main()
        finally:
            if guard.s1.claimed:
                with host.SCHEDULE.open("a",encoding="utf-8") as f:
                    f.write(f"\nm7-at2 {label}-{attempt} window released {guard.s1.runner.utc()} (no owned GPU/build job remains).\n")
        if code!=3:raise SystemExit(code)
        time.sleep(.05)
    raise SystemExit("GPU wait deadline reached; no job remains")

def batch(label):
    owner=json.loads(Path("D:/Coding/renderlab-work/gpu-slot.lock/owner.json").read_text())
    assert owner["owner"]=="m7-at2" and owner["label"].startswith(label+"-"), "AT2's owned n8 lease required"
    root=g.OUT/"images"/label
    root.mkdir(parents=True,exist_ok=True)
    def run(name,command,extra=None):
        if not order_ready():
            print("root order holds AT2 before step",name,flush=True)
            raise SystemExit(75)
        env=dict(os.environ,RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",**(extra or {}))
        with (root/(name+".log")).open("w") as log:
            code=subprocess.run(command,cwd=REPO,env=env,stdout=log,stderr=subprocess.STDOUT).returncode
        print(name,"exit",code,flush=True)
        if code:
            print((root/(name+".log")).read_text(errors="replace")[-6000:],flush=True)
            raise SystemExit(code)
    tests=str(g.BIN/"lib.exe")
    if os.environ.get("AT2_LIB", "1") != "0":
        run("lib",[tests,"--test-threads=2"])
    manifest="E:/renderlab-work/m7-g1c/quarry/manifest.json"
    for path in os.environ.get("AT2_PATHS", "nonrt,traced,raster").split(","):
        run("quarry-"+path,[tests,"atmosphere::tests::gpu_at1_quarry","--ignored","--exact","--nocapture"],
            dict(AT1_QUARRY=manifest,AT1_OUT=str(root/path),AT1_PATH=path,AT2="1"))
    for path in ["nonrt","rt"]:
        for mode in ["still","sun"]:
            run("game-"+path+"-"+mode,[str(g.BIN/"example.exe"),"--path",path,"--out",str(root/("game-"+path+"-"+mode)),
                "--frames","144","--every","16","--size","1280x720","--still",
                "--sun-speed", "1.5" if mode=="sun" else "0", "--sun-after","96","--timing-frames","32"])

if __name__=="__main__":
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("action",choices=["build","gpu","batch"])
    p.add_argument("label",nargs="?",default="a")
    a=p.parse_args()
    if a.action=="build":g.build()
    elif a.action=="gpu":gpu(a.label)
    else:batch(a.label)
