"""R1 collection; each invocation is guarded and offscreen. Fixtures gate quarry."""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess
import sys

ROOT=Path(__file__).resolve().parents[1]
OUT=Path("D:/Coding/renderlab-work/m6-exact-quality/r1")
F0=OUT.parent/"f0"
TEST="renderer::fresh_shadow_tests::primary_ray_gpu_tests::film_reference_tests::gpu_film_reference"

def cases():
    for policy in ("magnitude","viewed"):
        for kind in ("flat","convex"):
            for distance in (1000,10000):
                for incidence in (80,85,89):
                    yield f"{kind}-{distance}-{incidence}-{policy}",dict(workload=kind,distance=distance,incidence=incidence,receiver_policy=policy)
        for work in ("sawtooth","thin","stress"):
            yield f"{work}-{policy}",dict(workload=work,receiver_policy=policy)
    for policy in ("minimal","magnitude","viewed"):
        for pose in (0,1,2):
            yield f"quarry-{policy}-p{pose}",dict(workload="quarry",pose=pose,receiver_policy=policy)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument("--phase",choices=("fixtures","quarry"));p.add_argument("--case");p.add_argument("--attempt",default="01");a=p.parse_args()
    binary=list((OUT/"target/release/deps").glob("renderlab-*.exe"))
    assert len(binary)==1,binary
    template=json.loads((F0/"quarry-minimal-p0.json").read_text())
    def validate(directory):
        validation=directory/"validation.json"
        if validation.exists() and json.loads(validation.read_text())["passed"]:return 0
        result=subprocess.run([sys.executable,"-B",str(ROOT/"scripts/m6-receiver-fixtures.py"),str(directory)],cwd=ROOT,creationflags=subprocess.CREATE_NO_WINDOW,capture_output=True,text=True)
        (directory/f"oracle-run-{a.attempt}.log").write_text(result.stdout+result.stderr)
        print(result.stdout+result.stderr,flush=True)
        return result.returncode
    for name,fields in cases():
        if a.case and name!=a.case:continue
        if a.phase=="fixtures" and fields["workload"]=="quarry":continue
        if a.phase=="quarry" and fields["workload"]!="quarry":continue
        directory=OUT/name
        if (directory/"pose-0.film.bin").exists() or (directory/f"pose-{fields.get('pose',0)}.film.bin").exists():
            print("reuse",name,flush=True)
            if fields["workload"]!="quarry":
                code=validate(directory)
                if code:return code
            continue
        if fields["workload"]=="quarry":
            expected=[OUT/n/"validation.json" for n,f in cases() if f["workload"]!="quarry"]
            assert all(path.exists() and json.loads(path.read_text())["passed"] for path in expected),"all R1 fixtures must pass first"
        manifest=template|fields|dict(output=str(directory),minimal=True)
        path=OUT/f"{name}.json";path.write_text(json.dumps(manifest,indent=2))
        cmd=[sys.executable,"-B",str(ROOT/"scripts/m6-receiver-run.py"),"--label",name+"-"+a.attempt,"--manifest",str(path),"--minutes","10","--wait-seconds","45","--",str(binary[0]),TEST,"--ignored","--exact","--nocapture","--test-threads=1"]
        result=subprocess.run(cmd,cwd=ROOT,creationflags=subprocess.CREATE_NO_WINDOW)
        if result.returncode:return result.returncode
        if fields["workload"]!="quarry":
            code=validate(directory)
            if code:return code
    return 0

if __name__=="__main__":raise SystemExit(main())
