"""Paired P-LTS2 ARA1 game-light replay, inside p-lts2-run.py's lease.

Uses frozen ARA1 scenes without changing any quality setting. Captures byte hashes
of displayed PNG pixels and the renderer's resolved linear HDR bytes.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import time

REPO = Path(__file__).resolve().parents[1]
ROOT = Path("D:/Coding/renderlab-work/p-lts2")
SCENES = Path("D:/Coding/renderlab-work/ara1/phase2-scenes")
SCHEDULE = Path("E:/renderlab-work/gpu-schedule.md")


def order():
    return [line for line in SCHEDULE.read_text(encoding="utf-8").splitlines()
            if line.startswith("[root order]")][-1]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("label")
    p.add_argument("--binary", type=Path, required=True)
    p.add_argument("--baseline-binary", type=Path)
    p.add_argument("--candidate-standard", type=Path)
    p.add_argument("--scene-dir", type=Path, default=SCENES)
    p.add_argument("--resolution", choices=["quality", "timing"], default="timing")
    p.add_argument("--cases", default="game-lights,game-lights-unshadowed")
    p.add_argument("--paths", default="rt,nonrt")
    p.add_argument("--steady-warmup", type=int, default=0, help="append warm still frames and eight measured samples; retain the original replay")
    a = p.parse_args()
    out = ROOT / "runs" / a.label
    out.mkdir(parents=True, exist_ok=False)
    owner = json.loads(Path("D:/Coding/renderlab-work/gpu-slot.lock/owner.json").read_text())
    if owner["owner"] != "p-lts2":
        raise RuntimeError("P-LTS2 lease required")
    directive = order()
    summary = dict(lease=owner, root_order=directive, pairs=[], comparisons=[])
    for case in a.cases.split(","):
        scene = a.scene_dir / f"{case}-{a.resolution}.json"
        original_scene_sha256 = digest(scene)
        spec = json.loads(scene.read_text())
        asset = Path(spec["asset"])
        if not asset.is_absolute():
            asset = scene.parent / asset
        asset_sha256 = digest(asset)
        if a.steady_warmup:
            if a.steady_warmup < 0:
                raise ValueError("warmup must be nonnegative")
            spec["asset"] = str(asset.resolve())
            pose = spec["frames"][39]
            spec["frames"] += [pose | dict(cut=k == 0, capture=k == a.steady_warmup + 7,
                phase="steady_warmup" if k < a.steady_warmup else "steady")
                for k in range(a.steady_warmup + 8)]
            scene = out / f"{case}-steady.json"
            scene.write_text(json.dumps(spec, indent=2))
        expected_hdr = {k for k, frame in enumerate(spec["frames"]) if frame["capture"] or k == 0}
        expected_png = {f"frame-{k:04}.png" for k, frame in enumerate(spec["frames"]) if frame["capture"]}
        for mode in a.paths.split(","):
            if mode not in ("rt", "nonrt"):
                raise ValueError("paths must be rt and/or nonrt")
            paired = []
            for variant in ("base", "candidate"):
                latest = order()
                if latest != directive:
                    if "Running leases are not stopped by this order." not in latest:
                        raise RuntimeError("root order changed between steps")
                    directive = latest
                folder = out / f"{case}-{mode}-{variant}"
                folder.mkdir()
                binary = a.baseline_binary if variant == "base" and a.baseline_binary else a.binary
                env = os.environ | {"ARA1_HDR_HASH": "1", "VK_LOADER_LAYERS_DISABLE": "VK_LAYER_KHRONOS_validation"}
                env.pop("VK_INSTANCE_LAYERS", None)
                if variant == "base" and a.baseline_binary is None:
                    env["ARA1_STANDARD_PROGRAM"] = str(ROOT / "base-standard.wgsl")
                elif variant == "candidate" and a.candidate_standard:
                    env["ARA1_STANDARD_PROGRAM"] = str(a.candidate_standard)
                else:
                    env.pop("ARA1_STANDARD_PROGRAM", None)
                command = [str(binary), str(scene), mode, str(folder)]
                start = time.monotonic()
                start_unix = time.time()
                with (folder / "process.log").open("w") as log:
                    result = subprocess.run(command, cwd=REPO, env=env, stdout=log, stderr=subprocess.STDOUT,
                                            creationflags=subprocess.CREATE_NO_WINDOW, timeout=180)
                if result.returncode:
                    raise RuntimeError(f"{command}: {result.returncode}: " + (folder / "process.log").read_text()[-3000:])
                report = json.loads((folder / "engine.json").read_text())
                frames = report["frames"]
                row = dict(case=case, mode=mode, variant=variant, binary=str(binary),
                           start_unix=start_unix, end_unix=time.time(),
                           standard_override=env.get("ARA1_STANDARD_PROGRAM"),
                           binary_sha256=digest(binary), scene_sha256=digest(scene), original_scene_sha256=original_scene_sha256,
                           asset_sha256=asset_sha256, steady_warmup=a.steady_warmup,
                           resolution=spec["size"], seconds=time.monotonic()-start,
                           service_ms={phase: statistics.median(frames[k]["cpu_service_ms"] for k in indices)
                                       for phase, indices in (("still", range(32, 40)), ("move", range(64, 72)),
                                                               ("cut_first", [72]), ("cut_settled", range(80, 88)))},
                           hdr={f["index"]: f["hdr_sha256"] for f in frames if f.get("hdr_sha256")},
                           png={f.name: digest(f) for f in sorted(folder.glob("frame-*.png"))})
                if a.steady_warmup:
                    row["service_ms"]["steady_still"] = statistics.median(f["cpu_service_ms"] for f in frames[-8:])
                if set(row["hdr"]) != expected_hdr or set(row["png"]) != expected_png:
                    raise RuntimeError("missing capture witnesses; empty/missing hashes cannot establish parity")
                if digest(asset) != asset_sha256:
                    raise RuntimeError("asset changed during the paired replay")
                summary["pairs"].append(row)
                paired.append(row)
                print(case, mode, variant, row["service_ms"], flush=True)
                (out / "summary.json").write_text(json.dumps(summary, indent=2))
            comparison = dict(case=case, mode=mode, hdr_identical=paired[0]["hdr"] == paired[1]["hdr"],
                              png_identical=paired[0]["png"] == paired[1]["png"])
            summary["comparisons"].append(comparison)
            print(comparison, flush=True)
            (out / "summary.json").write_text(json.dumps(summary, indent=2))
            if not comparison["hdr_identical"] or not comparison["png_identical"]:
                raise RuntimeError("pure optimization changed a captured image")


if __name__ == "__main__":
    main()
