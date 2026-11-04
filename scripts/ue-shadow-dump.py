"""Decode local UE 5.7 RDG dump metadata and versioned main-light inputs."""
import argparse
import csv
import hashlib
import importlib.util
import json
from pathlib import Path
import re

import numpy as np
from PIL import Image

ROOT = Path("D:/Coding/renderlab-work/ue-m5/Quarry")
REFERENCE = Path("D:/Coding/renderlab-work/m6-adaptive/gpu-phase-quarry/quarry")
SUPPORT = Path("C:/Users/snowy/.t3/worktrees/renderlab/t3-fe465593/support")


def objects(path):
    """DumpGPU writes adjacent JSON objects, rather than a JSON array."""
    source = path.read_text(encoding="utf-8-sig")
    decoder = json.JSONDecoder()
    index = 0
    result = []
    while index < len(source):
        while index < len(source) and source[index].isspace():
            index += 1
        if index == len(source):
            break
        value, index = decoder.raw_decode(source, index)
        result.append(value)
    return result


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write(path, data):
    path.write_text(json.dumps(data, indent=2, allow_nan=False) + "\n")


class Dump:
    def __init__(self, root):
        self.root = root if (root / "Base").is_dir() else next(root.glob("M5Calibration-*"))
        self.passes = objects(self.root / "Base/Passes.json")
        self.resources = {v["UniqueResourceName"]: v for v in objects(self.root / "Base/ResourceDescs.json")}
        self.metadata = {p.stem: json.loads(p.read_text()) for p in (self.root / "StructuresMetadata").glob("*.json")}

    def inputs(self, event):
        return {self.resources[name.split(".")[0]]["Name"]: name for name in event["InputResources"]}

    def resource(self, name, event):
        """Resolve the last writer before this consumer, never lexicographic order."""
        writer = "00000000000000000000"
        for earlier in self.passes:
            if earlier["Id"] >= event["Id"]:
                break
            if name in earlier["OutputResources"]:
                writer = earlier["Pointer"]
        path = self.root / "Resources" / f"{name}.v{writer}.bin"
        desc = self.resources[name.split(".")[0]]
        fmt = desc["Format"]
        height, width = desc["ExtentY"], desc["ExtentX"]
        if fmt == "PF_DepthStencil":
            values = np.fromfile(path, dtype="<f4").reshape(height, width)
        elif fmt == "PF_A2B10G10R10":
            packed = np.fromfile(path, dtype="<u4").reshape(height, width)
            values = np.stack([(packed >> shift) & 1023 for shift in (0, 10, 20)], -1) / 1023
        elif fmt in ("PF_B8G8R8A8", "PF_R8G8B8A8"):
            values = np.fromfile(path, dtype="u1").reshape(height, width, 4) / 255
            if fmt == "PF_B8G8R8A8":
                values = values[..., [2, 1, 0, 3]]
        elif fmt in ("PF_FloatRGBA", "PF_G16R16F", "PF_R16F"):
            channels = {"PF_FloatRGBA": 4, "PF_G16R16F": 2, "PF_R16F": 1}[fmt]
            values = np.fromfile(path, dtype="<f2").reshape(height, width, channels).astype("f4")
        else:
            raise ValueError("unreviewed texture format: " + fmt)
        return values, dict(resource=name, version=writer, path=str(path), sha256=digest(path), description=desc)

    def scalar_parameters(self, event):
        """Decode inline numeric fields using this dump's own shader layout."""
        blob = (self.root / "Structures" / (event["Parameters"] + ".bin")).read_bytes()
        result = {}

        def visit(key, base, prefix):
            for member in self.metadata[key]["Members"]:
                name = prefix + member["Name"]
                offset = base + member["Offset"]
                kind = member["BaseType"]
                if kind in ("UBMT_INCLUDED_STRUCT", "UBMT_NESTED_STRUCT"):
                    visit(member["StructMetadata"], offset, name + ".")
                elif kind in ("UBMT_FLOAT32", "UBMT_INT32", "UBMT_UINT32"):
                    dtype = {"UBMT_FLOAT32": "<f4", "UBMT_INT32": "<i4", "UBMT_UINT32": "<u4"}[kind]
                    width = member["NumRows"] * member["NumColumns"]
                    elements = max(1, member["NumElements"])
                    stride = ((width * 4 + 15) // 16) * 16 if member["NumElements"] else width * 4
                    value = np.concatenate([np.frombuffer(blob, dtype=dtype, count=width, offset=offset + i * stride)
                                            for i in range(elements)])
                    if np.isfinite(value).all():
                        result[name] = value.tolist() if width * elements > 1 else value[0].item()
        visit(event["ParametersMetadata"], 0, "")
        return result


def decode(root, verbose=True):
    dump = Dump(root)
    light, = [p for p in dump.passes if p["EventName"].startswith("Light::StandardDeferred: M6Quarry_Sun")]
    inputs = dump.inputs(light)
    if verbose:
        print("PASSES", [(p["Id"], p["EventName"]) for p in dump.passes])
        print("LIGHT INPUTS", [(key, dump.resources[value.split('.')[0]].get("Format")) for key, value in inputs.items()
                               if any(word in key.lower() for word in ("shadow", "occlusion", "denois", "history", "gbuffera", "scenedepth"))])
    return dump, light, inputs


def capture_view(log, pose):
    lines = [line for line in log.splitlines() if "CameraCachePrivate =" in line]
    if not lines:
        raise ValueError("missing runtime main-view camera readback; recapture")
    line = lines[-1]
    values = re.search(r"Location=\(X=([^,]+),Y=([^,]+),Z=([^\)]+)\),Rotation=\(Pitch=([^,]+),Yaw=([^,]+),Roll=([^\)]+)\),FOV=([^,]+)", line)
    if values is None:
        raise ValueError("unrecognized runtime camera readback")
    if "PerspectiveNearClipPlane=-1.000000" not in line or "OffCenterProjectionOffset=(X=0.000000,Y=0.000000)" not in line:
        raise ValueError("runtime near override or projection offset changed")
    x, y, z, pitch, yaw, roll, fov = map(float, values.groups())
    expected = json.loads((ROOT / "manifest.json").read_text())["poses"][pose]
    eye = np.array([y, z, -x]) / 100
    forward = np.array(expected["target"]) - np.array(expected["eye"])
    forward /= np.linalg.norm(forward)
    h = np.tan(expected["fov_y_radians"] / 2) * 1920 / 1080
    expected_fov = np.degrees(2 * np.arctan(h))
    if np.max(np.abs(eye - expected["eye"])) > 1e-6 or abs(fov - expected_fov) > 1e-5 or abs(roll) > 1e-5:
        raise ValueError("runtime camera differs from registration")
    a, b = np.radians([pitch, yaw])
    ue_forward = np.array([np.cos(a) * np.cos(b), np.cos(a) * np.sin(b), np.sin(a)])
    actual_forward = ue_forward[[1, 2, 0]] * [1, 1, -1]
    if np.max(np.abs(actual_forward - forward)) > 1e-6:
        raise ValueError("runtime camera rotation differs")
    return dict(eye_source_m=eye.tolist(), pitch_yaw_roll_degrees=[pitch, yaw, roll], horizontal_fov_degrees=fov,
                forward_source=actual_forward.tolist(), near_cm=1.,
                near_evidence="r.SetNearClipPlane 0.5 clamps to 1 cm in installed UnrealEngine.cpp SetNearClipPlane; camera inherits global near (-1); device-depth/source-position cross-check retained",
                uniform_buffer_limit="DumpGPU records referenced View uniform layout but not its contents; projection reconstructed from runtime camera and depth")


def score_capture(root, reference=REFERENCE, support=SUPPORT):
    manifest = json.loads((ROOT / "manifest.json").read_text())
    identities = json.loads((reference.parent / "input-identities.json").read_text())
    for key in ("asset", "cameras", "lighting"):
        if identities[key] != manifest["inputs"][key]["sha256"]:
            raise ValueError("registered input identity mismatch: " + key)
    dump, light, inputs = decode(root, verbose=False)
    recipe = json.loads((dump.root.parent / "recipe.json").read_text())
    label, pose = dump.root.parent.name, recipe["pose"]
    log = (ROOT / "launches" / label / "ue.log").read_text()
    output = dump.root.parent / "decoded-v2"
    output.mkdir(exist_ok=False)
    view = capture_view(log, pose)
    depth, depth_id = dump.resource(inputs["SceneDepthZ"], light)
    normal, normal_id = dump.resource(inputs["GBufferA"], light)
    names = [key for key in inputs if key in ("ShadowMaskTexture", "RayTracingOcclusion") or key.startswith("Shadow.Denoiser.DenoiserOutput")]
    if len(names) != 1:
        raise ValueError("review the native attenuation resource: " + str(names))
    attenuation, mask_id = dump.resource(inputs[names[0]], light)
    if attenuation.shape[-1] != 4:
        raise ValueError("native light mask channel layout changed")
    # Directional-light whole-scene surface shadow is R; B multiplies per-object
    # shadowing. Both are sqrt encoded at the deferred-light texture input.
    candidate = attenuation[..., 0] ** 2 * attenuation[..., 2] ** 2
    native_cover = np.isfinite(depth) & (depth > 0)
    valid = native_cover & np.isfinite(candidate)
    encoded_normal = normal * 2 - 1
    source_normal = encoded_normal[..., [1, 2, 0]] * [1, 1, -1]
    forward = np.array(view["forward_source"])
    right = np.cross(forward, [0, 1, 0]); right /= np.linalg.norm(right)
    up = np.cross(right, forward)
    yy, xx = np.indices(depth.shape)
    h = np.tan(np.radians(view["horizontal_fov_degrees"]) / 2)
    ray = forward + ((xx + .5) / 960 - 1)[..., None] * h * right + (1 - (yy + .5) / 540)[..., None] * h * 1080 / 1920 * up
    z = np.zeros_like(depth, dtype="f8")
    np.divide(view["near_cm"] / 100, depth, out=z, where=native_cover)
    world = np.array(view["eye_source_m"]) + z[..., None] * ray
    world[~native_cover] = np.nan
    source_normal[~native_cover] = np.nan
    np.savez_compressed(output / "native.npz", visibility=candidate, valid=valid, device_depth=depth,
                        world_source_m=world.astype("f4"), normal_source=source_normal.astype("f4"), attenuation_rgba=attenuation)
    controls = list(csv.DictReader((dump.root / "Base/ConsoleVariables.csv").open(encoding="utf-8-sig")))
    controls = {v["CVar"]: v for v in controls}
    wanted = [c for c in recipe["commands"] if c.startswith("r.") and " " not in c]
    cvars = {key: controls.get(key) for key in wanted}
    write(output / "effective-cvars.json", cvars)
    # Preserve all shader parameter values, including diagnostic SPP and bias,
    # and the denoiser's measured frequency/history controls for this very frame.
    parameters = {str(p["Id"]) + ":" + p["EventName"]: dump.scalar_parameters(p) for p in dump.passes}
    ray_passes = [p for p in dump.passes if p["EventName"].startswith("RayTracedShadow")]
    if ray_passes:
        ray_values = dump.scalar_parameters(ray_passes[0])
        expected_radius = np.sin(np.radians(4.583662361) / 2)
        if abs(ray_values["Light.SourceRadius"] - expected_radius) > 1e-8:
            raise ValueError("effective shader source radius is stale; property readback alone is insufficient")
        expected_spp = 256 if recipe["configuration"] in ("hrt-256", "hrt-epsilon-zero") else 1
        if ray_values["SamplesPerPixel"] != expected_spp:
            raise ValueError("effective shader SPP differs")
    properties = [line.split("]", 2)[-1] for line in log.splitlines()
                  if re.search(r"\) (DirectionalLightComponent|CameraComponent) .* = ", line)]
    write(output / "readbacks.json", dict(view=view, properties=properties, parameters=parameters,
         mask=mask_id, depth=depth_id, normal=normal_id, missing_cvars=[k for k, v in cvars.items() if v is None]))
    scoring, images = load("ue-quarry-score"), load("ue-quarry-level-b")
    gate = scoring.load_gate(support / "fresh_gate.py")
    ref_path = reference / f"pose-{pose}-reference.npz"
    with np.load(ref_path, allow_pickle=False) as arrays:
        ref, sigma, source_depth, cover = [arrays[k].copy() for k in ("ref", "sigma", "depth", "cover")]
    production = np.fromfile(reference / f"pose-{pose}-seed-0-production.u32", dtype="<u4").reshape(ref.shape) / 256
    record = json.loads((reference / "gate.json").read_text())["poses"][pose]
    q = record["q"]
    production_metrics, _, pops = gate.metrics(production, ref, sigma, source_depth, cover, q)
    if production_metrics != record["seeds"][0]["production_metrics"]:
        raise ValueError("frozen production baseline reproduction failed")
    valid &= cover
    error = np.where(valid, candidate - ref, np.nan)
    populations = {name: dict(reference_n=int(mask.sum()), evaluated_n=int((mask & valid).sum()),
                   missing=int((mask & ~valid).sum()), candidate=scoring.summary(error[mask & valid]),
                   production=scoring.summary((production - ref)[mask & valid])) for name, mask in pops.items()}
    partial, solid = pops["partial"], pops["lit"] | pops["occluded"]
    envelope = np.maximum(np.maximum(gate.Z_ENVELOPE * sigma, q), gate.QUANTUM)
    contour = gate.along_contour(error, ref, partial, source_depth)
    finite = np.isfinite(contour)
    extremes = valid & partial & ((candidate <= 0) | (candidate >= 1))
    ys, xs = np.nonzero(extremes)
    with (output / "G6.csv").open("w", newline="") as stream:
        writer = csv.writer(stream); writer.writerow(["x", "y", "reference", "candidate", "sigma", "error"])
        writer.writerows((int(x), int(y), float(ref[y, x]), float(candidate[y, x]), float(sigma[y, x]), float(error[y, x])) for y, x in zip(ys, xs))
    result = dict(configuration=recipe["configuration"], label=label, pose=pose, reference_sha256=digest(ref_path),
                  metrics_sha256=digest(Path(gate.__file__)), production_full_domain=production_metrics,
                  full_domain_gate=None if np.any(cover & ~valid) else gate.metrics(candidate, ref, sigma, source_depth, cover, q)[0],
                  coverage=dict(source=int(cover.sum()), native=int(native_cover.sum()), evaluated=int(valid.sum()),
                                source_only=int((cover & ~native_cover).sum()), native_only=int((native_cover & ~cover).sum())),
                  populations=populations, G1=dict(violations=int((valid & partial & (np.abs(error) > envelope)).sum()), missing=int((partial & ~valid).sum())),
                  G2=dict(maximum=float(np.abs(error[valid & solid]).max()), over_002=int((valid & solid & (np.abs(error) > .002)).sum())),
                  G4=dict(evaluated=int(finite.sum()), missing=int((~finite).sum()), statistics=scoring.summary(contour[finite])),
                  G6=dict(count=int(extremes.sum()), csv="G6.csv"),
                  G5="static independent process at each pose; no contiguous motion/return history claim",
                  encoding="BGRA reordered to RGBA where applicable; directional surface = squared R * squared B; native depth coverage required",
                  verdict="Level B native-receiver profile against unchanged biased Renderlab reference; no parity inferred")
    for name, value, mask in (("reference", ref, cover), ("candidate", candidate, valid), ("production", production, cover)):
        Image.fromarray(images.visibility_image(value, cover, mask)).save(output / (name + ".png"))
    Image.fromarray(images.error_image(error, cover, valid)).save(output / "error-x8.png")
    # Inspect known causal probes alongside the actual worst errors, without
    # claiming that enumerating G6 is equivalent to inspecting every crop.
    points = [(1203, 669), (928, 325), (1388, 529), (1326, 607), (945, 276), (1577, 595), (1254, 549)]
    yy, xx = np.nonzero(partial & valid)
    points += [(int(xx[i]), int(yy[i])) for i in np.argsort(np.abs(error[yy, xx]))[-4:]]
    panels = [Image.open(output / name) for name in ("reference.png", "production.png", "candidate.png", "error-x8.png")]
    sheet = Image.new("RGB", (len(points) * 130, 4 * 130))
    for i, (x, y) in enumerate(points):
        for j, panel in enumerate(panels):
            crop = panel.crop((x - 16, y - 16, x + 17, y + 17)).resize((130, 130), Image.Resampling.NEAREST)
            sheet.paste(crop, (i * 130, j * 130))
    sheet.save(output / "crops.png")
    for panel in panels:
        panel.close()
    result["crop_points"] = points
    result["probe_values"] = [dict(x=x, y=y, reference=float(ref[y, x]), candidate=float(candidate[y, x]),
                                      native_covered=bool(native_cover[y, x]), normal_source=source_normal[y, x].tolist(),
                                      world_source_m=world[y, x].tolist()) for x, y in points[:7]]
    temporal = [p for p in dump.passes if p["EventName"] == "SSD TemporalAccumulation"]
    if temporal:
        event, = temporal
        history = {}
        identities = {}
        for kind, resources in (("previous", event["InputResources"]), ("current", event["OutputResources"])):
            names = [name for name in resources if dump.resources[name.split(".")[0]]["Name"] == "Shadow.Denoiser.TemporalAccumulation0"]
            if len(names) != 1:
                raise ValueError("review native temporal signal batch/layout")
            history[kind], identities[kind] = dump.resource(names[0], dict(Id=event["Id"] + 1) if kind == "current" else event)
        np.savez_compressed(output / "denoiser-history.npz", **history)
        def distribution(value, mask):
            value = value[mask]
            return dict(n=int(value.size), quantiles_0_10_50_90_99_100=np.quantile(value, [0, .1, .5, .9, .99, 1]).tolist(),
                        at_most_one=int((value <= 1.001).sum()), exactly_one=int((value == 1).sum()))
        result["denoiser"] = dict(
            layout="PENUMBRA_HISTORY: R normalized miss ratio, G accumulated sample count, B world blur radius (cm), A transmission distance",
            identities=identities, camera_cut_first_signal=dump.scalar_parameters(event)["bCameraCut"][0],
            partial_sample_count={kind: distribution(value[..., 1], partial & valid) for kind, value in history.items()},
            partial_world_blur_radius_cm={kind: distribution(value[..., 2], partial & valid) for kind, value in history.items()})
    source_world = np.fromfile(reference / f"pose-{pose}-world.f32", dtype="<f4").reshape(1080, 1920, 4)
    result["registered_source_projection"] = images.check_projection(source_world, cover, manifest["poses"][pose])
    distance_mm = np.linalg.norm(world[valid] - source_world[valid, :3], axis=-1) * 1000
    result["native_source_receiver_displacement_mm_quantiles_0_50_90_99_100"] = np.quantile(distance_mm, [0, .5, .9, .99, 1]).tolist()
    result["scoring_script_sha256"] = digest(Path(__file__))
    result["fresh_gate_facade_sha256"] = digest(support / "fresh_gate.py")
    write(output / "scores.json", result)
    print(label, "partial RMS", result["populations"]["partial"]["candidate"]["rms"],
          "missing", result["coverage"]["source_only"], flush=True)
    return result


def crosscheck(root, unshadowed):
    """Cross-check with main-light output before ReflectionEnvironmentAndSky.

    The registered opaque material has no emissive term and indirect/AO are off.
    Full-tree discovery proves that the sky pass follows this light. Do not
    subtract SceneColor v0: the uniform's unused target alias can be read back
    after the writer in DumpGPU. Keep the ratio separate from native attenuation.
    """
    def direct(path):
        dump, light, inputs = decode(path, verbose=False)
        after, identity = dump.resource(inputs["SceneColor"], dict(Id=light["Id"] + 1))
        depth, _ = dump.resource(inputs["SceneDepthZ"], light)
        normal, _ = dump.resource(inputs["GBufferA"], light)
        recipe = json.loads((dump.root.parent / "recipe.json").read_text())
        return after[..., :3].astype("f8"), identity, recipe, depth, normal
    a, shadowed_id, a_recipe, a_depth, a_normal = direct(root)
    b, unshadowed_id, b_recipe, b_depth, b_normal = direct(unshadowed)
    if a_recipe["pose"] != b_recipe["pose"] or a_recipe["configuration"].split("-")[0] != b_recipe["configuration"].split("-")[0]:
        raise ValueError("RGB pair pose/light recipe differs")
    if not b_recipe["configuration"].endswith("-unshadowed"):
        raise ValueError("expected explicitly unshadowed main-view pair")
    scores = root / "decoded-v2"
    with np.load(scores / "native.npz", allow_pickle=False) as data:
        native, coverage = data["visibility"], data["valid"]
    scoring = load("ue-quarry-score")
    ratio, valid, details = scoring.visibility(a, b, coverage)
    weights = np.array([.2126, .7152, .0722])
    difference = ratio - native
    same_receiver = (a_depth == b_depth) & np.all(a_normal == b_normal, axis=-1)
    output = dict(extraction=details, mask_minus_rgb=scoring.summary(-difference[valid]),
                  denominator_quantiles=np.quantile((b @ weights)[coverage], [0, .01, .1, .5, .9, 1]).tolist(),
                  dark_native_receivers=int((coverage & ~valid).sum()),
                  pairs=dict(shadowed=shadowed_id, unshadowed=unshadowed_id), brighter_controls={})
    output["receiver_crosscheck"] = dict(
        exact_depth_and_normal=int((valid & same_receiver).sum()), changed_depth_or_normal=int((valid & ~same_receiver).sum()),
        same_receiver_ratio_minus_mask=scoring.summary(difference[valid & same_receiver]),
        all_error_over_01=int((valid & (np.abs(difference) > .01)).sum()),
        same_receiver_error_over_01=int((valid & same_receiver & (np.abs(difference) > .01)).sum()))
    for threshold in (.01, .1, 1):
        use = valid & ((b @ weights) > threshold)
        output["brighter_controls"][str(threshold)] = scoring.summary(difference[use])
    write(scores / "rgb-crosscheck.json", output)
    print("RGB CROSSCHECK", output["mask_minus_rgb"], "native zero-direct", output["dark_native_receivers"])
    return output


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("--score", action="store_true")
    parser.add_argument("--crosscheck", type=Path, help="separate unshadowed native main-view dump")
    args = parser.parse_args()
    if args.score:
        score_capture(args.root)
    if args.crosscheck:
        crosscheck(args.root, args.crosscheck)
    if not args.score and not args.crosscheck:
        decode(args.root)
