"""One bounded GPU step through scripts/n8-run.py, with current root-order checks.

`order` prints the latest *anchored* directive and its hash. Read it before every
run; --after tokens require release lines after that directive. A changed order
refuses the lease. This never auto-requeues or starts an unattended waiter.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import sys
import time

REPO=Path(__file__).resolve().parents[2]
SCHEDULE=Path("E:/renderlab-work/gpu-schedule.md")


def order_state(path=SCHEDULE):
    lines=path.read_text(encoding="utf-8").splitlines()
    directives=[(i,line) for i,line in enumerate(lines) if line.startswith("[root order]")]
    if not directives:raise RuntimeError("no anchored root order in schedule")
    index,line=directives[-1]
    return dict(line=line,sha256=hashlib.sha256(line.encode()).hexdigest(),following=lines[index+1:])


def check_order(expected,after=()):
    state=order_state()
    if state["sha256"]!=expected:raise RuntimeError("root order changed; read the latest directive before retrying: "+state["line"])
    for token in after:
        if not released_after(state,token):
            raise RuntimeError("still waiting for release after latest root order: "+token)
    return state


def released_after(state,token):
    for line in state["following"]:
        match=re.fullmatch(r"(\S+) (.+?) window released (\d{4}-\d\d-\d\dT.+)",line)
        if match and (token==match[1] or token==match[2] or match[2].startswith(token+"-")):
            return True
    return False


def main():
    if sys.argv[1:]==["order"]:
        print(json.dumps(order_state(),indent=2));return 0
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--expected-order",required=True)
    p.add_argument("--after",action="append",default=[])
    p.add_argument("--root",type=Path,default=Path("D:/Coding/renderlab-work/ara1"))
    p.add_argument("--label",required=True)
    p.add_argument("--minutes",type=float,default=5)
    p.add_argument("--handoff-seconds",type=float,default=0,help="bounded in-process 50 ms polling; never starts another agent")
    p.add_argument("--low-memory-dry",action="store_true",help="root-authorized low-resolution fallback: 2 GiB preflight/runtime guard")
    p.add_argument("command",nargs=argparse.REMAINDER)
    a=p.parse_args()
    if not 0<a.minutes<=15:p.error("ARA1 leases must be <=15 minutes")
    directive=check_order(a.expected_order,() if a.handoff_seconds else a.after)
    spec=importlib.util.spec_from_file_location("ara1_n8",REPO/"scripts/n8-run.py")
    n8=importlib.util.module_from_spec(spec)
    os.environ.update(N8_OWNER="m7-ara1",N8_OUT=str(a.root/"lease"),ARA1_EXPECTED_ORDER=a.expected_order)
    spec.loader.exec_module(n8)
    if a.handoff_seconds:
        deadline=time.monotonic()+a.handoff_seconds
        print("waiting in-process for release/lock handoff",flush=True)
        while True:
            state=order_state()
            if state["sha256"]!=a.expected_order:raise RuntimeError("root order changed during handoff: "+state["line"])
            released=all(released_after(state,token) for token in a.after)
            if released and not n8.s1.runner.LOCK.exists():break
            if time.monotonic()>=deadline:raise RuntimeError("bounded handoff wait expired")
            time.sleep(.05)
    if a.low_memory_dry:
        n8.s1.runner.PREFLIGHT_MEMORY_MBYTES=2048
        n8.s1.runner.RUNTIME_MEMORY_MBYTES=2048
        original_state=n8.s1.runner.state
        first=True
        def state(owned=()):
            nonlocal first
            value=original_state(owned)
            if first:
                first=False
                deadline=time.monotonic()+30
                while value["available_mbytes"]<8192 and time.monotonic()<deadline and not value["games"] and not value["gpu_processes"]:
                    # The lock is already owned: memory queries cannot lose the handoff.
                    time.sleep(max(0,min(1,deadline-time.monotonic())))
                    if n8.s1.runner.resource_sample()["available_mbytes"]>=8192:break
                value=original_state(owned)
                print("preflight memory MiB",value["available_mbytes"],"(8 GiB preferred; authorized 2 GiB fallback)",flush=True)
            return value
        n8.s1.runner.state=state
    original_save=n8.s1.runner.save
    def save(path,value):
        if path==n8.s1.runner.LOCK/"owner.json":
            try:check_order(a.expected_order,a.after)
            except RuntimeError:
                # n8 has just atomically created this still-empty directory. A
                # changed order must not leave an orphaned shared GPU lock.
                n8.s1.runner.LOCK.rmdir()
                raise
            value["root_order"]=directive["line"]
        original_save(path,value)
    n8.s1.runner.save=save
    command=a.command[1:] if a.command[:1]==["--"] else a.command
    if not command:p.error("command required")
    sys.argv=[str(REPO/"scripts/n8-run.py"),"--label",a.label,"--minutes",str(a.minutes),
              "--record-resources" if a.low_memory_dry else "--fast-check","--",*command]
    try:return n8.s1.runner.main()
    finally:
        if n8.s1.claimed:
            with SCHEDULE.open("a",encoding="utf-8") as f:
                f.write(f"\nm7-ara1 {a.label} window released {n8.s1.runner.utc()} (no owned GPU/build job remains).\n")


if __name__=="__main__":
    try:raise SystemExit(main())
    except RuntimeError as e:
        print("DEFERRED:",e,flush=True);raise SystemExit(3)
