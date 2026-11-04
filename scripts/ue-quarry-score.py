"""Score registered UE direct-light pairs with the existing reduced-ray metrics.

Input NPZ: shadowed/unshadowed [H,W,3] unexposed linear RGB, reference/sigma/depth/
cover [H,W], production [H,W], q_prod scalar. Metadata JSON must describe input
identities, reference seeds and paired capture settings. Undefined direct-light
ratios stay visible as missing coverage; they must never become lit receivers.
This measures Level B's error profile. It does not declare Level-A acceptance.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import sys

import numpy as np

GATE = Path("D:/Coding/renderlab-work/m6-reduced-ray-gate/scripts/gate.py")
GATE_SHA = "0ffacfc6621145b30e5b919e89d471539898be32e9766fbe1464f1b2c7e57322"


def load_gate(path):
    if path.name == "fresh_gate.py":
        # The adaptive runner exposes the shared definitions as fresh_gate.gate.
        # Import its facade without calling prepare(), which would rewrite R.
        sys.path.insert(0, str(path.parent))
    spec = importlib.util.spec_from_file_location("registered_shadow_gate", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    if path.name == "fresh_gate.py":
        module = module.gate
    if hashlib.sha256(Path(module.__file__).read_bytes()).hexdigest() != GATE_SHA:
        raise ValueError("gate metrics changed; re-audit rather than silently changing the definition")
    return module


def visibility(shadowed, unshadowed, cover, minimum_direct=1e-6):
    if shadowed.shape != (*cover.shape, 3) or unshadowed.shape != shadowed.shape:
        raise ValueError("expected matching H,W,3 linear RGB and H,W coverage")
    if not np.isfinite(minimum_direct) or minimum_direct <= 0:
        raise ValueError("minimum direct light must be positive")
    weights = np.array([.2126, .7152, .0722])
    a, b = shadowed @ weights, unshadowed @ weights
    finite = np.isfinite(shadowed).all(-1) & np.isfinite(unshadowed).all(-1)
    nonnegative = (shadowed >= 0).all(-1) & (unshadowed >= 0).all(-1)
    valid = cover & finite & nonnegative & (b > minimum_direct)
    result = np.full(cover.shape, np.nan)
    # No clipping: negative, >1, invalid and dark-denominator populations are evidence.
    np.divide(a, b, out=result, where=valid)
    channels = np.full_like(shadowed, np.nan, dtype="f8")
    np.divide(shadowed, unshadowed, out=channels, where=(unshadowed > minimum_direct) & valid[..., None])
    spread = np.max(np.where(np.isfinite(channels), np.abs(channels - result[..., None]), 0), axis=-1)
    return result, valid, dict(
        covered=int(cover.sum()), valid=int(valid.sum()), undefined=int((cover & ~valid).sum()),
        nonfinite=int((cover & ~finite).sum()), negative_rgb=int((cover & finite & ~nonnegative).sum()),
        dark_denominator=int((cover & finite & nonnegative & (b <= minimum_direct)).sum()),
        outside_unit_interval=int((valid & ((result < 0) | (result > 1))).sum()),
        maximum_channel_ratio_disagreement=float(spread[valid].max()) if valid.any() else None,
        minimum_direct=minimum_direct, ratio="linear Rec709 luminance(shadowed) / luminance(unshadowed); unclipped")


def validate_metadata(metadata):
    reference = metadata["reference"]
    if reference["seeds"] != list(range(1, 65)) or reference["rays_per_seed"] != 256:
        raise ValueError("quarry reference must be the registered independent seeds 1..64, 256 rays each")
    if reference["production_seed"] != 0 or reference["spread_seeds"] != list(range(65, 81)):
        raise ValueError("production/seed-spread identity differs from the gate")
    if metadata["resolution"] != [1920, 1080]:
        raise ValueError("registered quarry resolution differs")
    for key in ("asset_sha256", "reference_sha256", "ue_capture_identity_sha256", "paired_settings_sha256"):
        value = metadata[key]
        if len(value) != 64 or any(c not in "0123456789abcdef" for c in value):
            raise ValueError("missing/invalid input identity: " + key)
    if metadata["environment"] != "black-both" or metadata["rgb_scale"] != "unexposed-linear":
        raise ValueError("paired captures must contain only unexposed direct light")
    if metadata["pose"] not in (0, 1, 2) or metadata["state"] not in ("cut", "steady", "motion", "return"):
        raise ValueError("unknown registered pose/state")
    if metadata["mode"] not in ("vsm-smrt", "hardware-rt-denoised"):
        raise ValueError("unknown UE shadow candidate")


def score(arrays, metadata, gate, minimum_direct):
    validate_metadata(metadata)
    cover = arrays["cover"]
    if cover.dtype != np.bool_ or cover.shape != (1080, 1920):
        raise ValueError("expected bool coverage at the registered resolution")
    ref, sigma, depth, prod = [arrays[k] for k in ("reference", "sigma", "depth", "production")]
    if any(a.shape != cover.shape for a in (ref, sigma, depth, prod)):
        raise ValueError("reference shapes differ")
    if any(not np.isfinite(a[cover]).all() for a in (ref, sigma, depth, prod)):
        raise ValueError("nonfinite covered reference values")
    if np.any((ref[cover] < 0) | (ref[cover] > 1) | (sigma[cover] < 0)):
        raise ValueError("invalid reference range")
    q = float(arrays["q_prod"])
    if not np.isfinite(q) or q < 0:
        raise ValueError("invalid production p99 floor")
    candidate, valid, extraction = visibility(arrays["shadowed"], arrays["unshadowed"], cover, minimum_direct)
    # Populations/contours are formed from the full reference. Missing candidate
    # pixels do not redefine narrow/faint/isolated/silhouette populations.
    populations = gate.populations(ref, depth, cover)
    missing = {name: int((mask & ~valid).sum()) for name, mask in populations.items()}
    result = dict(metadata=metadata, extraction=extraction, missing_by_reference_population=missing,
                  complete_visibility_coverage=not any(missing.values()),
                  status="UNMEASURED" if not valid.any() else "PARTIAL_PROFILE" if any(missing.values()) else "MEASURED_PROFILE",
                  G5="requires sequence/repeat/return comparison", G7="requires full images, crops and motion inspection",
                  verdict="Level B measured profile only; no parity or performance acceptance inferred")
    error = candidate - ref
    stats = {}
    for name, mask in populations.items():
        values = error[mask & valid]
        baseline = (prod - ref)[mask & valid]
        stats[name] = dict(reference_n=int(mask.sum()), evaluated_n=int(values.size),
                           candidate=summary(values), production=summary(baseline))
    partial = populations["partial"]
    envelope = np.maximum(np.maximum(5 * sigma, q), 1 / 256)
    over = valid & partial & (np.abs(error) > envelope)
    solid = valid & (populations["lit"] | populations["occluded"])
    # Missing contour samples invalidate a complete G4 comparison. Never fill
    # them with a zero error and let that appear to improve the statistic.
    contour = gate.along_contour(error, ref, partial, depth)
    finite_contour = np.isfinite(contour)
    extremes = valid & partial & ((candidate >= 1) | (candidate <= 0))
    ys, xs = np.nonzero(extremes)
    enumeration = [dict(x=int(x), y=int(y), candidate=float(candidate[y, x]), reference=float(ref[y, x]),
                        sigma=float(sigma[y, x]), error=float(error[y, x]),
                        crop_xyxy=[max(0, int(x) - 16), max(0, int(y) - 16), min(1920, int(x) + 17), min(1080, int(y) + 17)])
                   for y, x in zip(ys, xs)]
    result.update(G1=dict(evaluated=int((valid & partial).sum()), violations=int(over.sum()),
                          missing=int((partial & ~valid).sum()), q_prod=q),
                  G2=dict(evaluated=int(solid.sum()), maximum=float(np.abs(error[solid]).max()) if solid.any() else None,
                          over_002=int((solid & (np.abs(error) > .002)).sum())), G3=stats,
                  G4=dict(evaluated=int(finite_contour.sum()), missing=int((~finite_contour).sum()),
                          statistics=summary(contour[finite_contour])),
                  G6=dict(count=len(enumeration), entries=enumeration))
    return result, candidate, valid, error, populations


def summary(values):
    if values.size == 0:
        return None
    return dict(n=int(values.size), rms=float(np.sqrt(np.mean(values ** 2))), mean=float(values.mean()),
                p99=float(np.quantile(np.abs(values), .99)), maximum=float(np.abs(values).max()))


def sequence(paths, gate, output):
    """Compare residual changes and raw repeat/return visibility without alignment."""
    rows, previous, first = [], None, None
    for path in paths:
        with np.load(path, allow_pickle=False) as data:
            current = {k: data[k].copy() for k in ("visibility", "valid", "error", "partial")}
        if previous is not None:
            common = previous["partial"] & current["partial"] & previous["valid"] & current["valid"]
            delta = (current["error"] - previous["error"])[common]
            rows.append(dict(a=str(previous_path), b=str(path), motion_residual=summary(delta)))
        if first is None:
            first = current
        last = current
        previous, previous_path = current, path
    if first is None:
        raise ValueError("sequence is empty")
    same_valid = np.array_equal(first["valid"], last["valid"])
    common = first["valid"] & last["valid"]
    write_json(output, dict(pairs=rows, first_last_same_valid=same_valid,
                            first_last_equal_values=bool(same_valid and np.array_equal(first["visibility"][common], last["visibility"][common])),
                            first_last_difference=summary((last["visibility"] - first["visibility"])[common]),
                            scope="first/last must be the same registered pose to interpret as repeat/return; no reprojection"))


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n", encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gate", type=Path, default=GATE)
    parser.add_argument("--input", type=Path)
    parser.add_argument("--metadata", type=Path)
    parser.add_argument("--output", type=Path, required=True, help="new private output directory")
    parser.add_argument("--minimum-direct", type=float, default=1e-6)
    parser.add_argument("--sequence", nargs="+", type=Path)
    args = parser.parse_args()
    output = args.output.resolve()
    private = Path("D:/Coding/renderlab-work/ue-m5").resolve()
    if not output.is_relative_to(private):
        parser.error("outputs must stay inside the existing private UE root")
    gate = load_gate(args.gate)
    output.mkdir(parents=True, exist_ok=False)
    if args.sequence:
        sequence(args.sequence, gate, output / "sequence.json")
    else:
        if args.input is None or args.metadata is None:
            parser.error("--input and --metadata are required for a pose")
        metadata = json.loads(args.metadata.read_text(encoding="utf-8-sig"))
        with np.load(args.input, allow_pickle=False) as arrays:
            result, candidate, valid, error, populations = score(arrays, metadata, gate, args.minimum_direct)
        result["input_npz_sha256"] = hashlib.sha256(args.input.read_bytes()).hexdigest()
        result["metadata_sha256"] = hashlib.sha256(args.metadata.read_bytes()).hexdigest()
        result["gate_sha256"] = GATE_SHA
        write_json(output / "scores.json", result)
        np.savez(output / "visibility.npz", visibility=candidate, valid=valid, error=error, partial=populations["partial"])
        print(json.dumps({k: result[k] for k in ("status", "complete_visibility_coverage", "extraction", "missing_by_reference_population")}, indent=2))
