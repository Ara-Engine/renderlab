"""N8's one batched GPU check: every ignored N8 test in one lease, one process per bench.

Run inside scripts/n8-run.py, which owns the GPU lock and the memory guard:

    py -3 -B scripts/n8-run.py --label n8-batch --minutes 40 --fast-check -- \
        py -3 -B scripts/n8-batch.py <test binary> <root> [step ...]

Steps: fixture, s1, s2, s3, quarry (default: all). Each step writes its manifest under
<root>; scoring runs afterwards on the CPU (n8-score.py). N8b's score-* steps run N8 alone
against existing references: S1 builds its own in-process, S2/S3 hardlink N7a's regenerated
film references (REFERENCES) and skip regeneration, and the quarry step reuses a traced
reference already built in its output (score-quarry builds it once, with the Interactive
control). N8B_REPEATS sets the S2/S3 timing replays (default 8).
"""
import json
import os
import subprocess
import sys
import time
from pathlib import Path

QUARRY = Path("D:/Coding/renderlab-work/m6-adaptive/gpu-phase-quarry/quarry-manifest.json")
MOTION = Path("E:/renderlab-work/motion-benches")
S1 = Path("E:/renderlab-work/contact-fold")
MODES = ["Interactive", "Virtual"]
# N7a's regenerated S2/S3 film references (gpu-schedule.md, 08:43Z), compacted per pose.
REFERENCES = {"S2": Path("E:/renderlab-work/n7a/runs/s2-1"), "S3": Path("E:/renderlab-work/n7a/runs/s3-1")}
S2_CASES = ["camera-0.25px-r0.04", "shadow-step-r0.04"]


def link_references(root, bench, cases):
    """Hardlink the regenerated per-pose references (and the scene) into an N8b root."""
    (root / bench / "fixture").mkdir(parents=True, exist_ok=True)
    scene = root / bench / "fixture/scene.json"
    if not scene.exists():
        os.link(MOTION / bench / "fixture/scene.json", scene)
    for case in cases:
        destination = root / bench / "renderlab" / case / "reference"
        destination.mkdir(parents=True, exist_ok=True)
        for source in (REFERENCES[bench] / case / "reference").iterdir():
            if not (destination / source.name).exists():
                os.link(source, destination / source.name)


