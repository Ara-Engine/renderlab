"""TR0 S2G reference registration, bounded tracer batches and fixed event ages.

Primary-centre geometry comes from U0b's tracer. Ages are benchmark masks shared
by every candidate, never inputs to a renderer. S2G's cameras only translate;
lighting changes are measured separately from geometric disocclusion age.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import uuid
import zipfile

import numpy as np

ROOT = Path("E:/renderlab-work/m7-tr0")
OUT = ROOT / "s2g"
TRACKS = ("s2g-camera-0.25px", "s2g-camera-0.5px", "s2g-camera-1px", "s2g-sun-0.25deg")
QUALIFIED = "bf169ef7f8599bbff53bc8edcdb1875e8c117b09630e1c912e0f0f5d73335dfb"
SOURCE = Path("E:/renderlab-work/m7-u0b/scenes/S2G/scene.json")


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n", encoding="utf-8")


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def key(pose, lighting=False):
    values = pose["eye"] + pose["target"] + [pose["fov_y_radians"]]
    if lighting:
        values += pose["to_light"]
    # Match the effective f32 View inputs, without fitted camera alignment.
    return hashlib.sha256(np.asarray(values, "<f4").tobytes()).hexdigest()


def sequences():
    return {name: json.loads((ROOT / "inputs" / (name + ".json")).read_text())["frames"] for name in TRACKS}


def plan(_):
    groups = {"geometry": {}, "physical": {}}
    for name, frames in sequences().items():
        for i, pose in enumerate(frames):
            for kind, cases in groups.items():
                identity = key(pose, lighting=kind == "physical")
                case = cases.setdefault(identity, dict(key=identity, track=name, frame=i, aliases=[]))
                case["aliases"].append(dict(track=name, frame=i))
    result = {kind: list(cases.values()) for kind, cases in groups.items()}
    result["source_sequences"] = {name: sha(ROOT / "inputs" / (name + ".json")) for name in TRACKS}
    result["source_scene_sha256"] = sha(SOURCE)
    result["scope"] = "All 640 registered frames, exact f32 pose/light aliases; 1920x1080, zero shutter, black primary."
    save(OUT / "plan.json", result)
    print("S2G plan", {kind: len(cases) for kind, cases in groups.items()}, "640 registered frames")


def core():
    path = Path(__file__).with_name("m7-tr0.py")
    spec = importlib.util.spec_from_file_location("tr0", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def material_instances():
    scene = json.loads(SOURCE.read_text())
    return {name: [i + 1 for i, mesh in enumerate(scene["meshes"]) if mesh["name"] in names]
            for name, names in scene["populations"].items()}


def collect(case, archive, metadata, kind):
    """Retain centre geometry independently from low-spp colour; physical archives stay lossless."""
    destination = OUT / "geometry" / (key(sequences()[case["track"]][case["frame"]]) + ".npz")
    if not destination.exists() and not (OUT / "ages/protocol.json").exists():
        destination.parent.mkdir(parents=True, exist_ok=True)
        with np.load(archive) as data:
            np.savez_compressed(destination, **{k: data[k] for k in ("world", "hit", "depth")})
        save(destination.with_suffix(".json"), dict(source_archive=str(archive), source_sha256=sha(archive),
             geometry_sha256=sha(destination), samples=metadata["samples"], qualification=metadata.get("qualification"),
             note="Only deterministic primary-centre world/IDs/depth; low-spp radiance is never a film target."))
    if kind == "physical":
        name = f"{case['track']}-{case['frame']:03}"
        target = OUT / "physical" / (name + ".npz")
        target.parent.mkdir(parents=True, exist_ok=True)
        if target.exists() and sha(target) != sha(archive):
            if not target.with_suffix(".json").exists() or json.loads(target.with_suffix(".json").read_text()).get("full_archive_sha256") != sha(archive):
                raise ValueError("refusing to replace a different physical reference")
        if not target.exists():
            # Centre geometry is retained separately for age generation. Drop
            # its redundant copies without altering any scored array bytes.
            omitted, retained = {}, {}
            with zipfile.ZipFile(archive) as source, zipfile.ZipFile(target, "w", compression=zipfile.ZIP_DEFLATED) as compact:
                for info in source.infolist():
                    payload = source.read(info)
                    identity = hashlib.sha256(payload).hexdigest()
                    if info.filename in ("world.npy", "geometric_normal.npy", "n.npy", "depth.npy"):
                        omitted[info.filename] = identity
                    else:
                        compact.writestr(info.filename, payload)
                        retained[info.filename] = identity
            with zipfile.ZipFile(target) as compact:
                assert all(hashlib.sha256(compact.read(name)).hexdigest() == identity for name, identity in retained.items())
            metadata = dict(metadata, full_archive_sha256=sha(archive), archive_sha256=sha(target),
                            omitted_array_sha256=omitted, retained_array_sha256=retained,
                            retention="Scored L/D/V halves, masks and centre hit IDs are byte-identical NPY payloads. Redundant world/normal/depth/counts omitted; geometry/age provenance is separate.")
        else:
            metadata = json.loads(target.with_suffix(".json").read_text())
        metadata = dict(metadata, population_instances=material_instances(), corpus_key=case["key"], aliases=case["aliases"])
        save(target.with_suffix(".json"), metadata)


def import_qualified(args):
    registration = json.loads((OUT / "plan.json").read_text())
    meta = json.loads(args.metadata.read_text())
    archive = args.metadata.with_suffix(".npz")
    if not meta.get("accepted_for_scoring") or sha(archive) != meta["archive_sha256"]:
        raise ValueError("provider archive is not qualified or its bytes changed")
    if meta["source_scene_sha256"] != registration["source_scene_sha256"]:
        raise ValueError("provider S2G source differs from registration")
    frame = meta["frame"]
    pose = dict(frame["open"], to_light=frame["to_light"], fov_y_radians=frame["fov_y"])
    case = next(c for c in registration["physical"] if c["key"] == key(pose, True))
    collect(case, archive, meta, "physical")
    print("reused qualified physical/centre-geometry reference", case["track"], case["frame"])


def trace(args):
    """Each batch blocks on core.reference's own short hidden GPU lease, then compacts on CPU."""
    if sha(args.binary) != QUALIFIED:
        raise ValueError("pin the U0b-qualified binary before tracing")
    registration = json.loads((OUT / "plan.json").read_text())
    if sha(SOURCE) != registration["source_scene_sha256"]:
        raise ValueError("registered S2G source changed")
    pending = []
    for case in registration[args.kind]:
        target = (OUT / "geometry" / (case["key"] + ".npz") if args.kind == "geometry" else
                  OUT / "physical" / f"{case['track']}-{case['frame']:03}.npz")
        if not target.exists():
            pending.append(case)
    module = core()
    completed = 0
    while pending and completed < args.limit:
        if shutil.disk_usage(ROOT).free < 5 * 1024 ** 3:
            raise SystemExit("deferred before GPU lease: less than 5 GiB disk headroom")
        track = pending[0]["track"]
        batch = []
        while pending and pending[0]["track"] == track and len(batch) < min(3, args.limit - completed):
            batch.append(pending.pop(0))
        label = f"sg-{'geo' if args.kind == 'geometry' else 'film'}-{uuid.uuid4().hex[:8]}"
        print("S2G batch", label, track, [c["frame"] for c in batch], flush=True)
        module.reference(argparse.Namespace(label=label, track=track, frames=[c["frame"] for c in batch],
             samples=2 if args.kind == "geometry" else 1024, binary=args.binary,
             compact_tool=args.compact_tool, minutes=3, wait_seconds=args.wait_seconds))
        run = ROOT / "runs" / label
        removed = []
        for case in batch:
            name = f"{case['track']}-{case['frame']:03}"
            archive = run / "data" / (name + ".npz")
            meta = json.loads(archive.with_suffix(".json").read_text())
            meta.update(accepted_for_scoring=args.kind == "physical", qualification=dict(binary_sha256=QUALIFIED,
                        provider="U0b eight independent-AS/pipeline rebuilds plus analytic/Mitsuba controls",
                        batch_replay=str(run / "reference-readback.json")))
            collect(case, archive, meta, args.kind)
            removed.append(dict(path=str(archive), bytes=archive.stat().st_size, sha256=sha(archive)))
            archive.unlink()
        pin = run / "reference-tests.exe"
        removed.append(dict(path=str(pin), bytes=pin.stat().st_size, sha256=sha(pin)))
        pin.unlink()  # The immutable hash-named common binary remains available.
        save(run / "removed-transferred-inputs.json", removed)
        completed += len(batch)
        print("S2G collected", completed, "remaining", len(pending), flush=True)


