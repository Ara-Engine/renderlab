"""CPU-only Level B: bind existing UE pairs to the adaptive worker's frozen R.

Never launches Unreal/Renderlab, generates reference seeds, changes the gate, fits
an image gain, or aligns/resamples images. Missing UE receivers remain undefined.
"""
import argparse
import csv
import hashlib
import importlib.util
import json
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path("D:/Coding/renderlab-work/ue-m5/Quarry")
REFERENCE = Path("D:/Coding/renderlab-work/m6-adaptive/gpu-phase-quarry/quarry")
SUPPORT = Path("C:/Users/snowy/.t3/worktrees/renderlab/t3-fe465593/support")


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n", encoding="utf-8")


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def check_projection(world, cover, pose):
    ys, xs = np.nonzero(cover)
    stride = max(1, len(ys) // 4096)
    ys, xs = ys[::stride], xs[::stride]
    eye = np.array(pose["eye"])
    forward = np.array(pose["target"]) - eye
    forward /= np.linalg.norm(forward)
    right = np.cross(forward, pose["up"])
    right /= np.linalg.norm(right)
    up = np.cross(right, forward)
    relative = world[ys, xs, :3] - eye
    z = relative @ forward
    ty = np.tan(pose["fov_y_radians"] / 2)
    sx = ((relative @ right) / (z * ty * 1920 / 1080) + 1) * 960 - .5
    sy = (1 - (relative @ up) / (z * ty)) * 540 - .5
    error = np.hypot(sx - xs, sy - ys)
    result = dict(samples=len(xs), maximum_pixel_error=float(error.max()), rms_pixel_error=float(np.sqrt(np.mean(error ** 2))))
    if result["maximum_pixel_error"] > .01:
        raise ValueError("registered projection mismatch: " + str(result))
    return result


def visibility_image(values, cover, valid):
    rgb = np.repeat(np.rint(255 * np.clip(np.nan_to_num(values), 0, 1)).astype("uint8")[..., None], 3, -1)
    rgb[~cover] = 0
    rgb[cover & ~valid] = [255, 0, 255]
    return rgb


def error_image(values, cover, valid):
    value = np.clip(np.nan_to_num(values) * 8, -1, 1)
    rgb = np.stack([128 + 127 * np.maximum(value, 0), 128 - 127 * np.abs(value), 128 + 127 * np.maximum(-value, 0)], -1).astype("uint8")
    rgb[~cover] = 0
    rgb[cover & ~valid] = [255, 0, 255]
    return rgb


def temporal(reference, output, gate):
    source = []
    for pose in range(3):
        with np.load(reference / f"pose-{pose}-reference.npz", allow_pickle=False) as data:
            ref, depth, cover = [data[k].copy() for k in ("ref", "depth", "cover")]
        production = np.fromfile(reference / f"pose-{pose}-seed-0-production.u32", dtype="<u4").reshape(ref.shape) / 256
        source.append(dict(error=production - ref, partial=gate.populations(ref, depth, cover)["partial"]))
    results = []
    for mode in ("vsm", "hrt"):
        for family in ("steady", "first"):
            states = ["steady"] * 3 if family == "steady" else ["cut", "motion-first", "motion-first"]
            frames = []
            for pose, state in enumerate(states):
                with np.load(output / f"{mode}-pose-{pose}-{state}-motion.npz", allow_pickle=False) as data:
                    frames.append({k: data[k].copy() for k in ("visibility", "valid", "error", "partial")})
            pairs = []
            for a, b in ((0, 1), (1, 2)):
                pa, pb = frames[a]["partial"] & frames[a]["valid"], frames[b]["partial"] & frames[b]["valid"]
                full = gate.motion(source[a]["error"], source[b]["error"], source[a]["partial"], source[b]["partial"])
                observed = gate.motion(frames[a]["error"], frames[b]["error"], pa, pb)
                paired = gate.motion(source[a]["error"], source[b]["error"], pa, pb)
                pairs.append(dict(poses=[a, b], candidate_observable=observed, production_same_observable=paired,
                                  production_full=full, missing_reference_pairs=full["n"] - observed["n"]))
            common = frames[0]["valid"] & frames[2]["valid"]
            delta = frames[2]["visibility"][common] - frames[0]["visibility"][common]
            results.append(dict(mode=mode, family=family, pairs=pairs,
                return_same_valid=bool(np.array_equal(frames[0]["valid"], frames[2]["valid"])),
                return_observable_bit_identical=bool(np.array_equal(frames[0]["visibility"][common], frames[2]["visibility"][common])),
                return_difference=load("ue-quarry-score").summary(delta)))
    write(output / "G5.json", results)
    return results


def g6_atlases(reference, output):
    """Pack every enumerated 33px crop into indexed pages, avoiding tiny-file churn."""
    cases = json.loads((output / "summary.json").read_text())["cases"]
    source = {}
    for pose in range(3):
        with np.load(reference / f"pose-{pose}-reference.npz", allow_pickle=False) as data:
            ref, cover = data["ref"].copy(), data["cover"].copy()
        production = np.fromfile(reference / f"pose-{pose}-seed-0-production.u32", dtype="<u4").reshape(ref.shape) / 256
        images = [Image.fromarray(v) for v in (visibility_image(ref, cover, cover), error_image(np.zeros_like(ref), cover, cover),
                  visibility_image(production, cover, cover), error_image(production - ref, cover, cover))]
        source[pose] = ref, cover, images
    pages, total = [], 0
    for case in cases:
        label, pose = case["label"], case["pose"]
        result = json.loads((output / f"{label}.json").read_text())
        ref, cover, images = source[pose]
        with np.load(result["raw_input"], allow_pickle=False) as raw:
            if not np.array_equal(raw["source_cover"], cover):
                raise ValueError("crop source coverage differs")
            candidate, valid = raw["visibility"].copy(), raw["valid"].copy()
        candidate_images = [Image.fromarray(visibility_image(candidate, cover, valid)), Image.fromarray(error_image(candidate - ref, cover, valid))]
        panels = images + candidate_images
        with (output / f"{label}-G6.csv").open(newline="") as stream:
            entries = list(csv.DictReader(stream))
        with (output / f"{label}-G6-atlas-index.csv").open("w", newline="") as stream:
            writer = csv.writer(stream)
            writer.writerow(["x", "y", "reference", "sigma", "candidate", "error", "atlas", "atlas_xyxy"])
            for start in range(0, len(entries), 256):
                page = Image.new("RGB", (16 * 66, 16 * 99), (32, 32, 32))
                name = f"{label}-G6-atlas-{start // 256:03d}.png"
                for index, entry in enumerate(entries[start:start + 256]):
                    x, y = int(entry["x"]), int(entry["y"])
                    box = (x - 16, y - 16, x + 17, y + 17)
                    ox, oy = (index % 16) * 66, (index // 16) * 99
                    for panel, image in enumerate(panels):
                        page.paste(image.crop(box), (ox + (panel % 2) * 33, oy + (panel // 2) * 33))
                    writer.writerow([entry[k] for k in ("x", "y", "reference", "sigma", "candidate", "error")] +
                                    [name, [ox, oy, ox + 66, oy + 99]])
                page.save(output / name)
                page.close()
                pages.append(name)
        for image in candidate_images:
            image.close()
        total += len(entries)
    for _, _, images in source.values():
        for image in images:
            image.close()
    write(output / "G6-atlases.json", dict(receiver_crops=total, pages=pages,
          layout="each 66x99 tile: rows reference/production/Unreal; columns visibility/signed error x8; 33x33 source pixels per cell"))


def main(reference, support, output, vsm_run="vsm-capture-03", hrt_run="hrt-capture-01"):
    score = load("ue-quarry-score")
    gate = score.load_gate(support / "fresh_gate.py")
    manifest = json.loads((ROOT / "manifest.json").read_text())
    identities = json.loads((reference.parent / "input-identities.json").read_text())
    for key in ("asset", "cameras", "lighting"):
        if identities[key] != manifest["inputs"][key]["sha256"]:
            raise ValueError("source identity mismatch: " + key)
    camera_file = Path(json.loads((reference.parent / "quarry-manifest.json").read_text())["cameras"])
    if digest(camera_file) != identities["cameras"]:
        raise ValueError("reference camera file changed")
    cameras = json.loads(camera_file.read_text())["frames"]
    if cameras[:2] != manifest["poses"][:2]:
        raise ValueError("reference camera poses differ from UE registered poses")
    gate_record = json.loads((reference / "gate.json").read_text())
    output.mkdir(parents=True, exist_ok=False)
    audit = dict(inputs=identities, source_gate_sha256=digest(support / "fresh_gate.py"),
                 metrics_sha256=digest(support / "fresh_gate_metrics.py"), gate_json_sha256=digest(reference / "gate.json"),
                 reference={}, projection=[], source_cover_xor=[], source_depth_differences=[])
    results = []
    for pose in range(3):
        path = reference / f"pose-{pose}-reference.npz"
        audit["reference"][str(pose)] = dict(path=str(path), sha256=digest(path),
            production_seed0_sha256=digest(reference / f"pose-{pose}-seed-0-production.u32"))
        with np.load(path, allow_pickle=False) as data:
            ref, sigma, depth, cover = [data[k].copy() for k in ("ref", "sigma", "depth", "cover")]
        if ref.shape != (1080, 1920) or cover.dtype != np.bool_:
            raise ValueError("reference resolution/coverage type differs")
        world = np.fromfile(reference / f"pose-{pose}-world.f32", dtype="<f4").reshape(1080, 1920, 4)
        audit["projection"].append(check_projection(world, cover, manifest["poses"][pose]))
        production = np.fromfile(reference / f"pose-{pose}-seed-0-production.u32", dtype="<u4").reshape(ref.shape) / 256
        q = gate_record["poses"][pose]["q"]
        prod_metrics, prod_error, populations = gate.metrics(production, ref, sigma, depth, cover, q)
        if prod_metrics != gate_record["poses"][pose]["seeds"][0]["production_metrics"]:
            raise ValueError("production metrics differ from the adaptive worker's own seed-0 record")
        write(output / f"pose-{pose}-production.json", prod_metrics)
        for name, rgb in (("reference-visibility", visibility_image(ref, cover, cover)),
                          ("production-visibility", visibility_image(production, cover, cover)),
                          ("production-error-x8", error_image(prod_error, cover, cover))):
            Image.fromarray(rgb).save(output / f"pose-{pose}-{name}.png")
        for mode, run in (("vsm-smrt", vsm_run), ("hardware-rt-denoised", hrt_run)):
            capture = ROOT / "runs" / run / "capture.json"
            files = sorted((ROOT / "extracted" / run).glob(f"pose-{pose}-*-direct.npz"))
            if not files:
                raise ValueError("no extracted visibility for " + run)
            for raw_path in files:
                state = raw_path.name.removeprefix(f"pose-{pose}-").removesuffix("-direct.npz")
                with np.load(raw_path, allow_pickle=False) as raw:
                    shadowed, unshadowed = raw["shadowed"].copy(), raw["unshadowed"].copy()
                    ue_cover = raw["ue_cover"].copy()
                    if state == "steady":
                        audit["source_cover_xor"].append(dict(mode=mode, pose=pose, pixels=int((raw["source_cover"] ^ cover).sum())))
                        audit["source_depth_differences"].append(dict(mode=mode, pose=pose, pixels=int((raw["depth"] != depth).sum())))
                shadowed[~ue_cover] = np.nan
                arrays = dict(shadowed=shadowed, unshadowed=unshadowed, reference=ref, sigma=sigma, depth=depth,
                              cover=cover, production=production, q_prod=np.array(q))
                metadata = dict(reference=dict(seeds=list(range(1, 65)), rays_per_seed=256, production_seed=0, spread_seeds=list(range(65, 81))),
                    resolution=[1920, 1080], asset_sha256=identities["asset"], reference_sha256=digest(path),
                    ue_capture_identity_sha256=digest(capture), paired_settings_sha256=digest(ROOT / "manifest.json"),
                    environment="black-both", rgb_scale="unexposed-linear", pose=pose, state="steady" if state == "steady" else "cut" if pose == 0 else "motion", mode=mode)
                result, candidate, valid, error, pops = score.score(arrays, metadata, gate, 1e-6)
                label = f"{'vsm' if mode == 'vsm-smrt' else 'hrt'}-pose-{pose}-{state}"
                entries = result["G6"].pop("entries")
                with (output / f"{label}-G6.csv").open("w", newline="") as stream:
                    writer = csv.writer(stream)
                    writer.writerow(["x", "y", "reference", "sigma", "candidate", "error", "crop_xyxy"])
                    for entry in entries:
                        writer.writerow([entry[k] for k in ("x", "y", "reference", "sigma", "candidate", "error", "crop_xyxy")])
                result.update(raw_input=str(raw_path), raw_input_sha256=digest(raw_path), captured_state=state,
                              production_full_domain=prod_metrics, G6_csv=f"{label}-G6.csv")
                write(output / f"{label}.json", result)
                if state in ("cut", "motion-first", "steady"):
                    cv = visibility_image(candidate, cover, valid)
                    ce = error_image(error, cover, valid)
                    Image.fromarray(cv).save(output / f"{label}-visibility.png")
                    Image.fromarray(ce).save(output / f"{label}-error-x8.png")
                    # Complete shared-scale panels also define every enumerated crop.
                    sheet = np.concatenate([np.concatenate([visibility_image(ref, cover, cover), error_image(np.zeros_like(ref), cover, cover)], 1),
                        np.concatenate([visibility_image(production, cover, cover), error_image(prod_error, cover, cover)], 1),
                        np.concatenate([cv, ce], 1)], 0)
                    Image.fromarray(sheet).save(output / f"{label}-sheet.png")
                    np.savez(output / f"{label}-motion.npz", visibility=candidate, valid=valid, error=error, partial=pops["partial"])
                    # Worst envelope/error crops are review aids; G6 CSV enumerates all.
                    partial_valid = pops["partial"] & valid
                    ys, xs = np.nonzero(partial_valid)
                    ranked = np.argsort(np.abs(error[ys, xs]))[-16:]
                    crops = []
                    for index in ranked:
                        y, x = int(ys[index]), int(xs[index])
                        sl = np.s_[max(0, y - 16):min(1080, y + 17), max(0, x - 16):min(1920, x + 17)]
                        tile = np.concatenate([np.concatenate([visibility_image(v[sl], cover[sl], m[sl]), error_image(e[sl], cover[sl], m[sl])], 1)
                            for v, e, m in ((ref, np.zeros_like(ref), cover), (production, prod_error, cover), (candidate, error, valid))], 0)
                        name = f"{label}-crop-{x}-{y}.png"
                        Image.fromarray(np.repeat(np.repeat(tile, 3, 0), 3, 1)).save(output / name)
                        crops.append(name)
                    result["inspection_crops"] = crops
                    write(output / f"{label}.json", result)
                results.append(dict(label=label, mode=mode, pose=pose, state=state, partial=result["G3"]["partial"],
                    G1=result["G1"], G2=result["G2"], G4=result["G4"], G6=result["G6"], missing=result["missing_by_reference_population"]))
                print(label, "RMS", result["G3"]["partial"]["candidate"]["rms"], "missing partial", result["G1"]["missing"], flush=True)
    write(output / "input-audit.json", audit)
    g5 = temporal(reference, output, gate)
    write(output / "summary.json", dict(cases=results, G5=g5, status="OBSERVABLE_DOMAIN_LEVEL_B; complete-domain metrics undefined where direct ratio is unobservable"))
    g6_atlases(reference, output)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", type=Path, default=REFERENCE)
    parser.add_argument("--support", type=Path, default=SUPPORT)
    parser.add_argument("--output", type=Path, default=ROOT / "level-b")
    parser.add_argument("--temporal-only", action="store_true", help="finish existing pose scores without regenerating images")
    parser.add_argument("--crops-only", action="store_true", help="pack all existing G6 receiver crops")
    parser.add_argument("--vsm-run", default="vsm-capture-03")
    parser.add_argument("--hrt-run", default="hrt-capture-01")
    args = parser.parse_args()
    if not args.output.resolve().is_relative_to(ROOT.resolve()):
        parser.error("outputs must remain private")
    if args.crops_only:
        g6_atlases(args.reference, args.output)
    elif args.temporal_only:
        gate = load("ue-quarry-score").load_gate(args.support / "fresh_gate.py")
        temporal(args.reference, args.output, gate)
    else:
        main(args.reference, args.support, args.output, args.vsm_run, args.hrt_run)
