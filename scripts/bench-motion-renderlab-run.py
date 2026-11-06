"""Verify the library by exit status, then run one owned Renderlab GPU batch."""
import os
import hashlib
import json
from pathlib import Path
import subprocess
import sys


def main():
    repo=Path(__file__).resolve().parents[1]
    target=Path("E:/renderlab-work/motion-benches/target")
    command=["cargo","test","-p","renderlab","--lib","-j","2","--target-dir",str(target),"--","--test-threads=1"]
    subprocess.run(command,cwd=repo,check=True,stdout=sys.stdout,stderr=subprocess.STDOUT,creationflags=subprocess.CREATE_NO_WINDOW)
    binaries=list((target/"debug/deps").glob("renderlab-*.exe"))
    if len(binaries)!=1:raise RuntimeError("ambiguous library test binary")
    sources=[*repo.glob("src/**/*.rs"),*repo.glob("shaders/**/*.wgsl"),repo/"Cargo.toml",repo/"Cargo.lock"]
    identity=dict(binary=str(binaries[0]),binary_sha256=hashlib.file_digest(binaries[0].open("rb"),"sha256").hexdigest(),
                  source_sha256={str(p.relative_to(repo)):hashlib.file_digest(p.open("rb"),"sha256").hexdigest() for p in sources},
                  profile="development/test opt-1 renderlab; active Vulkan validation layer; not a release CPU measurement",
                  library_test_exit=0)
    (Path(os.environ["RENDERLAB_MOTION_ROOT"])/"renderlab-build.json").write_text(json.dumps(identity,indent=2))
    subprocess.run([str(binaries[0]),"gpu_motion_bench_batch","--ignored","--nocapture","--test-threads=1"],cwd=repo,check=True,stdout=sys.stdout,stderr=subprocess.STDOUT,creationflags=subprocess.CREATE_NO_WINDOW)


if __name__=="__main__":main()