def basis(pose):
    eye, target = [np.asarray(pose[k], "f4").astype("f8") for k in ("eye", "target")]
    forward = target - eye
    forward /= np.linalg.norm(forward)
    right = np.cross(forward, [0., 1., 0.])
    right /= np.linalg.norm(right)
    return eye, forward, right, np.cross(right, forward)


def advance_age(previous, current, old_pose, pose, old_age):
    """Nearest previous-centre fetch, same placement and 2%-forward-depth acceptance.

    Current primary misses keep age only where the previous pixel also missed;
    this infinity projection is valid for S2G's translation-only camera tracks.
    """
    eye, forward, right, up = basis(old_pose)
    if not np.allclose(forward, basis(pose)[1], rtol=0, atol=1e-6):
        raise ValueError("S2G age protocol requires a translation-only camera")
    height, width = old_age.shape
    offset = current["world"].astype("f8") - eye
    depth = offset @ forward
    safe_depth = np.maximum(depth, 1e-20)
    tangent = np.tan(np.float32(old_pose["fov_y_radians"]) * .5)
    x = (offset @ right / (safe_depth * tangent * width / height) * .5 + .5) * width
    y = (-offset @ up / (safe_depth * tangent) * .5 + .5) * height
    inside = (depth > 0) & (x >= 0) & (x < width) & (y >= 0) & (y < height)
    px = np.clip(np.floor(x), 0, width - 1).astype("i4")
    py = np.clip(np.floor(y), 0, height - 1).astype("i4")
    placement = current["hit"][..., 0]
    old_placement = previous["hit"][..., 0]
    covered = placement != 0
    valid = covered & inside & (placement == old_placement[py, px])
    valid &= np.abs(previous["depth"][py, px] - depth) <= .02 * safe_depth
    age = np.zeros_like(old_age)
    age[valid] = np.minimum(old_age[py[valid], px[valid]].astype("u2") + 1, 255).astype("u1")
    background = ~covered & (old_placement == 0)
    age[background] = np.minimum(old_age[background].astype("u2") + 1, 255).astype("u1")
    return age


