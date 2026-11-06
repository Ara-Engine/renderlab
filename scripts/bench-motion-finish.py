"""After native capture, regenerate matched materials and overlap CPU compaction."""
import datetime
import json
import os
from pathlib import Path
import subprocess
import threading
import time

ROOT=Path("E:/renderlab-work/motion-benches")
REPO=Path(__file__).resolve().parents[1]
LOCK=Path("D:/Coding/renderlab-work/gpu-slot.lock")
SCHEDULE=Path("E:/renderlab-work/gpu-schedule.md")


def native_complete(root):
    path=root/"unreal-render-process.json"
    try:return (root/"unreal/complete.json").exists() and json.loads(path.read_text())["exit"]==0
    except (FileNotFoundError,json.JSONDecodeError):return False


def main():
    while not all(native_complete(ROOT/b) for b in ("S2","S3")):time.sleep(5)
    while LOCK.exists():time.sleep(2)
    now=datetime.datetime.now(datetime.timezone.utc).isoformat()
    with SCHEDULE.open("a",encoding="utf-8") as ledger:
        ledger.write(f"\nUE reservation RELEASED owner=s2-s3-competitor-benches {now}: both native engines complete. Corrected Renderlab metallic0 batches request the next slots (~12min GPU total, CPU scoring overlaps); queued N4 short check may go first.\n")
    print("Native complete; UE reservation released",flush=True)
    env=os.environ.copy();env.update(OPENBLAS_NUM_THREADS="2",OMP_NUM_THREADS="2",NUMBA_NUM_THREADS="2")
    errors=[]
    def scoring():
        try:
            for bench in ("S2","S3"):
                root=ROOT/bench;scene=json.loads((root/"fixture/scene.json").read_text())
                for track in scene["tracks"]:
                    for radius in scene["radii"]:
                        case=f"{track['name']}-r{radius:g}";directory=root/"renderlab"/case
                        while not (directory/"reference-reuse.json").exists():time.sleep(2)
                        if (directory/"scores.json").exists():continue
                        print("CPU paired scoring",bench,case,flush=True)
                        subprocess.run(["py","-3","-B","scripts/bench-motion-score.py","--root",str(root),"--case",case],cwd=REPO,env=env,check=True)
                subprocess.run(["py","-3","-B","scripts/bench-motion-score.py","--root",str(root),"--comparison-only"],cwd=REPO,env=env,check=True)
        except Exception as error:errors.append(repr(error))
    worker=threading.Thread(target=scoring,daemon=True);worker.start()
    # Allow the short already-queued N4 check its requested turn after UE.
    time.sleep(60)
    for bench in ("S2","S3"):
        root=ROOT/bench;env["RENDERLAB_MOTION_ROOT"]=str(root);attempt=1
        while True:
            label=f"renderlab-metallic0-{attempt:02}"
            if any((root/"runs"/(label+suffix)).exists() for suffix in (".json","-deferred.json")):attempt+=1;continue
            command=["py","-3","-B","scripts/bench-motion-run.py","--label",label,"--minutes","30","--fast-check","--manifest",str(root/"manifest.json"),"--","py","-3","-B","scripts/bench-motion-renderlab-run.py"]
            code=subprocess.run(command,cwd=REPO,env=env,creationflags=subprocess.CREATE_NO_WINDOW).returncode
            if errors:raise RuntimeError(errors)
            if code==0:break
            path=root/"runs"/(label+".json");record=json.loads(path.read_text()) if path.exists() else {}
            if code!=3 and not any("memory" in v for v in record.get("failures",[])):raise SystemExit(code)
            attempt+=1;time.sleep(120)
    while worker.is_alive():worker.join(timeout=10)
    if errors:raise RuntimeError(errors)
    print("Matched-material GPU batches and paired CPU scoring complete",flush=True)


if __name__=="__main__":main()
