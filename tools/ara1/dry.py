"""One short engine batch. Run through lease.py; reads the current order per step."""
import argparse
import os
from pathlib import Path
import subprocess
import sys

from lease import check_order

REPO=Path(__file__).resolve().parents[2]


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("engine",choices=["ara","renderlab"])
    p.add_argument("--root",type=Path,default=Path("D:/Coding/renderlab-work/ara1"))
    p.add_argument("--tag",required=True)
    p.add_argument("--startup-minimal",action="store_true")
    a=p.parse_args();scene=a.root/("scenes-minimal" if a.startup_minimal else "scenes-dry")/"game.json";out=a.root/"dry"/a.tag
    binary=a.root/("target-ara-main/debug/ara1-ara.exe" if a.engine=="ara" else "target-renderlab/debug/examples/ara1_capture.exe")
    source=a.root/"ara-main" if a.engine=="ara" else REPO
    modes=["bake","raster","hybrid"] if a.engine=="ara" else ["nonrt","rt"]
    for mode in modes:
        check_order(os.environ["ARA1_EXPECTED_ORDER"])
        command=[sys.executable,"-B",str(REPO/f"tools/ara1/run-{a.engine}.py"),"--binary",str(binary),"--source",str(source),
                 "--scene",str(scene),"--mode",mode,"--out",str(out/mode),"--profile","dev","--contention","shared GPU, functional dry run; not performance evidence"]
        if a.engine=="ara":command += ["--bake-cache",str(a.root/("bakes/game-minimal" if a.startup_minimal else "bakes/game"))]
        print("ARA1 dry step",mode,flush=True)
        subprocess.run(command,check=True)


if __name__=="__main__":main()
