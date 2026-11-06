"""Use S1's hidden-process launcher, shared GPU lease and 4/2 GiB live guard."""
import importlib.util
import os
import sys
from pathlib import Path

spec=importlib.util.spec_from_file_location("s1_runner",Path(__file__).with_name("bench-contact-fold-run.py"))
s1=importlib.util.module_from_spec(spec);spec.loader.exec_module(s1)
s1.runner.OUT=Path(os.environ["RENDERLAB_MOTION_ROOT"])
s1.runner.OWNER="s2-s3-competitor-benches"
original_state=s1.runner.state


def state(owned=()):
    # cargo and the Unreal supervisor own child processes; never mistake those
    # descendants for a foreign renderer, while still rejecting unrelated jobs.
    value=original_state(owned)
    # Snapshot descendants after the original query: cargo can spawn the test
    # executable between queries while compilation completes.
    processes=s1.runner.ps("Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId | ConvertTo-Json -Compress")
    descendants=set(owned)
    while True:
        found={p["ProcessId"] for p in processes if p["ParentProcessId"] in descendants}
        if found<=descendants:break
        descendants|=found
    live={p["ProcessId"] for p in processes}
    for key in ("games","gpu_processes","builds"):
        value[key]=[p for p in value[key] if p["ProcessId"] not in descendants and p["ProcessId"] in live]
    return value


s1.runner.state=state
if __name__=="__main__":
    if "--ue-startup" in sys.argv:
        sys.argv.remove("--ue-startup")
        if "--fast-check" in sys.argv:sys.argv.remove("--fast-check")
        sys.argv.insert(sys.argv.index("--"),"--record-resources")
        s1.runner.PREFLIGHT_MEMORY_MBYTES=6144
    try:raise SystemExit(s1.runner.main())
    finally:
        if s1.claimed:
            with s1.SCHEDULE.open("a",encoding="utf-8") as f:
                label=sys.argv[sys.argv.index("--label")+1]
                f.write(f"\ns2-s3-competitor-benches {label} window released {s1.runner.utc()} (no owned GPU/build job remains).\n")