def inputs(args):
    """Compare unjittered native G-buffers with authored interpolation, not face normals."""
    from scipy.ndimage import distance_transform_edt
    module = core()
    run = ROOT / "runs" / args.label
    recipe = json.loads((run / "recipe.json").read_text())
    if recipe.get("aa") != "none" or recipe.get("static_source_frame") is None:
        raise ValueError("input audit requires an unjittered static control")
    pose = recipe["frames"][0]
    registration = json.loads((OUT / "plan.json").read_text())
    case = next(c for c in registration["physical"] if c["key"] == key(pose, True))
    archive = OUT / "physical" / f"{case['track']}-{case['frame']:03}.npz"
    with np.load(archive) as data:
        hit, world, edge = data["hit"], data["world"], data["pop_edge"]
    dump = module.NATIVE.s1.native.Dump(run / "dump")
    consumer = next(p for p in dump.passes if {"GBufferA", "GBufferB", "GBufferC"} <=
                    {dump.resources[n.split('.')[0]]["Name"] for n in p["InputResources"]})
    buffers, identities = {}, {}
    for name in ("GBufferA", "GBufferB", "GBufferC"):
        resource = next(n for n in consumer["InputResources"] if dump.resources[n.split('.')[0]]["Name"] == name)
        buffers[name], identities[name] = module.NATIVE.resource(dump, resource, consumer)
    normal = buffers["GBufferA"][..., :3] * 2 - 1
    normal = normal[..., [1, 2, 0]] * [1, 1, -1]  # UE world -> registered source world.
    normal /= np.maximum(np.linalg.norm(normal, axis=-1, keepdims=True), 1e-30)
    encoded = buffers["GBufferC"][..., :3]
    base_color = np.where(encoded <= .04045, encoded / 12.92, ((encoded + .055) / 1.055) ** 2.4)
    scene = json.loads(SOURCE.read_text())
    interior = (hit[..., 0] != 0) & (distance_transform_edt(~edge) > 2)
    result = []
    eye = np.asarray(pose["eye"], "f4").astype("f8")
    for index, mesh in enumerate(scene["meshes"]):
        selected = interior & (hit[..., 0] == index + 1)
        if not selected.any():
            continue
        triangles = np.asarray(mesh["indices"], "i4").reshape(-1, 3)[hit[..., 1][selected] - 1]
        vertices = np.asarray(mesh["positions"], "f4").astype("f8")[triangles]
        authored = np.asarray(mesh["normals"], "f4").astype("f8")[triangles]
        a, b = vertices[:, 1] - vertices[:, 0], vertices[:, 2] - vertices[:, 0]
        offset = world[selected].astype("f8") - vertices[:, 0]
        dot = lambda x, y: np.einsum("ij,ij->i", x, y)
        aa, ab, bb, ap, bp = dot(a, a), dot(a, b), dot(b, b), dot(a, offset), dot(b, offset)
        denominator = aa * bb - ab * ab
        if np.any(np.abs(denominator) < 1e-25):
            raise ValueError("degenerate source triangle in shading-normal audit")
        u, v = (bb * ap - ab * bp) / denominator, (aa * bp - ab * ap) / denominator
        expected = authored[:, 0] + u[:, None] * (authored[:, 1] - authored[:, 0]) + v[:, None] * (authored[:, 2] - authored[:, 0])
        face = np.cross(a, b)
        back = dot(face, eye - world[selected]) < 0
        expected *= np.where(back, -1, 1)[:, None]
        expected /= np.maximum(np.linalg.norm(expected, axis=-1, keepdims=True), 1e-30)
        angles = np.degrees(np.arccos(np.clip(dot(expected, normal[selected]), -1, 1)))
        material = scene["materials"][mesh["material"]]
        b_values = buffers["GBufferB"][selected]
        result.append(dict(mesh=mesh["name"], n=int(selected.sum()), source_back_faces=int(back.sum()),
            shading_normal_angle_degrees=np.quantile(angles, [.5, .95, .99, 1]).tolist(),
            expected=dict(roughness=material["roughness"], metallic=material["metallic"],
                          ue_specular=.5 * material["specular"], base_color=material["base_color"]),
            actual=dict(metallic_quantiles=np.quantile(b_values[:, 0], [.01, .5, .99]).tolist(),
                        specular_quantiles=np.quantile(b_values[:, 1], [.01, .5, .99]).tolist(),
                        roughness_quantiles=np.quantile(b_values[:, 2], [.01, .5, .99]).tolist(),
                        base_color_mean=base_color[selected].mean(0).tolist())))
    save(run / "input-readback.json", dict(rows=result, identities=identities, consumer=consumer["EventName"],
        reference=str(archive), reference_sha256=sha(archive), source_scene_sha256=sha(SOURCE),
        note="Unjittered static source-frame control; geometric interior >2px. Reference hit primitive/world reconstruct authored vertex-normal interpolation, with source-back reversal. GBufferC sRGB decoded; no fitted alignment or normal sign selection."))
    print(json.dumps(result, indent=2))


