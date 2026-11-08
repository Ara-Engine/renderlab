"""M7-R1: score RF1-RF10 captures (scripts/m7-r1-run.py `rf` steps) against R0a's static
references, its Unreal floors and its current-renderer baseline rows (docs/m7-r1.md).

    py -3 -B scripts/m7-r1-score.py D:/Coding/renderlab-work/m7-r1/runs/LABEL

R0a's scenes, reference archives and scores on E: are read only. Writes rf-scores.json,
rf-summary.md and inspection sheets (reference | R1 | 4x |difference| at -14 stops) into the
run directory. Same unfitted measures as R0a: no gain, denoising or pixel rejection.
"""
import importlib.util
import json
import sys
from pathlib import Path

import numpy as np

REPO = Path(__file__).resolve().parent.parent
FIXTURES = [f"RF{i}" for i in range(1, 11)]
UNREAL = ("sw-mesh-epic", "hw-hit-epic")


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), REPO / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ReferenceArgs:
    reference_label = "final"
    spp = 2048


def main(run):
    from PIL import Image
    r0a = load("m7-r0a")
    codes = load("m7-u0b-score").codes
    floors = json.loads((r0a.OUT / "scores" / "c-static.json").read_text())["rows"]
    floor = {(r["fixture"], r["pose"], r["config"]): r for r in floors}
    rows, lines = [], []
    lines.append("| Fixture / pose | Path | Before | R1 | UE SW Epic | UE HW-Hit Epic | Populations better / worse (of -14, -10) |")
    lines.append("|---|---|---:|---:|---:|---:|---|")
    (run / "sheets").mkdir(exist_ok=True)
    for fixture in FIXTURES:
        scene = json.loads((r0a.OUT / "scenes" / fixture / "scene.json").read_text())
        for pose in scene["poses"]:
            case = f"{fixture}-{pose['name']}"
            reference_path = r0a.static_reference_path(ReferenceArgs, fixture, case)
            with np.load(reference_path) as ref:
                h0, h1 = ref["L0"], ref["L1"]
                populations = {k[4:]: ref[k] for k in ref.files if k.startswith("pop_")}
            reference = (h0 + h1) / 2
            for root in sorted(run.glob("rf-*")):
                backend = root.name[3:]
                config = "renderlab-" + backend.split("-")[0]
                before = floor.get((fixture, pose["name"], config), {})
                if (root / f"{case}.unsupported.json").exists():
                    rows.append(dict(fixture=fixture, pose=pose["name"], config=config, status="unsupported",
                                     reason=json.loads((root / f"{case}.unsupported.json").read_text())["reason"]))
                    lines.append(f"| {case} | {backend} | {before.get('status', '—')} | unsupported | "
                                 + " | ".join(f"{floor[(fixture, pose['name'], u)]['measures']['reflector']['display_rms_-14']:.3f}" for u in UNREAL) + " | — |")
                    continue
                meta = json.loads((root / f"{case}.json").read_text())
                image = np.fromfile(root / f"{case}.f32", "<f4").reshape(meta["height"], meta["width"], 3)
                if not np.isfinite(image).all() or image.shape != h0.shape:
                    raise ValueError(f"{case} {backend}: dimensions or finite check failed")
                scores = {name: r0a.measures(image, h0, h1, mask) for name, mask in populations.items()}
                record = dict(fixture=fixture, pose=pose["name"], config=config, measures=scores,
                              features=r0a.feature_measures(image, reference, populations),
                              reference_archive_sha256=r0a.sha(reference_path))
                rows.append(record)
                better = worse = 0
                for name, value in scores.items():
                    old = before.get("measures", {}).get(name, {})
                    for stops in (-14, -10):
                        key = f"display_rms_{stops}"
                        if key in value and key in old:
                            better += value[key] < old[key] - 1e-6
                            worse += value[key] > old[key] + 1e-6
                old = before.get("measures", {}).get("reflector", {}).get("display_rms_-14")
                lines.append(f"| {case} | {backend} | {old if old is None else f'{old:.3f}'} | "
                             f"{scores['reflector']['display_rms_-14']:.3f} | "
                             + " | ".join(f"{floor[(fixture, pose['name'], u)]['measures']['reflector']['display_rms_-14']:.3f}" for u in UNREAL)
                             + f" | {better} / {worse} |")
                sheet = np.concatenate([codes(reference, -14), codes(image, -14),
                                        np.minimum(255, 4 * np.abs(codes(image, -14) - codes(reference, -14)))], axis=1)
                Image.fromarray(np.clip(sheet, 0, 255).astype("u1")).save(run / "sheets" / f"{case}-{backend}.png")
    (run / "rf-scores.json").write_text(json.dumps(dict(rows=rows, reference_label=ReferenceArgs.reference_label,
        scope="static primary-surface populations; no fitted gain, denoising or noisy-pixel removal"), indent=2))
    (run / "rf-summary.md").write_text("Whole-reflector display RMS at -14 stops, 8-bit F0 codes (lower is better). "
        "Before is R0a's current-renderer row (scores/c-static.json).\n\n" + "\n".join(lines) + "\n")
    print("\n".join(lines))


if __name__ == "__main__":
    main(Path(sys.argv[1]))