def manifests(root):
    quarry = json.loads(QUARRY.read_text())
    repeats = int(os.environ.get("N8B_REPEATS", "8"))
    # Pre-N8p reference screens keep Exact, including partial settings in lease scripts.
    variants = {name: {**settings, "virtual": {"lookup": "exact", **settings.get("virtual", {})}}
                for name, settings in json.loads(os.environ.get("N8B_VARIANTS", '{"default": {}}')).items()}
    return {
        "fixture": ("gpu_virtual_fixture", dict(output=str(root / "fixture-run"))),
        "s1": ("gpu_contact_fold_batch", dict(scene=str(S1 / "fixture/scene.json"), output=str(root / "S1/renderlab"), modes=MODES)),
        "s2": ("gpu_motion_bench_batch", dict(scene=str(MOTION / "S2/fixture/scene.json"), output=str(root / "S2/renderlab"), root=str(root / "S2"),
                                              score_script=None, modes=MODES, cases=["camera-0.25px-r0.04", "shadow-step-r0.04"])),
        "s3": ("gpu_motion_bench_batch", dict(scene=str(MOTION / "S3/fixture/scene.json"), output=str(root / "S3/renderlab"), root=str(root / "S3"),
                                              score_script=None, modes=MODES, cases=["sun-r0.04"])),
        "quarry": ("gpu_virtual_quarry", dict(quarry, output=str(root / "quarry-run"))),
        "quarry-timing": ("gpu_virtual_quarry", dict(quarry, output=str(root / "quarry-timing"), reference=False)),
        # N8b decision traces (VSM_DEBUG) against Interactive on the same directions.
        "diag-s1": ("gpu_contact_fold_virtual_diagnose", dict(scene=str(S1 / "fixture/scene.json"), output=str(root / "diag"))),
        "diag-s2": ("gpu_motion_virtual_diagnose", dict(scene=str(MOTION / "S2/fixture/scene.json"), output=str(root / "diag"),
                                                     case="camera-0.25px-r0.04", frames=[8, 15, 16, 17, 18, 20, 24, 28, 31, 32, 33, 40])),
        "diag-quarry": ("gpu_virtual_quarry_diagnose", dict(quarry, output=str(root / "diag"))),
        "score-s1": ("gpu_contact_fold_batch", dict(scene=str(S1 / "fixture/scene.json"), output=str(root / "S1/renderlab"), modes=["Virtual"])),
        "score-s2": ("gpu_motion_bench_batch", dict(scene=str(MOTION / "S2/fixture/scene.json"), output=str(root / "S2/renderlab"), root=str(root / "S2"),
                                                    score_script=None, modes=["Virtual"], cases=S2_CASES, skip_reference=True, repeats=repeats)),
        "score-s3": ("gpu_motion_bench_batch", dict(scene=str(MOTION / "S3/fixture/scene.json"), output=str(root / "S3/renderlab"), root=str(root / "S3"),
                                                    score_script=None, modes=["Virtual"], cases=["sun-r0.04"], skip_reference=True, repeats=repeats)),
        "score-quarry": ("gpu_virtual_quarry", dict(quarry, output=str(root / "quarry-run"), modes=["interactive", "n8"])),
        "score-quarry-n8": ("gpu_virtual_quarry", dict(quarry, output=str(root / "quarry-run"), modes=["n8"])),
        # N8b variant screens (counts against Interactive, shadow GPU ms); N8B_VARIANTS is
        # a JSON {name: {"constants": {...}, "virtual": {...}}}.
        "screen-s2": ("gpu_motion_virtual_screen", dict(scene=str(MOTION / "S2/fixture/scene.json"), output=str(root / "screen"),
                                                       case="camera-0.25px-r0.04", frame=28, turn_degrees=0.0167, variants=variants)),
        "screen-s3": ("gpu_motion_virtual_screen", dict(scene=str(MOTION / "S3/fixture/scene.json"), output=str(root / "screen"),
                                                       case="sun-r0.04", frame=0, turn_degrees=0.0167, variants=variants)),
        "screen-quarry": ("gpu_virtual_quarry_screen", dict(quarry, output=str(root / "screen"), turn_degrees=0.0286, variants=variants,
                                                         dump=os.environ.get("N8E_DUMP") == "1")),
        "diag-s3": ("gpu_motion_virtual_diagnose", dict(scene=str(MOTION / "S3/fixture/scene.json"), output=str(root / "diag"),
                                                     case="sun-r0.04", frames=[0])),
        "diag-quarry-variants": ("gpu_virtual_quarry_diagnose", dict(quarry, output=str(root / "diag"), variants=dict(
            s32=dict(march_samples=32), t1=dict(texels_per_pixel_log2=1.0), s32t1=dict(march_samples=32, texels_per_pixel_log2=1.0)))),
    }


def main():
    binary, root = Path(sys.argv[1]), Path(sys.argv[2])
    steps = sys.argv[3:] or ["fixture", "s1", "s2", "s3", "quarry"]
    plan = manifests(root)
    record = []
    for step in steps:
        test, manifest = plan[step]
        # Historical reference benches retain Exact; an explicit Surface selection wins.
        manifest = dict(manifest, virtual={"lookup": "exact", **json.loads(os.environ.get("N8_VIRTUAL", "{}"))})
        if "N8B_CONSTANTS" in os.environ:
            # Lookup override constants (JSON) for the diagnosis traces.
            manifest = dict(manifest, constants=json.loads(os.environ["N8B_CONSTANTS"]))
        path = root / f"manifest-{step}.json"
        path.parent.mkdir(parents=True, exist_ok=True)
        if step in ("score-s2", "score-s3"):
            link_references(root, step[-2:].upper(), manifest["cases"])
        path.write_text(json.dumps(manifest, indent=2))
        env = dict(os.environ, RENDERLAB_PRIMARY_MANIFEST=str(path))
        started = time.monotonic()
        print(f"N8 batch step {step}: {test}", flush=True)
        # The filter is a substring: skip longer-named siblings (gpu_virtual_quarry_screen...).
        result = subprocess.run([str(binary), test, "--skip", test + "_", "--ignored", "--nocapture", "--test-threads=1"], env=env)
        record.append(dict(step=step, test=test, exit=result.returncode, seconds=time.monotonic() - started))
        (root / "batch.json").write_text(json.dumps(record, indent=2))
        if result.returncode:
            raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
