"""CPU-only old/new receiver evidence: frozen gates, film metrics and paired timing."""
import argparse
import csv
import importlib.util
import json
from pathlib import Path
import subprocess
import sys

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
OUT = Path("E:/renderlab-work/receiver-adoption")
sys.path.insert(0, str(ROOT / "support"))
import fresh_gate
import fresh_level_b_metrics as lb


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


film = module("film_score", "m6-film-score.py")
r1 = module("receiver_score", "m6-receiver-score.py")


def save(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False))


def gates():
    # The exact candidate is the production estimator, as in the retained original
    # primary gate. Synthetic masks prove its identity to the gate's replay only.
    directory = OUT / "primary"
    for pose in range(3):
        normal = np.fromfile(directory / f"pose-{pose}-normal.f32", dtype="<f4").reshape(*lb.SHAPE, 4)
        covered = normal[..., 3] != 0
        for seed in range(4):
            prefix = directory / f"pose-{pose}-seed-{seed}"
            counts = np.fromfile(str(prefix) + "-production.u32", dtype="<u4").reshape(lb.SHAPE)
            occ = np.where(covered, 256-counts.astype(np.int64), 0).reshape(-1)
            words = np.zeros((occ.size, 8), dtype="<u4")
            bits = np.arange(256)
            for start in range(0, occ.size, 65536):
                n = occ[start:start+65536, None]
                mask = (n >= 1) & ((bits == 0) | ((bits >= 64) & (bits < 64+np.minimum(n-1, 192))) | ((bits >= 1) & (bits < 1+np.maximum(n-193, 0))))
                assert np.array_equal(mask.sum(1), n[:, 0])
                words[start:start+65536] = np.packbits(mask, axis=1, bitorder="little").view("<u4")
            hits = words.reshape(*lb.SHAPE, 8)
            hits.tofile(str(prefix) + "-hits.u32")
            pop = np.unpackbits(hits.view("u1").reshape(*lb.SHAPE, 32), axis=-1, bitorder="little")[..., :64].sum(-1)
            np.where(covered, 64-pop, 0xffffffff).astype("<u4").tofile(str(prefix) + "-prefix.u32")
            counts.tofile(str(prefix) + "-adaptive.u32")
        for seed in range(4):
            source = directory / (f"pose-{pose}-production.rgba16f" if seed == 0 else f"pose-{pose}-seed-{seed}-production.rgba16f")
            target = directory / (f"pose-{pose}-adaptive.rgba16f" if seed == 0 else f"pose-{pose}-seed-{seed}-adaptive.rgba16f")
            target.write_bytes(source.read_bytes())
    try:
        result = fresh_gate.prepare(directory, "quarry")
    except RuntimeError:
        # This task explicitly requests rescoring both frozen gates. Preserve the
        # failed Level-A result and continue scoring; never alter its thresholds.
        result = json.loads((directory / "gate.json").read_text())
    print("Level A", result["numeric_pass"], flush=True)
    fresh_gate.inspection(directory, "quarry")
    command = [sys.executable, "-B", str(ROOT / "support/fresh_level_b.py"), "--reference", str(directory),
        "--candidate", str(OUT / "static"), "--native", str(film.NATIVE), "--output", str(OUT / "level-b")]
    subprocess.run(command, cwd=ROOT, check=False)


def timings():
    records = []
    for path in sorted(OUT.glob("paired-*/paired-timing.json")):
        data = json.loads(path.read_text())
        for mode in ("hard", "interactive", "exact"):
            samples = [s for s in data["samples"] if s["mode"] == mode]
            row = dict(run=path.parent.name, mode=mode, pairs=data["pairs"], stages={})
            for field in ("span_ms", "receiver_ms", "compute_ms", "shading_ms", "tone_ms"):
                pairs = {}
                for s in samples:
                    if s[field] is not None:
                        pairs.setdefault(s["pair"], {})[s["baseline"]] = s[field]
                if not pairs:
                    continue
                old = np.array([s[True] for s in pairs.values()])
                new = np.array([s[False] for s in pairs.values()])
                ratios = new / old
                rng = np.random.default_rng(767)
                bootstrap = np.median(rng.choice(ratios, size=(5000, len(ratios)), replace=True), axis=1)
                interval = np.quantile(bootstrap, [.025, .975]).tolist()
                row["stages"][field] = dict(old_median_ms=float(np.median(old)), new_median_ms=float(np.median(new)),
                    paired_ratio=float(np.median(ratios)), paired_ratio_95pct_interval=interval,
                    regression_beyond_noise=interval[0] > 1.0)
            records.append(row)
    save(OUT / "timing-summary.json", records)
    for row in records:
        print(row["run"], row["mode"], row["stages"]["span_ms"], flush=True)


def radiance_deltas(actual, reference):
    return {"linear":actual[..., :3]-reference[..., :3],
        **{f"codes_{stops}":r1.codes(actual, stops)-r1.codes(reference, stops) for stops in (0, 4)}}


