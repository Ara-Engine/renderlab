"""M7-TR1: write one GPU plan (manifests plus a runner plan for scripts/m7-tr1-run.py plan).

  py -3 -B scripts/m7-tr1-plan.py <label> --paths rt,nonrt --variants variants.json
         [--visits N] [--binary tr1a] [--frames reference|all]

Each step runs `gpu_tr1_sequence` on one path with the registered Q-walk, the T1b black-
primary lighting (TR0's matched inputs) and the frames that have physical references.
Variants are {name: Reconstruction overrides}; see src/temporal_reconstruction_tests.rs.
"""
import argparse
import json
from pathlib import Path

# Plans and runs on D: since TR1b (bulk output; milestone-7.md). Earlier evidence stays in E:/renderlab-work/m7-tr1.
ROOT = Path("D:/Coding/renderlab-work/m7-tr1")
TR0 = Path("E:/renderlab-work/m7-tr0")
S2G_SCENE = "E:/renderlab-work/m7-u0b/scenes/S2G/scene.json"
TEST = ("renderer::fresh_shadow_tests::primary_ray_gpu_tests::visibility_buffer_tests::"
        "temporal_reconstruction_tests::gpu_tr1_sequence")
QUARRY = {
    "asset": "D:/Coding/projects/desktop/renderlab/measurements/quarry-quality/detail/full.rla",
    "limits": "D:/Coding/projects/desktop/renderlab/measurements/quarry-quality/detail/limits.json",
    "cameras": "D:/Coding/renderlab-work/m6-hardware-msaa/quarry-cameras.json",
    "lighting": str(TR0 / "runs/t1b-black-b/lighting-black-background.json"),
    "environment": "D:/Coding/renderlab-work/m5-independent-production/sun128.rle2",
    "virtual_cache": "D:\\Coding\\projects\\desktop\\renderlab\\measurements\\quarry-quality\\detail\\cache",
    "virtual_manifest": "D:\\Coding\\projects\\desktop\\renderlab\\measurements\\quarry-quality\\detail\\virtual.manifest",
    "sequence": "E:/renderlab-work/t1-gpu/sequences/q-walk.json",
}


def reference_frames():
    bar = json.loads((TR0 / "bar/tr1-bar.json").read_text())
    case = next(c for c in bar["cases"] if c["label"] == "w-e-r-c1")
    return [f for f in range(case["total_frames"]) if f not in set(case["missing_frames"])]


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("label")
    parser.add_argument("--paths", default="rt,nonrt")
    parser.add_argument("--variants", required=True, help="JSON file or inline JSON object")
    parser.add_argument("--visits", type=int, default=0)
    parser.add_argument("--binary", default="tr1a")
    parser.add_argument("--frames", default="reference")
    parser.add_argument("--no-replay", action="store_true")
    parser.add_argument("--no-write", action="store_true", help="hash the frames without writing them")
    parser.add_argument("--track", default="q-walk", help="q-walk, or a TR0 input track (q-disocclusion, q-sun-fast, s2g-*)")
    args = parser.parse_args()
    text = args.variants
    variants = json.loads(Path(text).read_text() if not text.lstrip().startswith("{") else text)
    plan_dir = ROOT / "plans" / args.label
    plan_dir.mkdir(parents=True, exist_ok=False)
    # reference: every Q-walk frame with a physical reference (93); checkpoints: T1b's 60
    # scored frames, which carry the fixed age masks (screens); all: every frame; none: no
    # capture (cost runs).
    frames = {"reference": reference_frames,
              "checkpoints": lambda: json.loads((TR0 / "runs/t1b-black-b/manifest.json").read_text())["scored"],
              "all": lambda: None, "none": lambda: []}.get(args.frames, lambda: [int(f) for f in args.frames.split(",")])()
    steps = []
    for step in args.paths.split(","):
        # A step is a path, optionally with a repeat suffix (rt-b) for cross-process identity.
        path = step.split("-b")[0] if step.endswith("-b") else step
        manifest = dict(QUARRY, path=path, output=str(ROOT / "runs" / args.label / step / "data"),
                        variants=variants, visits=args.visits, replay=not args.no_replay, validate=False,
                        write_frames=not (args.no_write or step.endswith("-b") or path == "nonrt-on-rt"))
        if args.track != "q-walk":
            # TR0's registered input timelines; S2G replaces the quarry with its bench scene.
            manifest["sequence"] = str(TR0 / "inputs" / (args.track + ".json"))
            if args.track.startswith("s2g-"):
                manifest = {k: v for k, v in manifest.items() if k not in QUARRY or k == "sequence"}
                manifest["scene"] = S2G_SCENE
        if frames is not None:
            manifest["frames"] = frames
        target = plan_dir / f"manifest-{step}.json"
        target.write_text(json.dumps(manifest, indent=1) + "\n")
        steps.append(dict(name=step, binary=args.binary, test_name=TEST,
                          env={"RENDERLAB_PRIMARY_MANIFEST": str(target)}))
    (plan_dir / "plan.json").write_text(json.dumps(steps, indent=1) + "\n")
    print(plan_dir / "plan.json")


if __name__ == "__main__":
    main()
