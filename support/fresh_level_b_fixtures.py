"""S1-S4 on the traced thin/contact and speck/wire GPU fixtures.

Uses the unchanged e1c7822 analytic disk oracle and the same Level-B check()
rules as the quarry. There are no U-bounds without native Unreal fixture data.
"""
import argparse, json, math, sys, time
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import numpy as np
import fresh_gate as oracle
import fresh_level_b as gate
import fresh_level_b_metrics as lb


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ("thin", "stress", "frozen", "output"):
        parser.add_argument("--" + key, type=Path, required=True)
    args = parser.parse_args()
    start = time.perf_counter()
    args.output.mkdir(parents=True, exist_ok=True)
    result, failures = {}, []
    shape = (192, 256)
    for work, candidate in (("thin", args.thin), ("stress", args.stress)):
        rectangles = json.loads((args.frozen / work / "rectangles.json").read_text())
        errors, rows = {}, {}
        for pose in range(5):
            world = np.fromfile(candidate / f"pose-{pose}-world.f32", dtype="<f4").reshape(*shape, 4)
            normal = np.fromfile(candidate / f"pose-{pose}-normal.f32", dtype="<f4").reshape(*shape, 4)
            cache = args.output / f"{work}-pose-{pose}-reference.npz"
            if not cache.exists():
                ref, sigma = oracle.analytic_reference(world, rectangles)
                np.savez_compressed(cache, ref=ref, sigma=sigma)
            reference = np.load(cache)
            d = dict(ref=reference["ref"], sigma=reference["sigma"], depth=world[..., 3],
                     cover=normal[..., 3] != 0, front=normal[..., 3] != 0, ndl=np.ones(shape))
            pops = lb.populations(d)
            def counts(seed, kind):
                return np.fromfile(candidate / f"pose-{pose}-seed-{seed}-{kind}.u32", dtype="<u4").reshape(shape) / 256
            spread = [lb.stats(counts(s, "production"), d, pops, d["cover"], domains=("front",))[0]["front"] for s in range(65, 81)]
            rho0 = float(np.median([gate.rho(x) for x in spread]))
            bound = math.sqrt(.9*rho0*rho0 + .9)
            for seed in range(16):
                s, err = lb.stats(counts(seed, "1x8"), d, pops, d["cover"], domains=("front",))
                prod = lb.stats(counts(seed, "production"), d, pops, d["cover"], domains=("front",))[0]["front"]
                # An absent fixture population has no error or bias to gate.
                # Keep n=0 visible in the report instead of inventing receivers.
                for statistics in (s["front"], prod):
                    for population in gate.U_RMS:
                        if statistics[population]["n"] == 0:
                            statistics[population].update(rms=0.0, mean=0.0)
                bounds = {f"U-rms-{p}": float("inf") for p in gate.U_RMS} | {"S-rho": bound}
                checks = {k: v for k, v in gate.check(s["front"], bounds, prod).items() if k.startswith("S-")}
                key = f"{pose}/{seed}"
                rows[key] = dict(checks=checks, statistics=s["front"], rho0=rho0)
                failures.extend(f"{work}/{key}/{k}" for k, v in checks.items() if not v[2])
                errors[pose, seed] = err, pops["partial"]
            print(work, pose, round(time.perf_counter()-start, 2), flush=True)
        motion = []
        for pose in range(4):
            for seed in range(16):
                ea, pa = errors[pose, seed]; eb, pb = errors[pose+1, seed]
                m = pa & pb
                motion.append(float(np.sqrt(((eb-ea)[m]**2).mean())))
        # The return pose also replays every required seed, beyond the complete
        # seed-zero HDR return assertion in the GPU suite.
        for seed in range(16):
            assert (candidate / f"pose-0-seed-{seed}-1x8.u32").read_bytes() == (candidate / f"pose-3-seed-{seed}-1x8.u32").read_bytes()
        result[work] = dict(cases=rows, motion_rms=motion, return_counts_exact=True)
    result.update(failures=failures, passed=not failures, seconds=time.perf_counter()-start)
    (args.output / "fixtures.json").write_text(json.dumps(result, indent=2, allow_nan=False))
    print("fixtures", "PASS" if not failures else failures, result["seconds"], flush=True)
    if failures: raise SystemExit(1)


if __name__ == "__main__":
    main()