def radiance_metrics(deltas, mask):
    row = dict(pixels=int(mask.sum()))
    delta = deltas["linear"][mask]
    row["linear_rms"] = float(np.sqrt(np.mean(delta**2))) if delta.size else 0.
    for stops in (0, 4):
        delta = deltas[f"codes_{stops}"][mask]
        row[f"codes_{stops}_rms"] = float(np.sqrt(np.mean(delta**2))) if delta.size else 0.
        row[f"codes_{stops}_pixels_over_4"] = int((np.max(np.abs(delta), axis=1) > 4).sum()) if delta.size else 0
    return row


def films():
    rows, radiance, equivalents = [], [], []
    images = OUT / "images"
    images.mkdir(exist_ok=True)
    old_root = Path("D:/Coding/renderlab-work/m6-exact-quality/f0")
    centres = OUT / "centres"
    for pose in range(3):
        old = film.decode(old_root / f"quarry-policy-p{pose}", pose)
        new_dir = OUT / f"quarry-viewed-p{pose}"
        new = film.decode(new_dir, pose)
        point = np.load(OUT / f"primary/pose-{pose}-reference.npz")["ref"]
        old_point = np.load(film.FROZEN / f"gate-final/reference/pose-{pose}-reference.npz")["ref"]
        for state, data, reference_point in (("old", old, old_point), ("new", new, point)):
            pops = film.populations(data, reference_point)
            pops["all-covered"] = data["cover"] & (data["n"] > 0)
            pops["image"] = np.ones(lb.SHAPE, bool)
            pops["multi-surface"] = data["cover"] & (data["n"] < 256)
            pops["background-centre"] = ~data["cover"]
            direct_dir = OUT / f"old-film-p{pose}" if state == "old" else new_dir
            direct = r1.radiance(direct_dir, pose, True) if (direct_dir / f"pose-{pose}.direct.bin").exists() else None
            for mode in ("hard", "exact", "interactive"):
                prefix = centres / f"pose-{pose}-{state}-{mode}"
                actual = np.fromfile(str(prefix)+".rgba16f", dtype="<f2").reshape(*lb.SHAPE, 4).astype(float)
                visibility = np.fromfile(str(prefix)+"-visibility.rgba16f", dtype="<f2").reshape(*lb.SHAPE, 4)[..., 0].astype(float)
                stats = film.statistics(visibility, data, pops, np.ones(lb.SHAPE, bool), domains=("all", "front"))
                for domain, populations in stats.items():
                    for name, stat in populations.items():
                        if isinstance(stat, dict) and "n" in stat:
                            rows.append(dict(pose=pose, policy=state, method=mode, domain=domain, population=name, **stat))
                row = dict(pose=pose, policy=state, method=mode, visibility=stats, radiance={})
                sun = np.fromfile(str(prefix)+".sun.rgba16f", dtype="<f2").reshape(*lb.SHAPE, 4).astype(float)
                full_delta = radiance_deltas(actual, data["hdr"])
                sun_delta = radiance_deltas(sun, direct) if direct is not None else None
                for name, mask in pops.items():
                    row["radiance"][name] = {"full":radiance_metrics(full_delta, mask)}
                    if sun_delta is not None:
                        row["radiance"][name]["sun"] = radiance_metrics(sun_delta, mask)
                radiance.append(row)
                print("film score", pose, state, mode, flush=True)
                for stops in (0, 4):
                    film.tone(actual, stops).save(images / f"pose-{pose}-{state}-{mode}-full-{stops}.png")
                    film.tone(sun, stops).save(images / f"pose-{pose}-{state}-{mode}-sun-{stops}.png")
                film.vis(visibility, data["cover"]).save(images / f"pose-{pose}-{state}-{mode}-visibility.png")
                film.error(visibility-data["ref"], data["cover"], np.ones(lb.SHAPE, bool)).save(images / f"pose-{pose}-{state}-{mode}-error.png")
            for label, hdr in (("film", data["hdr"]), ("direct-film", direct)):
                if hdr is not None:
                    for stops in (0, 4):
                        film.tone(hdr, stops).save(images / f"pose-{pose}-{state}-{label}-{stops}.png")
            film.vis(data["ref"], data["cover"]).save(images / f"pose-{pose}-{state}-film-visibility.png")
        equivalent = json.loads((new_dir / "r1-equivalence.json").read_text())
        equivalents.append(dict(pose=pose, comparisons=equivalent))
    fields = ["pose", "policy", "method", "domain", "population", "reference_n", "n", "missing_n", "rms", "rms_noise_subtracted", "mean", "se", "p99", "max", "noise_rms"]
    with (OUT / "film-populations.csv").open("w", newline="") as f:
        writer = csv.DictWriter(f, fields, extrasaction="ignore")
        writer.writeheader()
        writer.writerows(rows)
    save(OUT / "film-metrics.json", radiance)
    save(OUT / "r1-equivalences.json", equivalents)
    for row in radiance:
        print(row["pose"], row["policy"], row["method"], "partial", row["visibility"]["all"]["partial"]["rms"],
            "missed/false", row["visibility"]["all"]["missed_shadows"], row["visibility"]["all"]["false_shadows"],
            "image", row["radiance"]["image"], flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--phase", choices=("gates", "film", "timing"), required=True)
    args = parser.parse_args()
    {"gates":gates, "film":films, "timing":timings}[args.phase]()


if __name__ == "__main__":
    main()
