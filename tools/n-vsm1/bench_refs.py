"""N-VSM1: S2/S3 bench roots on D: for scripts/n8-batch.py's score steps.

n8-batch hardlinks the scene and N7a's film references from E: into its root, which fails for a
root on another drive. This copies them once to D:/Coding/renderlab-work/n-vsm1/bench/refs and
hardlinks them from there into each ROOT, where n8-batch then finds them in place.

    py -3 -B tools/n-vsm1/bench_refs.py ROOT...
"""
import os
from pathlib import Path
import shutil
import sys

MOTION = Path("E:/renderlab-work/motion-benches")
REFERENCES = {"S2": Path("E:/renderlab-work/n7a/runs/s2-1"), "S3": Path("E:/renderlab-work/n7a/runs/s3-1")}
CASES = {"S2": ["camera-0.25px-r0.04", "shadow-step-r0.04"], "S3": ["sun-r0.04"]}
REFS = Path("D:/Coding/renderlab-work/n-vsm1/bench/refs")


def files():
    for bench, cases in CASES.items():
        yield MOTION / bench / "fixture/scene.json", Path(bench) / "fixture/scene.json"
        for case in cases:
            for source in (REFERENCES[bench] / case / "reference").iterdir():
                yield source, Path(bench) / "renderlab" / case / "reference" / source.name


if __name__ == "__main__":
    for source, relative in files():
        local = REFS / relative
        if not local.exists():
            local.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, local)
        for root in sys.argv[1:]:
            target = Path(root) / relative
            if not target.exists():
                target.parent.mkdir(parents=True, exist_ok=True)
                os.link(local, target)
    print("references in place for", *sys.argv[1:])
