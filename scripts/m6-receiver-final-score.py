"""CPU replay of the final single-upload receiver batch; no renderer launch."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import time

for name in ("OPENBLAS_NUM_THREADS", "OMP_NUM_THREADS", "NUMBA_NUM_THREADS"):
    os.environ[name] = "2"
import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--t1-score", type=Path, required=True)
    args = parser.parse_args()
    start = time.perf_counter()
    r1 = module("receiver_fixture_score", ROOT / "scripts/m6-receiver-fixtures.py")
    grazing = module("receiver_grazing_score", ROOT / "scripts/m6-receiver-grazing.py")
    grazing.set_num_threads(2)
    film = module("receiver_t1_score", args.t1_score)
    rows = {}
    for pose in range(3):
        d = film.decode(args.directory / "reference" / f"ref-p{pose}", pose)
        pops = film.populations(d)
        scorer = film.Scorer(d, pops)
        pose_rows = {}
        for mode in ("hard", "exact", "interactive"):
            estimates = {}
            for variant in ("old", "new"):
                prefix = args.directory / f"pose-{pose}-{variant}-{mode}"
                L = film.hdr(prefix.with_suffix(".rgba16f"))
                D = film.hdr(prefix.with_suffix(".sun.rgba16f"))
                V = film.hdr(prefix.with_suffix(".visibility.rgba16f"))[..., 0]
                score = scorer.score(L, D, V)
                score["S2_lost_contacts"] = int((pops["contact"] & ((V - d["ref"]) > .5)).sum())
                estimates[variant] = score
            pose_rows[mode] = estimates
        rows[str(pose)] = pose_rows
        panels = []
        for label in ("old", "new"):
            L = film.hdr(args.directory / f"pose-{pose}-{label}-interactive.rgba16f")
            image = Image.fromarray(np.uint8(np.clip(film.codes(L, 0), 0, 255)))
            image.save(args.directory / f"quarry-p{pose}-{label}.png")
            panels.append(image.resize((960, 540)))
        image = Image.fromarray(np.uint8(np.clip(film.codes(d["L"], 0), 0, 255)))
        image.save(args.directory / f"quarry-p{pose}-film.png")
        panels.append(image.resize((960, 540)))
        sheet = Image.new("RGB", (960, 1620))
        for i, image in enumerate(panels):
            sheet.paste(image, (0, i * 540))
        sheet.save(args.directory / f"quarry-p{pose}-comparison.png")
        (args.directory / "radiance-score.json").write_text(json.dumps(rows, indent=2, allow_nan=False))
        del scorer, pops, d
    fixtures = {}
    for directory in sorted((args.directory / "fixtures").iterdir()):
        if "-view-" in directory.name:
            if not (directory / "grazing-oracle.json").exists():
                grazing.score(directory)
            result = json.loads((directory / "grazing-oracle.json").read_text())
        else:
            result = json.loads((directory / "validation.json").read_text()) if (directory / "validation.json").exists() else r1.check(directory, "viewed")
            (directory / "validation.json").write_text(json.dumps(result, indent=2))
        fixtures[directory.name] = result
    timings = {}
    for path in args.directory.glob("timing-*/paired-timing.json"):
        data = json.loads(path.read_text())
        modes = {}
        for mode in ("hard", "exact", "interactive"):
            old = {s["pair"]: s for s in data["samples"] if s["mode"] == mode and s["baseline"]}
            new = {s["pair"]: s for s in data["samples"] if s["mode"] == mode and not s["baseline"]}
            modes[mode] = {field: dict(old_median_ms=float(np.median([s[field] for s in old.values()])),
                new_median_ms=float(np.median([s[field] for s in new.values()])),
                paired_ratio=float(np.median([new[i][field] / s[field] for i, s in old.items()])))
                for field in ("receiver_ms", "span_ms")}
        timings[path.parent.name] = modes
    result = dict(fixtures=fixtures, radiance_visibility=rows, timings=timings,
        seconds=time.perf_counter() - start, reference_population="same regenerated film reference for old and new")
    result["fixtures_passed"] = all(r["passed"] for r in fixtures.values())
    result["contact_passed"] = all(v["new"]["S2_lost_contacts"] <= v["old"]["S2_lost_contacts"]
        and v["new"]["F"]["contact"]["rms"] <= v["old"]["F"]["contact"]["rms"] + 1e-12
        for p in rows.values() for v in p.values())
    (args.directory / "final-score.json").write_text(json.dumps(result, indent=2, allow_nan=False))
    print(json.dumps({k: result[k] for k in ("fixtures_passed", "contact_passed", "seconds")}))
    if not result["fixtures_passed"] or not result["contact_passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
