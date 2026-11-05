"""Production receiver fixtures/references, each in a bounded offscreen GPU lease."""
import argparse
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
OUT = Path("E:/renderlab-work/receiver-adoption")
R1 = Path("D:/Coding/renderlab-work/m6-exact-quality/r1")
PREFIX = "renderer::fresh_shadow_tests::primary_ray_gpu_tests::"


def main():
    global OUT
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--phase", choices=("fixtures", "quarry", "primary", "replay", "static", "timing"), required=True)
    parser.add_argument("--case")
    parser.add_argument("--attempt", default="1")
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--output-root", type=Path, default=OUT)
    parser.add_argument("--runner", type=Path, default=ROOT / "scripts/m6-receiver-adoption-run.py")
    args = parser.parse_args()
    OUT = args.output_root
    OUT.mkdir(parents=True, exist_ok=True)
    binary = args.binary or next((OUT / "target/release/deps").glob("renderlab-*.exe"))
    template = json.loads(Path("D:/Coding/renderlab-work/m6-exact-quality/f0/quarry-minimal-p0.json").read_text())
    jobs = []
    if args.phase == "fixtures":
        for kind in ("flat", "convex"):
            for distance in (1000, 10000):
                for incidence in (80, 85, 89):
                    jobs.append((f"{kind}-{distance}-{incidence}-viewed", dict(workload=kind, distance=distance, incidence=incidence)))
        jobs += [(f"{kind}-viewed", dict(workload=kind)) for kind in ("sawtooth", "thin", "stress")]
    elif args.phase == "quarry":
        jobs = [(f"quarry-viewed-p{pose}", dict(workload="quarry", pose=pose)) for pose in (0, 1, 2)]
    else:
        jobs = [(args.phase, dict(workload="quarry"))]
    for name, fields in jobs:
        if args.case and name != args.case:
            continue
        directory = OUT / name
        if args.phase in ("fixtures", "quarry"):
            test = PREFIX + "film_reference_tests::receiver_policy_tests::gpu_receiver_policy_r1_equivalence"
            if (directory / "r1-equivalence.json").exists():
                print("reuse", name, flush=True)
                continue
        else:
            test = PREFIX + {
                "primary": "gpu_primary_traced_quarry_acceptance",
                "replay": "soft_shadow_probe_tests::soft_shadow_replay_tests::gpu_soft_shadow_mask_replay",
                "static": "soft_shadow_probe_tests::soft_shadow_acceptance_tests::gpu_soft_shadow_traced_acceptance",
                "timing": "soft_shadow_probe_tests::soft_shadow_acceptance_tests::gpu_soft_shadow_complete_paired_timing",
            }[args.phase]
        manifest = template | fields | dict(output=str(directory), receiver_policy="production", minimal=False,
            r1_reference=str(R1 / name), replay=str(OUT / "replay"), reference=str(OUT / "primary"), static_repeats=32)
        path = OUT / f"{name}.json"
        path.write_text(json.dumps(manifest, indent=2))
        command = [sys.executable, "-B", str(args.runner), "--label", name + "-" + args.attempt,
            "--manifest", str(path), "--minutes", "10", "--wait-seconds", "45", "--", str(binary), test,
            "--ignored", "--exact", "--nocapture", "--test-threads=1"]
        result = subprocess.run(command, cwd=ROOT, creationflags=subprocess.CREATE_NO_WINDOW)
        if result.returncode:
            return result.returncode
        if args.phase == "fixtures":
            command = [sys.executable, "-B", str(ROOT / "scripts/m6-receiver-fixtures.py"), str(directory)]
            result = subprocess.run(command, cwd=ROOT, creationflags=subprocess.CREATE_NO_WINDOW)
            if result.returncode:
                return result.returncode
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
