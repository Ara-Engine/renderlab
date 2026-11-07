"""Run N1c offscreen fixtures/benches under n1c-run.py's shared GPU/memory lease.

One process per device loads the quarry once, attaches its cooked clusterLOD package and
runs every cut bound (--lod) and software-raster extent (--extent). `*-no-rt` steps use a
device created without either experimental ray feature or any AS limit.

    py -3 -B scripts/n1c-batch.py <test binary> <root> quarry quarry-no-rt --lod -1 0.5 1
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

QUARRY = Path("D:/Coding/renderlab-work/m6-adaptive/gpu-phase-quarry/quarry-manifest.json")
DETAIL = Path("D:/Coding/projects/desktop/renderlab/measurements/quarry-quality/detail")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("binary", type=Path)
    p.add_argument("root", type=Path)
    p.add_argument("steps", nargs="*", default=["fixture"])
    p.add_argument("--timing", action="store_true")
    p.add_argument("--lod", type=float, nargs="+", default=[0.35])
    p.add_argument("--extent", type=float, nargs="+", default=[32])
    p.add_argument("--lift", type=float, nargs="+", default=[1])
    p.add_argument("--poses", type=int, nargs="+", default=[0, 1, 0])
    p.add_argument("--stage-repeats", type=int, default=8)
    p.add_argument("--no-lod", action="store_true", help="original geometry clusters only")
    p.add_argument("--no-reference", action="store_true")
    p.add_argument("--hash-only", action="store_true", help="snapshot counts/hit hashes without HDR/world dumps")
    p.add_argument("--caster-error", type=float, nargs="+", help="N8c caster cut errors (texels; negative: original geometry)")
    p.add_argument("--turn", type=int, default=0, help="N8c sun-turning frames timed after each configuration")
    p.add_argument("--move-frames", type=int, default=0, help="N8f camera-moving, then caster-moving frames timed after each configuration")
    p.add_argument("--move-batch", type=int, default=0, help="N8f placement (draw batch) the caster-moving frames move")
    a = p.parse_args()
    a.root.mkdir(parents=True, exist_ok=True)
    record = []
    for step in a.steps:
        no_rt = step.endswith("-no-rt")
        name = step.removesuffix("-no-rt")
        m = dict(output=str(a.root / step), no_rt=no_rt, validate=not a.timing, timing=a.timing,
                 hash_only=a.hash_only)
        if name == "fixture":
            test = "gpu_visibility_fixture"
        elif name == "quarry":
            test = "gpu_visibility_quarry"
            m.update(json.loads(QUARRY.read_text()), output=str(a.root / step),
                     reference=not (a.timing or a.no_reference), lod_pixels=a.lod, software_extent=a.extent, lod_lift=a.lift,
                     poses=a.poses, stage_repeats=a.stage_repeats, turn_frames=a.turn,
                     move_frames=a.move_frames, move_batch=a.move_batch)
            if a.caster_error:
                m["caster_error"] = a.caster_error
            if os.environ.get("N8_VIRTUAL"):
                m["virtual"] = json.loads(os.environ["N8_VIRTUAL"])
            # N8e: {name: settings}, every configuration once per variant (labels -v{name}).
            if os.environ.get("N8E_VARIANTS"):
                m["virtual_variants"] = json.loads(os.environ["N8E_VARIANTS"])
            if not a.no_lod:
                m.update(virtual_cache=str(DETAIL / "cache"), virtual_manifest=str(DETAIL / "virtual.manifest"))
        else:
            test = "gpu_visibility_bench"
            scene = Path("E:/renderlab-work/contact-fold/fixture/scene.json") if name == "S1" else Path(f"E:/renderlab-work/motion-benches/{name}/fixture/scene.json")
            m["scene"] = str(scene)
        path = a.root / f"manifest-{step}.json"
        path.write_text(json.dumps(m, indent=2))
        env = dict(os.environ, RENDERLAB_PRIMARY_MANIFEST=str(path))
        if a.timing:
            env["VK_INSTANCE_LAYERS"] = ""
        start = time.monotonic()
        print(step, test, flush=True)
        result = subprocess.run([str(a.binary), test, "--ignored", "--nocapture", "--test-threads=1"], env=env)
        record.append(dict(step=step, exit=result.returncode, seconds=time.monotonic() - start))
        (a.root / "batch.json").write_text(json.dumps(record, indent=2))
        if result.returncode:
            raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