def colour_control(args):
    """Trace actual main-colour consumers and compare fixed physical-D masks."""
    module = core()
    run = ROOT / "runs" / args.label
    recipe = json.loads((run / "recipe.json").read_text())
    capture = json.loads((run / "capture-readback.json").read_text())
    if recipe.get("program") != "sun" or len(recipe["frames"]) != 1 or recipe.get("static_source_frame") is None:
        raise ValueError("colour control requires a one-frame static sun-only capture")
    if capture["exposure_readback_frames"] != 1:
        raise ValueError("measured manual pre-exposure is required")
    pose = recipe["frames"][0]
    registration = json.loads((OUT / "plan.json").read_text())
    case = next(c for c in registration["physical"] if c["key"] == key(pose, True))
    archive = OUT / "physical" / f"{case['track']}-{case['frame']:03}.npz"
    metadata = json.loads(archive.with_suffix(".json").read_text())
    if sha(archive) != metadata["archive_sha256"]:
        raise ValueError("physical reference hash changed")
    with np.load(archive) as data:
        reference = (data["D0"] + data["D1"]) * .5
        hit = data["hit"]
    measured_pe = capture["identities"][0]["exposure"]["values"][0]
    registered_pe = 2 ** recipe.get("registered_exposure_stops", recipe["exposure_stops"])
    reference_luma = reference @ np.array([.2126, .7152, .0722])
    dump = module.NATIVE.s1.native.Dump(run / "dump")
    post_consumer, post_name = module.colour_input(dump, dump.passes, recipe)
    stages = [("tonemap_input", post_consumer, post_name)]
    if recipe.get("aa", "tsr") == "tsr":
        rejects = [p for p in dump.passes if p["EventName"].startswith("TSR RejectShading(")]
        if len(rejects) != 1:
            raise ValueError("expected exactly one actual RejectShading consumer")
        reject = rejects[0]
        names = [n for n in reject["InputResources"] if dump.resources[n.split('.')[0]]["Name"] == "SceneColor"]
        if len(names) != 1:
            raise ValueError("expected exactly one SceneColor input to RejectShading")
        stages.insert(0, ("reject_input", reject, names[0]))
    rows, identities, consumers = {}, {}, {}
    image_codes = {"physical D": module.load("m6-temporal-gpu-score").codes(reference * registered_pe, 0)}
    for stage, consumer, name in stages:
        rgb, identity = module.NATIVE.resource(dump, name, consumer)
        if stage == "tonemap_input" and identity["sha256"] != capture["identities"][0]["hdr"]["sha256"]:
            raise ValueError("actual Tonemap input differs from capture proof")
        physical = rgb[..., :3] / measured_pe
        luma = physical @ np.array([.2126, .7152, .0722])
        identities[stage], consumers[stage] = identity, consumer["EventName"]
        image_codes[stage] = module.load("m6-temporal-gpu-score").codes(physical * registered_pe, 0)
        for population, instances in metadata["population_instances"].items():
            mask = np.isin(hit[..., 0], instances)
            highlight = mask & (reference_luma >= np.quantile(reference_luma[mask], .95))
            rows.setdefault(population, {})[stage] = dict(
                highlight_energy_ratio=float(luma[highlight].sum() / reference_luma[highlight].sum()),
                whole_material_energy_ratio=float(luma[mask].sum() / reference_luma[mask].sum()),
                highlight_pixels=int(highlight.sum()), material_pixels=int(mask.sum()))
    # A retained scientific image uses the same registered F0 transform for all
    # stages, including the exposure diagnostic, without an automatic gain.
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    figure, axes = plt.subplots(1, len(image_codes), figsize=(6 * len(image_codes), 4))
    for axis, (stage, codes) in zip(np.atleast_1d(axes), image_codes.items(), strict=True):
        axis.imshow(np.clip(codes / 255, 0, 1))
        axis.set_title(stage)
        axis.axis("off")
    figure.suptitle(f"{args.label}: fixed registered -12 EV display; measured PE {measured_pe:g}")
    figure.tight_layout()
    figure.savefig(run / "colour-control.png", dpi=160)
    plt.close(figure)
    result = dict(rows=rows, identities=identities, consumers=consumers,
        reference=str(archive), reference_sha256=sha(archive), measured_pre_exposure=measured_pe,
        registered_exposure_stops=recipe.get("registered_exposure_stops", recipe["exposure_stops"]),
        diagnostic_only=recipe.get("diagnostic_only", False),
        mask_definition="Top 5% physical-D luminance within fixed qualified current-centre material masks; whole-material sums also retained. Same reference and masks at all exposures.",
        conversion="Divide native RGB by measured manual pre-exposure, then use registered exposure for display/scoring. No fitted gain.")
    save(run / "colour-control-readback.json", result)
    print(json.dumps(dict(rows=rows, measured_pre_exposure=measured_pe), indent=2))


