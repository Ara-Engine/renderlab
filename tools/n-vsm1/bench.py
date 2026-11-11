"""N-VSM2: S1-S3 against the film references for one lookup, and small per-frame dumps.

    py -3 -B tools/n-vsm1/bench.py run ROOT BIN LOOKUP STEP...     # inside a lease
        STEP: s1 | s2 | s3 (the scored benches, every frame, one repeat), or
              s2:CASE:FRAME,FRAME... / s3:CASE:FRAME,... (those frames' captures only)
    py -3 -B tools/n-vsm1/bench.py score ROOT                      # CPU: scores, table, packs, deletes raw

`run` is scripts/n8-batch.py's score steps with the references on D: (bench_refs.py) and an
optional frame list. `score` runs the unchanged scorers (scripts/n8-score.py, S1's
bench-contact-fold-score.py), packs every captured visibility crop into ROOT/vis-BENCH-CASE.npz
(uint16 counts of 256, frames in `frames`) and deletes the raw captures.
RENDERLAB_VD / RENDERLAB_VD_SHADER select a Disk variant (steps.py `@KEY=VALUE`).
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

import numpy as np

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
MOTION = Path("E:/renderlab-work/motion-benches")
S1 = Path("E:/renderlab-work/contact-fold")
CASES = {"s2": ["camera-0.25px-r0.04", "shadow-step-r0.04"], "s3": ["sun-r0.04"]}


def run(root, binary, lookup, steps):
    subprocess.check_call(["py", "-3", "-B", str(HERE / "bench_refs.py"), str(root)])
    worst = 0
    for step in steps:
        bench, _, rest = step.partition(":")
        if bench == "s1":
            test = "gpu_contact_fold_batch"
            manifest = dict(scene=str(S1 / "fixture/scene.json"), output=str(root / "S1/renderlab"), modes=["Virtual"])
        else:
            test = "gpu_motion_bench_batch"
            upper = bench.upper()
            manifest = dict(scene=str(MOTION / upper / "fixture/scene.json"), output=str(root / upper / "renderlab"), root=str(root / upper),
                            score_script=None, modes=["Virtual"], cases=CASES[bench], skip_reference=True, repeats=1, no_full_captures=True)
            if rest:
                case, frames = rest.split(":")
                manifest.update(cases=[case], capture_frames=[int(f) for f in frames.split(",")])
        manifest["virtual"] = {"lookup": lookup}
        path = root / f"manifest-{bench}.json"
        path.write_text(json.dumps(manifest, indent=2))
        code = subprocess.run([binary, test, "--skip", test + "_", "--ignored", "--nocapture", "--test-threads=1"],
                              env=dict(os.environ, RENDERLAB_PRIMARY_MANIFEST=str(path))).returncode
        print("bench", step, "exit", code, flush=True)
        worst = max(worst, abs(code))
    return min(worst, 1)


def pack(root):
    """Visibility crops to counts, per case; then the raw captures go."""
    for bench in ("S2", "S3"):
        base = root / bench / "renderlab"
        if not base.is_dir():
            continue
        scene = json.loads((root / bench / "fixture/scene.json").read_text())
        x0, y0, x1, y1 = scene["crop"]
        for case in (p for p in base.iterdir() if (p / "Virtual").is_dir()):
            files = sorted((case / "Virtual").glob("*-visibility.rgba16f"))
            if not files:
                continue
            counts = np.stack([np.rint(np.fromfile(f, dtype="<f2").reshape(y1 - y0, x1 - x0, 4)[..., 0].astype("f4") * 256).astype("u2") for f in files])
            np.savez_compressed(root / f"vis-{bench}-{case.name}.npz", counts=counts, frames=np.array([int(f.name[:4]) for f in files]))
        shutil.rmtree(base)


def score(root):
    full = [b for b in ("S2", "S3") if (root / b / "renderlab/timings.json").exists()
            and not json.loads((root / f"manifest-{b.lower()}.json").read_text()).get("capture_frames")]
    if full:
        with (root / "n8-score.log").open("w") as log:
            subprocess.check_call(["py", "-3", "-B", str(REPO / "scripts/n8-score.py"), "--root", str(root)], stdout=log)
    if (root / "S1/renderlab").is_dir():
        (root / "S1/fixture").mkdir(parents=True, exist_ok=True)
        shutil.copy2(S1 / "fixture/scene.json", root / "S1/fixture/scene.json")
        with (root / "S1-score.log").open("w") as log:
            subprocess.check_call(["py", "-3", "-B", str(REPO / "scripts/bench-contact-fold-score.py"), "--root", str(root / "S1")], stdout=log)
        shutil.rmtree(root / "S1/renderlab")
    pack(root)
    table(root)


def table(root):
    if (root / "S1/comparison.csv").exists():
        import csv
        for r in csv.DictReader((root / "S1/comparison.csv").open()):
            if r["mode"] == "RL Virtual" and r["environment"] == "0":
                print("S1 pose", r["pose"], "partial %.4f" % float(r["partial_rms"]), "flips", r["solid_flips"], "leaks", r["leaks"], "lost", r["contact_lost"])
    if (root / "n8-comparison.json").exists():
        for r in json.loads((root / "n8-comparison.json").read_text()):
            if r["source"] == "this run":
                print(r["bench"], r["case"][:11], r["phase"][:13].ljust(13), "partial %.4f" % r["partial_rms"], "flips", r["flips_max"], "leaks", r["leaks_max"],
                      "lost", r["contact_lost"], "shadow ms %.2f" % (r["shadow_p50"] or 0))


if __name__ == "__main__":
    root = Path(sys.argv[2])
    root.mkdir(parents=True, exist_ok=True)
    if sys.argv[1] == "run":
        raise SystemExit(run(root, sys.argv[3], sys.argv[4], sys.argv[5:]))
    {"score": score, "table": table}[sys.argv[1]](root)
