"""Phase-2 held-window checks; one renderer process at a time, no builds."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

from lease import check_order

REPO=Path(__file__).resolve().parents[2]
ROOT=Path("D:/Coding/renderlab-work/ara1")


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("engine",choices=["renderlab","ara"])
    p.add_argument("--tag",required=True)
    p.add_argument("--reuse-bake",action="store_true")
    p.add_argument("--reuse-completed",help="previous tag whose successful steps remain valid")
    a=p.parse_args()
    specs=[("minimal",ROOT/"scenes-minimal/game.json"),("normal",ROOT/"scenes-dry/game.json")]
    modes=["rt"] if a.engine=="renderlab" else ["bake","raster","hybrid"]
    for profile,scene in specs:
        for mode in modes:
            cache=ROOT/"bakes"/("p2-game-"+profile)
            if a.reuse_bake and mode=="bake" and (cache/"bake.json").exists():continue
            if a.reuse_completed:
                previous=ROOT/"phase2-startup"/a.reuse_completed/(profile+"-"+mode)/"process.json"
                if previous.exists() and json.loads(previous.read_text())["exit_code"]==0:continue
            state=check_order(os.environ["ARA1_EXPECTED_ORDER"])
            binary=ROOT/("target-renderlab/debug/examples/ara1_capture.exe" if a.engine=="renderlab" else "target-ara-main/debug/ara1-ara.exe")
            source=REPO if a.engine=="renderlab" else ROOT/"ara-main"
            out=ROOT/"phase2-startup"/a.tag/(profile+"-"+mode)
            command=[sys.executable,"-B",str(REPO/f"tools/ara1/run-{a.engine}.py"),"--binary",str(binary),"--source",str(source),
                     "--scene",str(scene),"--mode",mode,"--out",str(out),"--profile","dev","--contention","root held window; other units instructed to stop builds and GPU; verify lease samples"]
            if a.engine=="ara":command += ["--bake-cache",str(cache)]
            print("startup step",profile,mode,"order",state["sha256"],flush=True)
            subprocess.run(command,check=True)


if __name__=="__main__":main()
