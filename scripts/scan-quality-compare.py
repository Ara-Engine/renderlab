"""Compare a bounded quarry replay against a full-detail supersampled reference."""
from __future__ import annotations

import argparse
from functools import cache
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageFilter


@cache
def file_identity(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def load(path):
    with Image.open(path) as image:
        return np.array(image.convert("RGB"))


def linear(pixels):
    x = np.arange(256, dtype=np.float32) / 255
    table = np.where(x <= 0.04045, x / 12.92, ((x + 0.055) / 1.055) ** 2.4)
    return table[pixels]


def foreground(pixels, background):
    return np.max(np.abs(pixels.astype(np.int16) - background), axis=2) > 1


def masks(reference, coverage):
    solid = Image.fromarray((coverage >= 0.5).astype(np.uint8) * 255)
    outer = np.array(solid.filter(ImageFilter.MaxFilter(5))) > 0
    inner = np.array(solid.filter(ImageFilter.MinFilter(5))) > 0
    edge = (outer != inner) | ((coverage > 0) & (coverage < 1))
    luma = reference @ np.array([0.2126, 0.7152, 0.0722], dtype=np.float32)
    gradient = np.zeros_like(luma)
    gradient[:, 1:] = np.abs(np.diff(luma, axis=1))
    gradient[1:] = np.maximum(gradient[1:], np.abs(np.diff(luma, axis=0)))
    texture = inner & ~edge & (gradient >= 0.04)
    return {"edge": edge, "texture": texture, "smooth_interior": inner & ~edge & ~texture}


def statistics(residual, regions):
    error = np.abs(residual)
    result = {}
    for name, mask in regions.items():
        values = error[mask].mean(axis=1)
        result[name] = {"pixels": int(mask.sum()), "mae": float(values.mean()) if values.size else None,
                        "p95": float(np.percentile(values, 95)) if values.size else None}
    return result


def image_path(directory, index):
    paths = [directory / f"frame-{index:04}.{ext}" for ext in ("ppm", "png")]
    matches = [p for p in paths if p.exists()]
    if len(matches) != 1:
        raise ValueError(f"expected exactly one PPM or PNG for frame {index} in {directory}")
    return matches[0]


def verify_run(directory, expected, reference, plan, indices):
    raw = (directory / "run.txt").read_text()
    args = json.loads(raw.split("\nargs=", 1)[1].strip())
    def option(key, default=None):
        return args[args.index(key) + 1] if key in args else default
    poses = json.loads(Path(option("--cameras")).read_text())["frames"]
    scale = 2 if reference else 1
    for i in indices:
        registered = expected[i]
        pose = dict(registered, width=registered["width"] * scale, height=registered["height"] * scale)
        if poses[i] != pose:
            raise ValueError(f"run camera {i} differs from the declared comparison")
    start, count = int(option("--start", "0")), int(option("--frames", "0"))
    available = {start} if "--hold-pose" in args and count > 0 else set(range(start, start + count))
    if not set(indices) <= available:
        raise ValueError("run does not cover the requested registered frames")
    if reference and (option("--mode") != "full" or option("--samples") != "4"):
        raise ValueError("reference requires full geometry and 4x MSAA")
    mode = option("--mode", "stream")
    if mode == "full":
        asset = Path(option("--asset")).resolve()
        allowed = {str(Path(plan["canonical_asset"]).resolve()), str((Path(plan["output"]) / "full.rla").resolve())}
        if str(asset) not in allowed:
            raise ValueError("full run asset is outside the supplied variant plan")
        expected_hash = plan["canonical_payload_sha256"] if asset == Path(plan["canonical_asset"]).resolve() else plan["proof"]["full_sha256"]
        identity = file_identity(asset)
        if identity != expected_hash:
            raise ValueError("full run payload differs from its recorded digest")
        full_content = identity
    elif mode == "stream":
        manifest = Path(option("--manifest")).read_bytes()
        if len(manifest) != 80 or manifest[:8] != b"RVM\0\1\0\0\0":
            raise ValueError("invalid stream manifest")
        package_hash = manifest[40:72].hex()
        canonical = plan["canonical_package_sha256"]
        if package_hash not in {canonical, plan.get("proof", {}).get("package_sha256")}:
            raise ValueError("streamed run package differs from the supplied variant plan")
        package = Path(option("--cache")) / "virtual-v1/packages" / package_hash
        if file_identity(package.resolve()) != package_hash:
            raise ValueError("streamed run package bytes differ from the manifest")
        identity = package_hash
        full_content = plan["canonical_payload_sha256"] if package_hash == canonical else plan["proof"]["full_sha256"]
    else:
        raise ValueError("quality comparison requires full or stream rendering")
    return {"mode": mode, "samples": option("--samples", "1"), "content": identity, "full_content": full_content,
            "run_sha256": hashlib.sha256(raw.encode()).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("plan", type=Path)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("reference", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--goal", choices=("edges", "texture"), default="edges")
    parser.add_argument("--baseline-plan", type=Path, help="plan for a separately prepared 2K control")
    parser.add_argument("--reference-plan", type=Path, help="plan for a selective 8K reference")
    parser.add_argument("--frames", help="registered indices, such as 1,2 for a two-view spatial pilot")
    args = parser.parse_args()
    out = args.output.resolve()
    if out.exists() or out.drive.lower() != "d:":
        raise ValueError("choose a fresh D-drive comparison directory")
    plan = json.loads(args.plan.read_text())
    cameras = json.loads((args.plan.parent / "cameras.json").read_text())["frames"]
    indices = sorted(set(map(int, args.frames.split(',')))) if args.frames else list(range(len(cameras)))
    if not indices or indices[0] < 0 or indices[-1] >= len(cameras):
        raise ValueError("comparison frame outside registered cameras")
    thresholds = plan["acceptance"]
    plans = {"baseline": json.loads(args.baseline_plan.read_text()) if args.baseline_plan else plan,
             "candidate": plan,
             "reference": json.loads(args.reference_plan.read_text()) if args.reference_plan else plan}
    if len({p["canonical_payload_sha256"] for p in plans.values()}) != 1:
        raise ValueError("comparison plans derive from different canonical geometry/content")
    proofs = [p["proof"]["unchanged_rla_prefix_sha256"] for p in plans.values() if "proof" in p]
    if len(set(proofs)) > 1:
        raise ValueError("variant geometry, placements, or material bindings differ")
    runs = {name: verify_run(directory, cameras, name == "reference", plans[name], indices) for name, directory in
            [("baseline", args.baseline), ("candidate", args.candidate), ("reference", args.reference)]}
    if args.goal == "edges" and len({r["full_content"] for r in runs.values()}) > 1:
        raise ValueError("edge/AA acceptance requires identical texture content")
    background = np.array(thresholds["background_rgb8"], dtype=np.int16)
    out.mkdir(parents=True)
    # Save the contract before reading candidate pixels. Failed comparisons keep this record.
    (out / "contract.json").write_text(json.dumps({"plan_sha256": hashlib.sha256(args.plan.read_bytes()).hexdigest(),
        "goal": args.goal, "frames": indices, "acceptance": thresholds}, indent=2) + "\n")
    frames, temporal, previous, motion = [], [], None, []
    pairs = {tuple(pair) for pair in plan["motion_pairs"]}
    for index in indices:
        pose = cameras[index]
        paths = {name: image_path(directory, index) for name, directory in
                 [("baseline", args.baseline), ("candidate", args.candidate), ("reference", args.reference)]}
        raw_ref = load(paths["reference"])
        h, w = pose["height"], pose["width"]
        if raw_ref.shape != (h * 2, w * 2, 3):
            raise ValueError("reference must be exactly 2x per axis, with unchanged camera projection")
        reference = linear(raw_ref).reshape(h, 2, w, 2, 3).mean(axis=(1, 3))
        coverage = foreground(raw_ref, background).reshape(h, 2, w, 2).mean(axis=(1, 3))
        regions = masks(reference, coverage)
        row = {"frame": index, "phase": pose["phase"], "images": {}, "results": {}}
        residuals = {}
        for name in ("baseline", "candidate"):
            raw = load(paths[name])
            if raw.shape != (h, w, 3):
                raise ValueError("native image extent differs from registered camera")
            residuals[name] = linear(raw) - reference
            values = statistics(residuals[name], regions)
            disagreement = (foreground(raw, background) != (coverage >= 0.5)) & ~regions["edge"]
            values["coverage_mismatch_outside_edge_fraction"] = float(disagreement.mean())
            row["results"][name] = values
            if name == "candidate" and 2 <= index <= 10:
                motion.append(Image.fromarray(raw))
        for name, path in paths.items():
            row["images"][name] = {"path": str(path.resolve()), "sha256": file_identity(path.resolve())}
        if index in (0, 1, 2, 11):
            heat = np.abs(residuals["candidate"]).max(axis=2)
            Image.fromarray(np.uint8(np.clip(heat * 4, 0, 1) * 255)).save(out / f"error-{index:04}.png")
            Image.fromarray(regions["edge"].astype(np.uint8) * 255).save(out / f"edge-{index:04}.png")
            x = np.clip(reference, 0, 1)
            srgb = np.where(x <= 0.0031308, x * 12.92, 1.055 * x ** (1 / 2.4) - 0.055)
            Image.fromarray(np.uint8(np.floor(srgb * 255 + 0.5))).save(out / f"reference-{index:04}.png")
        if previous is not None and (previous[0], index) in pairs:
            union = {key: regions[key] | previous[2][key] for key in regions}
            temporal.append({"frames": [previous[0], index], "results": {
                name: statistics(residuals[name] - previous[1][name], union) for name in residuals}})
        previous = index, residuals, regions
        frames.append(row)
    issues = []
    for row in frames:
        b, c = (row["results"][name] for name in ("baseline", "candidate"))
        for region, metric, limit in [("edge", "mae", thresholds["edge_mae_max"]),
                                      ("edge", "p95", thresholds["edge_p95_max"]),
                                      ("smooth_interior", "mae", thresholds["smooth_mae_max"])]:
            value = c[region][metric]
            if value is None or value > limit:
                issues.append(f"frame {row['frame']} {region} {metric} exceeds gate or has no pixels")
        if c["coverage_mismatch_outside_edge_fraction"] > thresholds["coverage_mismatch_fraction_max"]:
            issues.append(f"frame {row['frame']} coverage differs outside the reference edge band")
        for region in ("edge", "texture", "smooth_interior"):
            if b[region]["mae"] is not None and c[region]["mae"] > b[region]["mae"] * 1.05 + 0.001:
                issues.append(f"frame {row['frame']} {region} error regresses")
    for row in temporal:
        for region in ("edge", "texture"):
            b, c = (row["results"][name][region]["mae"] for name in ("baseline", "candidate"))
            limit = thresholds[f"temporal_{region}_mae_max"]
            if c is None or c > limit or (b is not None and c > b * 1.05 + 0.001):
                issues.append(f"motion {row['frames']} {region} residual change exceeds gate")
    region = "edge" if args.goal == "edges" else "texture"
    selected = frames if args.goal == "edges" else [r for r in frames if r["frame"] in (1, 2)]
    means = {}
    for name in ("baseline", "candidate"):
        values = [r["results"][name][region]["mae"] for r in selected if r["results"][name][region]["mae"] is not None]
        means[name] = float(np.mean(values)) if values else None
    reduction = thresholds[f"{args.goal}_improvement_min"]
    if any(v is None for v in means.values()) or means["candidate"] > max(means["baseline"] * (1 - reduction), 0.001):
        issues.append(f"{args.goal} mean error reduction is below the declared gate")
    report = {"numeric_status": "ISSUES" if issues else "PASS", "visual_status": "UNRUN", "runs": runs,
              "temporal_status": "SAMPLED" if temporal else "UNRUN", "compared_frames": indices,
              "validation_status": "inspect process records separately", "issues": issues,
              "improvement_region_means": means,
              "temporal_method": "difference of reference-relative residuals at fixed screen pixels; no motion compensation",
              "frames": frames, "temporal": temporal}
    if len(motion) > 1:
        motion[0].save(out / "candidate-motion.webp", save_all=True, append_images=motion[1:],
                       lossless=True, method=0, duration=100, loop=0)
    (out / "results.json").write_text(json.dumps(report, indent=2, allow_nan=False) + "\n")
    print(json.dumps({k: v for k, v in report.items() if k not in ("frames", "temporal")}, indent=2))
    raise SystemExit(bool(issues))


if __name__ == "__main__":
    main()
