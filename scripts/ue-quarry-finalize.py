"""CPU-only binding and inspection derivatives for the full-detail UE unit."""
import hashlib
import importlib.util
import json
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path("D:/Coding/renderlab-work/ue-m5/Quarry")
LEVEL_B = ROOT / "level-b-full"
REFERENCE = Path("D:/Coding/renderlab-work/m6-adaptive/gpu-phase-quarry/quarry")


def read(path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def finalize():
    gate = read(REFERENCE / "gate.json")
    for pose in range(3):
        if read(LEVEL_B / f"pose-{pose}-production.json") != gate["poses"][pose]["seeds"][0]["production_metrics"]:
            raise ValueError("production metrics differ from the worker")
    spec = importlib.util.spec_from_file_location("ue_extract", Path(__file__).with_name("ue-quarry-extract.py"))
    extract = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(extract)
    for mode in ("vsm", "hrt"):
        frames = []
        for pose in range(3):
            with Image.open(LEVEL_B / f"{mode}-pose-{pose}-steady-sheet.png") as image:
                overview = image.convert("RGB").resize((960, 810), Image.Resampling.NEAREST)
            overview.save(LEVEL_B / f"{mode}-pose-{pose}-overview.png")
            frames.append(overview)
            with np.load(ROOT / "extracted" / f"full-{mode}-capture-01" / f"pose-{pose}-steady-direct.npz") as data:
                Image.fromarray(extract.display(data["shadowed"], -8)).save(LEVEL_B / f"{mode}-pose-{pose}-lit-minus8.png")
        frames[0].save(LEVEL_B / f"{mode}-return-overview.gif", save_all=True, append_images=frames[1:], duration=600, loop=0)
        for frame in frames:
            frame.close()
    summary = read(LEVEL_B / "summary.json")
    summary.update(production_matches_worker_seed0_metrics_exactly=True,
                   scope="Observable direct-ratio domain only; populations fixed from full reference; no imputation, gain fit, alignment or reference generation")
    (LEVEL_B / "summary-final.json").write_text(json.dumps(summary, indent=2) + "\n")
    identities = {p.name: digest(p) for p in LEVEL_B.iterdir() if p.is_file() and p.name != "identities.json"}
    (LEVEL_B / "identities.json").write_text(json.dumps(identities, indent=2) + "\n")
    files = [ROOT.parent / "Config/DefaultEngine.ini", ROOT.parent / "M5Calibration.uproject"]
    for pattern in ("*.exe", "*.pak", "*.utoc", "*.ucas"):
        files.extend((ROOT / "package").rglob(pattern))
    files.extend((ROOT.parent / "Content/M6Quarry/Imported/quarry/StaticMeshes").glob("*.uasset"))
    files.extend((ROOT.parent / "Content/M6Quarry/Materials").glob("*.uasset"))
    record = dict(files={str(p): dict(bytes=p.stat().st_size, sha256=digest(p)) for p in files},
                  reference={str(p): digest(p) for p in REFERENCE.glob("pose-*-reference.npz")},
                  score_identities_sha256=digest(LEVEL_B / "identities.json"),
                  gpu_release=read(ROOT.parent / "gpu-released.json"))
    (ROOT / "full-detail-identities.json").write_text(json.dumps(record, indent=2) + "\n")
    print("Production metrics match; inspection derivatives and private identities written.")


if __name__ == "__main__":
    finalize()
