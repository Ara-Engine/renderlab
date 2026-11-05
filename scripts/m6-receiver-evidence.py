"""Snapshot R1 tooling, record provenance, and hash retained evidence (no GPU)."""
import datetime
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

def sha(path):
    with path.open("rb") as stream:return hashlib.file_digest(stream,"sha256").hexdigest()

def main():
    repo=Path(__file__).resolve().parents[1]
    root=Path("D:/Coding/renderlab-work/m6-exact-quality/r1")
    files=[repo/p for p in ("src/film_reference_tests.rs","src/receiver_policy_tests.rs","support/film_accumulate.wgsl","support/receiver_policy.wgsl","scripts/m6-film-run.py","docs/m6-receiver-policy.md")]
    files+=list((repo/"scripts").glob("m6-receiver-*.py"))
    for path in files:
        target=root/"source"/path.relative_to(repo);target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(path,target)
    windows=json.loads((root/"gpu-windows.json").read_text())
    assert all(w["seconds"]<=45*60 for w in windows)
    runs=[json.loads(p.read_text()) for p in (root/"runs").glob("*.json") if not p.name.endswith(("-preflight.json","-deferred.json"))]
    gpu=[r for r in runs if r["exit"]==0 and r["manifest"] is not None]
    assert len(gpu)==39,len(gpu)
    baselines={}
    for pose in (0,1):
        directory=root.parent/f"f0/quarry-minimal-p{pose}"
        for path in directory.glob(f"pose-{pose}.*"):baselines[str(path)]=sha(path)
    baselines[str(root.parent/"f0/provenance.json")]=sha(root.parent/"f0/provenance.json")
    provenance=dict(utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),source_commit=subprocess.check_output(["git","rev-parse","HEAD"],cwd=repo,text=True).strip(),
                    evidence_root=str(root),physical_root=str(root.resolve()),accepted_gpu_runs=len(gpu),
                    gpu_binaries=sorted({r["executable_sha256"] for r in gpu}),baseline_hashes=baselines,
                    windows=len(windows),held_seconds=sum(w["seconds"] for w in windows),longest_seconds=max(w["seconds"] for w in windows),
                    no_production_change=True,no_push=True,inspection="Complete fixture sheets and quarry R_F/R_L/R_D at both exposures, decision crops, frozen contacts; static only.")
    (root/"provenance.json").write_text(json.dumps(provenance,indent=2))
    hashes=[]
    for path in sorted(root.rglob("*")):
        if not path.is_file() or "target" in path.relative_to(root).parts or path.name=="identities.sha256":continue
        hashes.append(f"{sha(path)}  {path.relative_to(root).as_posix()}")
    (root/"identities.sha256").write_text("\n".join(hashes)+"\n")
    print("accepted GPU runs",len(gpu),"evidence files hashed",len(hashes),"windows",len(windows),"held seconds",provenance["held_seconds"])

if __name__=="__main__":main()