def ages(_):
    registration = json.loads((OUT / "plan.json").read_text())
    missing = [case["key"] for case in registration["geometry"] if not (OUT / "geometry" / (case["key"] + ".npz")).exists()]
    if missing:
        raise SystemExit(f"geometry stage incomplete: {len(missing)} distinct views missing; no ages generated")
    cached_key, cached = None, None
    records = []
    for name, frames in sequences().items():
        previous, old_pose, age = None, None, None
        for i, pose in enumerate(frames):
            identity = key(pose)
            path = OUT / "geometry" / (identity + ".npz")
            if identity != cached_key:
                with np.load(path) as data:
                    cached = {k: data[k] for k in ("world", "hit", "depth")}
                cached_key = identity
            if i == 0:
                age = np.zeros(cached["depth"].shape, "u1")
            else:
                age = advance_age(previous, cached, old_pose, pose, age)
            destination = OUT / "ages" / name / f"f{i:03}.npz"
            destination.parent.mkdir(parents=True, exist_ok=True)
            np.savez_compressed(destination, age=age)
            records.append(dict(track=name, frame=i, geometry_key=identity, age_sha256=hashlib.sha256(age.tobytes()).hexdigest(),
                                pixels=int(age.size), age0=int((age == 0).sum()), age32=int((age >= 32).sum())))
            previous, old_pose = cached, pose
        print("geometry ages", name, len(frames), flush=True)
    save(OUT / "ages/protocol.json", dict(records=records, source_sequences=registration["source_sequences"],
        definition="Fixed external benchmark ages: nearest previous centre, same instance, 2% forward depth; age0 on rejection, +1 on acceptance capped255. Black infinity is same-pixel for pure translation. Sun changes do not reset geometric age.",
        scope="S2G only; does not replace recovered original T1b age masks. Offline scoring only, no renderer consumes these ages."))


