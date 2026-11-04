"""Level-B gate: frozen U/S rules from the 2026-11-04 design, on GPU counts.

The check(), bounds() and rho() rules below are copied unchanged from gate_b.py.
Reference counts are immutable traced-primary acceptance data; native masks are
rescored on those receivers. No filtering, history or threshold relaxation.
"""
import argparse, json, math, sys, time
from pathlib import Path
import numpy as np
sys.path.insert(0, str(Path(__file__).resolve().parent))
import fresh_level_b_metrics as lb

U_RMS = ("partial", "narrow", "wide", "isolated", "silhouette", "faint", "lit", "occluded", "contact")
BIAS_POPS = ("partial", "narrow", "wide", "faint", "isolated")
Z_BIAS = 4.0          # |mean| <= |production same-seed mean| + 4 SE (family of 60 tests per candidate)
COHERENT_SHARE = 0.10  # along-contour coherent share of error variance; production worst draw 1-6%, VSM 27%


def rho(s):
    pr = s["partial"]["rms"]
    return s["contour"]["rms"] / (pr / 3) if pr > 0 else 0.0


def bounds(profile, pose, rho0):
    c = profile[str(pose)]["cases"]
    best = lambda pop, k="rms": min(c[x]["front"][pop][k] for x in ("vsm", "hrt1"))
    b = {f"U-rms-{p}": best(p) for p in U_RMS}
    motion = profile["pose_change_0_1"]
    b["U-motion"] = min(motion[x]["front"]["rms"] for x in ("vsm", "hrt1"))
    b["S-rho"] = math.sqrt((1 - COHERENT_SHARE) * rho0 ** 2 + 9 * COHERENT_SHARE)
    return b


def check(s, b, prod):
    out = {}
    for p in U_RMS:
        out[f"U-rms-{p}"] = (s[p]["rms"], b[f"U-rms-{p}"], s[p]["rms"] <= b[f"U-rms-{p}"])
    f = s["flips"]
    out["S-flips"] = (f["lit_dark_gt_0.5"] + f["umbra_lit_gt_0.5"], 0, f["lit_dark_gt_0.5"] + f["umbra_lit_gt_0.5"] == 0)
    out["S-contact"] = (f["contact_lost_gt_0.5"], 0, f["contact_lost_gt_0.5"] == 0)
    for p in BIAS_POPS:
        se = s[p]["rms"] / math.sqrt(max(s[p]["n"], 1))
        lim = abs(prod[p]["mean"]) + Z_BIAS * max(se, 1e-6)
        out[f"S-bias-{p}"] = (abs(s[p]["mean"]), lim, abs(s[p]["mean"]) <= lim)
    out["S-rho"] = (rho(s), b["S-rho"], rho(s) <= b["S-rho"])
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ("reference", "candidate", "native", "output"):
        parser.add_argument("--" + key, type=Path, required=True)
    args = parser.parse_args()
    start = time.perf_counter()
    args.output.mkdir(parents=True, exist_ok=True)
    lb.R, lb.NATIVE, lb.REFERENCE = args.reference, args.native, args.output / "reference"
    lb.REFERENCE.mkdir(exist_ok=True)
    # Cache only derived references, leaving the registered evidence untouched.
    for pose in range(3):
        cache = lb.REFERENCE / f"pose-{pose}-reference.npz"
        if not cache.exists():
            total = np.zeros(lb.SHAPE); square = np.zeros(lb.SHAPE)
            for seed in range(1, 65):
                v = lb.counts(pose, seed) / 256
                total += v; square += v*v
            world = np.fromfile(lb.R / f"pose-{pose}-world.f32", dtype="<f4").reshape(*lb.SHAPE, 4)
            normal = np.fromfile(lb.R / f"pose-{pose}-normal.f32", dtype="<f4").reshape(*lb.SHAPE, 4)
            np.savez_compressed(cache, ref=total/64, sigma=np.sqrt(np.maximum(0, (square-total*total/64)/63)),
                                depth=world[..., 3], cover=normal[..., 3] != 0)
    data = {p: lb.load_pose(p) for p in range(3)}
    pops = {p: lb.populations(data[p]) for p in data}
    native, errors = {}, {}
    for pose in data:
        cases = {}
        for case in ("vsm", "hrt1"):
            nd = np.load(lb.native_dir(case, pose) / "native.npz")
            valid = nd["valid"] & data[pose]["cover"]
            s, err = lb.stats(nd["visibility"], data[pose], pops[pose], valid, domains=("front",))
            cases[case] = s
            errors[case, pose] = (err, valid)
        native[str(pose)] = {"cases": cases, "covered": int(data[pose]["cover"].sum()),
                             "front": int(data[pose]["front"].sum()),
                             "excluded": int((data[pose]["cover"] & ~data[pose]["front"]).sum())}
    native["pose_change_0_1"] = {}
    for case in ("vsm", "hrt1"):
        ea, va = errors[case, 0]; eb, vb = errors[case, 1]
        m = pops[0]["partial"] & pops[1]["partial"] & data[0]["front"] & data[1]["front"] & va & vb
        native["pose_change_0_1"][case] = {"front": {"rms": float(np.sqrt(((eb-ea)[m]**2).mean()))}}
    result = {"bounds": {}, "cases": {}, "motion": [], "native_rescore": native}
    failures = []
    candidate_errors = {}
    for pose in data:
        d, p = data[pose], pops[pose]
        draws = [lb.stats(lb.counts(pose, seed)/256, d, p, d["cover"], domains=("front",))[0]["front"]
                 for seed in range(65, 81)]
        rho0 = float(np.median([rho(x) for x in draws]))
        b = bounds(native, pose, rho0)
        result["bounds"][str(pose)] = b | {"rho0": rho0}
        for seed in range(4):
            actual = np.fromfile(args.candidate / f"pose-{pose}-seed-{seed}-interactive.u32", dtype="<u4").reshape(lb.SHAPE)
            prod = lb.stats(lb.counts(pose, seed)/256, d, p, d["cover"], domains=("front",))[0]["front"]
            s, err = lb.stats(actual/256, d, p, d["cover"], domains=("front",))
            checks = check(s["front"], b, prod)
            key = f"{pose}/{seed}"
            result["cases"][key] = {"checks": checks, "statistics": s["front"]}
            failures.extend(f"{key}/{k}" for k, value in checks.items() if not value[2])
            candidate_errors[pose, seed] = err
        print("gate pose", pose, "complete", round(time.perf_counter()-start, 2), flush=True)
    for a, b in ((0, 1), (1, 2)):
        for seed in range(4):
            m = pops[a]["partial"] & pops[b]["partial"] & data[a]["front"] & data[b]["front"]
            rms = float(np.sqrt(((candidate_errors[b, seed]-candidate_errors[a, seed])[m]**2).mean()))
            bound = result["bounds"]["0"]["U-motion"]
            passed = rms <= bound
            result["motion"].append({"poses": [a, b], "seed": seed, "rms": rms, "bound": bound, "pass": passed})
            if not passed: failures.append(f"{a}->{b}/{seed}/U-motion")
    acceptance = json.loads((args.candidate / "acceptance.json").read_text())
    result["S5"] = acceptance["repeat_exact"] and acceptance["return_exact"]
    if not result["S5"]: failures.append("S5")
    result.update(failures=failures, passed=not failures, seconds=time.perf_counter()-start)
    (args.output / "gate.json").write_text(json.dumps(result, indent=2, allow_nan=False))
    print("Level B", "PASS" if not failures else failures, "seconds", result["seconds"], flush=True)
    if failures: raise SystemExit(1)


if __name__ == "__main__":
    main()
