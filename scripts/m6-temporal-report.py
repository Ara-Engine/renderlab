"""Print the T1 CPU-phase tables (markdown) from m6-temporal-replay.py's JSON.

py -3 -B scripts/m6-temporal-report.py D:/Coding/renderlab-work/m6-exact-quality/t1/temporal-replay.json
"""
import json
import sys

POPS = ("partial", "faint", "narrow", "contact", "isolated", "lit", "occluded", "backlit",
        "multi-surface", "background-centre", "all-covered", "image")


def fmt(x, d=3):
    return "—" if x is None else f"{x:.{d}f}"


def main():
    r = json.load(open(sys.argv[1]))
    print("noise model:", {k: r["noise_model"][k] for k in ("partial_rms", "partial_n", "lit_nonzero", "umbra_nonzero")})
    for pose, p in r["poses"].items():
        print(f"\n## pose {pose} checks", p["checks"])
        print(f"\n### pose {pose}: N=8 / first N <= bar, ordering x noise (L0 image, L4 image, D4 image, F partial)")
        print("| Ordering | Rays | L0 image N=8 | first | L4 image N=8 | first | D4 image N=8 | first | F partial N=8 | first |")
        print("|---|---|---:|---:|---:|---:|---:|---:|---:|---:|")
        for key, rows in p["summary"].items():
            order, noise = key.split("/")
            cells = []
            for ch, pop in (("L0", "image"), ("L4", "image"), ("D4", "image"), ("F", "partial")):
                row = rows.get(f"{ch}/{pop}")
                cells += [fmt(row["N8"]), str(row["first_below_bar"])] if row else ["—", "—"]
            print(f"| {order} | {noise} | " + " | ".join(cells) + " |")
        best = p["summary"]["centre-progressive-scrambled/8-rays"]
        for ch in ("L0", "L4", "D0", "D4", "F"):
            print(f"\n### pose {pose} {ch}: centre-progressive-scrambled, 8 rays")
            print("| Population | E frozen | E viewed proxy | Interactive | N=1 | N=2 | N=4 | N=8 | N=16 | N=8 / bar | first <= bar | beta 4-16 | model N=32 |")
            print("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
            for pop in POPS:
                row = best.get(f"{ch}/{pop}")
                if not row:
                    continue
                print(f"| {pop} | {fmt(row['exact'])} | {fmt(row['exact_viewed_proxy'])} | {fmt(row['interactive'])} | {fmt(row['N1'])} | {fmt(row['N2'])} | {fmt(row['N4'])} | "
                      f"{fmt(row['N8'])} | {fmt(row['N16'])} | {fmt(row['ratio8_bar'], 2)} | {row['first_below_bar']} | {fmt(row['beta_4_16'], 2)} | {fmt(row['model32'])} |")
        print(f"\n### pose {pose}: single-sample errors by lattice position (L0 all-covered, L0 image, D4 all-covered, F partial)")
        for i, s in sorted(p["single"].items(), key=lambda kv: int(kv[0])):
            print(i, fmt(s["L0"]["all-covered"]), fmt(s["L0"]["image"]), fmt(s["D4"]["all-covered"]), fmt(s["F"]["partial"]))
        if "motion" in p:
            print(f"\n### pose {pose}: motion model (pan, x), totals in codes; bar = min(E frozen, E viewed proxy)")
            print("| f | alpha | equiv. iid N | L0 all-covered bias / noise / total | bar | L4 all-covered total | bar | D4 all-covered total | bar | L0 partial total | bar |")
            print("|---:|---:|---:|---|---:|---:|---:|---:|---:|---:|---:|")
            for key, row in p["motion"].items():
                f, a = key.split("/")
                c = row["L0/all-covered"]
                print(f"| {f[2:]} | {a[2:]} | {row['equivalent_iid_samples']:.1f} | {c['bias']:.3f} / {c['noise']:.3f} / {c['total']:.3f} | {c['exact']:.3f} | "
                      f"{row['L4/all-covered']['total']:.3f} | {row['L4/all-covered']['exact']:.3f} | {row['D4/all-covered']['total']:.3f} | {row['D4/all-covered']['exact']:.3f} | "
                      f"{row['L0/partial']['total']:.3f} | {row['L0/partial']['exact']:.3f} |")
            print("single step model:", p["motion_single_step"])
        if "motion_2d" in p:
            print(f"\n### pose {pose}: 2D motion model; total (bias/noise) in codes, bar = min(E frozen, E viewed proxy)")
            print("| Fetch | motion | alpha | L0 all-covered | L0 partial | L0 multi-surface | L0 background-centre | L4 all-covered | D4 all-covered | L0 image |")
            print("|---|---|---:|---|---|---|---|---|---|---|")
            for key, row in p["motion_2d"].items():
                parts = key.split("/")
                kind, motion, alpha = parts[0], parts[1] if parts[1] != "single" else parts[2], parts[2] if parts[1] != "single" else "single"
                cells = []
                for k in ("L0/all-covered", "L0/partial", "L0/multi-surface", "L0/background-centre", "L4/all-covered", "D4/all-covered", "L0/image"):
                    c = row[k]
                    cells.append(f"{c['total']:.2f} ({c['bias']:.2f}/{c['noise']:.2f}) / {c['bar']:.2f}")
                print(f"| {kind} | {motion} | {alpha} | " + " | ".join(cells) + " |")
        if "r1_reconciliation" in p:
            print(f"\n### pose {pose}: R1 reconciliation (saved order, Exact rays, linear HDR)", json.dumps({k: dict(first=v["first_below_exact"], exact=v["exact"], N4=v["curve"][3], N5=v["curve"][4]) for k, v in p["r1_reconciliation"].items()}))
    t = r["transfer_1cm"]
    print("\n## transfer 1 cm", t["motion"], t["depth_quantiles"], t.get("catmull_rom_subset"))
    print("| Rule | accepted all-covered | L0 all-covered | noise floor | E | E viewed | L0 partial | E | L0 multi-surface | E | L0 bg-centre | E | D4 all-covered | E |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for name, rows in t["rules"].items():
        a = rows["L0/all-covered"]
        cells = [fmt(a["accepted"], 4), fmt(a["rms_accepted"]), fmt(a["noise_floor"]), fmt(a["exact_on_accepted"]), fmt(a["exact_viewed_proxy_on_accepted"])]
        for ch, pop in (("L0", "partial"), ("L0", "multi-surface"), ("L0", "background-centre"), ("D4", "all-covered")):
            x = rows[f"{ch}/{pop}"]
            cells += [fmt(x["rms_accepted"]), fmt(x["exact_on_accepted"])]
        print(f"| {name} | " + " | ".join(cells) + " |")
    print("ghost:", json.dumps(t["ghost"], indent=0))


if __name__ == "__main__":
    main()