def check(_):
    """Analytic plane controls test history acceptance, view boundaries and rejection."""
    width = height = 8
    pose = dict(eye=[0., 0., 2.], target=[0., 0., 0.], fov_y_radians=float(np.float32(.85)))
    y, x = np.indices((height, width))
    tangent = np.tan(np.float32(pose["fov_y_radians"]) * .5)
    world = np.stack(((2 * (x + .5) / width - 1) * 2 * tangent,
                      (1 - 2 * (y + .5) / height) * 2 * tangent, np.zeros_like(x)), -1).astype("f4")
    geometry = dict(world=world, hit=np.ones((height, width, 3), "u4"), depth=np.full((height, width), 2., "f4"))
    age = np.full((height, width), 9, "u1")
    assert np.array_equal(advance_age(geometry, geometry, pose, pose, age), np.full_like(age, 10))
    changed = dict(geometry, hit=np.full_like(geometry["hit"], 2))
    assert not advance_age(geometry, changed, pose, pose, age).any()
    farther = dict(geometry, world=world.copy())
    farther["world"][..., 2] = -.1
    assert not advance_age(geometry, farther, pose, pose, age).any()
    shift = 4 * tangent / width
    moved_pose = dict(pose, eye=[shift, 0., 2.], target=[shift, 0., 0.])
    moved = dict(geometry, world=world.copy())
    moved["world"][..., 0] += shift
    translated = advance_age(geometry, moved, pose, moved_pose, age)
    assert np.all(translated[:, :-1] == 10) and np.all(translated[:, -1] == 0)
    print("S2G geometry-age controls pass: static, placement, depth rejection, newly revealed boundary")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("plan", "ages", "check"):
        commands.add_parser(name)
    p = commands.add_parser("import")
    p.add_argument("metadata", type=Path)
    p = commands.add_parser("trace")
    p.add_argument("kind", choices=("geometry", "physical"))
    p.add_argument("--binary", type=Path, default=ROOT / "binaries" / (QUALIFIED + ".exe"))
    p.add_argument("--compact-tool", type=Path, required=True)
    p.add_argument("--limit", type=int, default=3)
    p.add_argument("--wait-seconds", type=int, default=120)
    commands.add_parser("inputs").add_argument("label")
    commands.add_parser("colour-control").add_argument("label")
    args = parser.parse_args()
    {"plan": plan, "ages": ages, "check": check, "import": import_qualified, "trace": trace, "inputs": inputs,
     "colour-control": colour_control}[args.command](args)
