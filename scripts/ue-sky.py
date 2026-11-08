"""UE-SKY: isolated cook, hidden captures and CPU scoring. Bulk output stays on D:.

Reuses the repository's guarded process/lease implementation. Never runs Cargo,
changes an existing package, or copies engine source. See docs/ue-sky.md.
"""
import argparse
import contextlib
import datetime as dt
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time

import numpy as np

REPO = Path(__file__).resolve().parents[1]
ROOT = Path("D:/Coding/renderlab-work/ue-sky")
PACKAGE = Path("D:/Coding/renderlab-work/ue-sky-package")
SOURCE = Path("E:/renderlab-work/m7-u0/ue-project")
SKY = Path("D:/Coding/renderlab-work/m7-refs-sky")
REFERENCES = ROOT / "reference-tr1b-k"
BASELINES = ROOT / "baseline-scores"
PROJECT = ROOT / "ue-project/M5Calibration.uproject"


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), REPO / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n", encoding="utf-8")


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def host():
    helper = load("m7-u0b")
    helper.OUT = ROOT
    u = helper.ue()
    u.OUT, u.OWNER, u.PROJECT = ROOT, "ue-sky", PROJECT
    u.PACKAGE_DIR = PACKAGE
    u.PACKAGE = PACKAGE / "Windows/Engine/Binaries/Win64/UnrealGame.exe"
    u.UE_FLAGS = ["-corelimit=2" if f == "-corelimit=8" else f for f in u.UE_FLAGS]
    inherited_tool = u.engine_tool
    u.engine_tool = lambda p: False if re.fullmatch(r"renderlab-[0-9a-f]+\.exe", p["Name"].lower()) else inherited_tool(p)
    return helper, u


def announce_cpu(label):
    _, u = host()
    if quiet_conflicts(u):
        raise SystemExit("CPU work deferred by foreign QUIET reservation")
    u.schedule_line(f"[ue-sky CPU-only] {label}; no GPU/test binary/Unreal process. {u.utc().isoformat()}")
    print("CPU-only:", label, flush=True)


def quiet_conflicts(u):
    """A released quiet reservation is complete; otherwise respect its stated end."""
    lines = u.SCHEDULE.read_text(encoding="utf-8").splitlines()
    active = []
    for i, line in enumerate(lines):
        heading = re.match(r"^\[([^\]]+) QUIET (?:active|window)\]", line)
        if not heading:
            continue
        label = re.search(r"label=([^ ;]+)", line)
        end = re.search(r"end=([^ ;]+)", line)
        if label and any(label[1] in later and ("window released" in later or "QUIET completed" in later) for later in lines[i + 1:]):
            continue
        if not label and any(later.startswith(heading[1] + " ") and "window released" in later for later in lines[i + 1:]):
            continue
        if end and dt.datetime.fromisoformat(end[1]) <= u.utc():
            continue
        active.append(line)
    return active


@contextlib.contextmanager
def lease(label, minutes):
    _, u = host()
    conflicts = quiet_conflicts(u)
    if conflicts:
        raise SystemExit("deferred by QUIET reservation: " + repr(conflicts))
    u.schedule_line(f"[ue-sky request] label={label} minutes={minutes}; hidden/offscreen only; {u.utc().isoformat()}")
    with u.lease(label, minutes) as owner:
        yield u, owner


def clone(_):
    announce_cpu("copy source content/config into isolated D: project")
    if (ROOT / "clone.json").exists():
        raise SystemExit("verified project copy already exists")
    PROJECT.parent.mkdir(parents=True, exist_ok=True)
    records = []
    for folder in ("Content", "Config"):
        shutil.copytree(SOURCE / folder, PROJECT.parent / folder, dirs_exist_ok=True)
        for p in (SOURCE / folder).rglob("*"):
            if p.is_file():
                copied = PROJECT.parent / p.relative_to(SOURCE)
                digest = sha(p)
                if sha(copied) != digest:
                    raise ValueError("copy differs: " + str(p))
                records.append(dict(path=str(p.relative_to(SOURCE)), bytes=p.stat().st_size, sha256=digest))
    shutil.copyfile(SOURCE / PROJECT.name, PROJECT)
    config = PROJECT.parent / "Config/DefaultEngine.ini"
    text = config.read_text(encoding="utf-8-sig").replace((SOURCE / "DerivedDataCache").as_posix(), (PROJECT.parent / "DerivedDataCache").as_posix())
    config.write_text(text, encoding="utf-8")
    if 'Path="E:' in text:
        raise ValueError("DDC redirected to E:")
    sequence = SKY / "sky-walk.json"
    (ROOT / "inputs").mkdir(exist_ok=True)
    shutil.copyfile(sequence, ROOT / "inputs/sky-walk.json")
    save(ROOT / "clone.json", dict(source=str(SOURCE), files=records, sequence_sha256=sha(sequence),
         config_sha256=sha(config), source_content_mutated=False))


def prepare(_):
    out = ROOT / "prepare"
    out.mkdir(exist_ok=True)
    with lease("sky-prepare", 12) as (u, owner):
        u.guarded([u.EDITOR, PROJECT, *u.UE_FLAGS, *u.COMMON_INI, "-NullRHI", "-run=pythonscript",
            "-script=" + str(REPO / "scripts/ue-sky-prepare.py"), "-abslog=" + str(out / "ue.log")],
            out, timeout=int(u.remaining_seconds(owner)) - 15, env=dict(os.environ, RENDERLAB_UE_SKY_ROOT=str(ROOT)))
    if not (out / "content.json").exists():
        raise ValueError("content proof absent")


def identity():
    return dict(executable_sha256=sha(PACKAGE / "Windows/Engine/Binaries/Win64/UnrealGame.exe"),
        files={str(p.relative_to(PACKAGE)): dict(bytes=p.stat().st_size, sha256=sha(p))
               for p in PACKAGE.rglob("*") if p.is_file() and "Saved" not in p.parts})


def cook(_):
    if PACKAGE.exists():
        raise SystemExit("new package destination already exists; refusing replacement")
    out = ROOT / "cook"
    maps = ["/Game/UESky/SkyWalk", "/Game/UESky/SkyCalibration"]
    with lease("sky-cook", 45) as (u, owner):
        u.guarded([u.EDITOR, PROJECT, *u.UE_FLAGS, *u.COMMON_INI, "-NullRHI", "-run=cook", "-TargetPlatform=Windows",
            "-Map=" + "+".join(maps), "-unversioned", "-skipeditorcontent", "-cookcultures=en", "-NoZenAutoLaunch",
            "-abslog=" + str(out / "cook/ue.log")], out / "cook", timeout=int(u.remaining_seconds(owner)) - 300)
        log = (out / "cook/ue.log").read_text(encoding="utf-8-sig", errors="replace")
        if "Failed to compile Material" in log or "doesn't have a valid ShaderMap" in log:
            raise RuntimeError("cook contains a material compilation failure; refusing to package")
        u.guarded([os.environ["COMSPEC"], "/d", "/c", u.UAT, "BuildCookRun", f"-project={PROJECT}", "-nop4",
            "-skipbuild", "-skipcook", "-stage", "-pak", "-archive", f"-archivedirectory={PACKAGE}",
            "-clientconfig=Development", "-targetplatform=Win64", "-unattended", "-utf8output", "-nocompile",
            "-nocompileuat", "-installed"], out / "package", timeout=int(u.remaining_seconds(owner)) - 15)
    save(out / "identity.json", dict(identity(), maps=maps, project_config_sha256=sha(PROJECT.parent / "Config/DefaultEngine.ini")))


def capture_tool(level, *, all_parameters=True):
    """TR0's immutable camera/event protocol with only the level and dump scope adapted."""
    tr0 = load("m7-tr0")
    helper, u = host()
    tr0.ROOT, tr0.U0 = ROOT, u
    original_macro, original_settings, original_guarded = tr0.macro, tr0.settings, u.guarded
    sun = np.array([.4, .8, .3]); sun /= np.linalg.norm(sun)

    def macro(out, name, lines):
        lines = [line for line in lines if not (all_parameters and name.endswith("f002") and line.startswith("r.DumpGPU.Root"))]
        if "getall PlayerCameraManager CameraCachePrivate" in lines:
            lines += ["getall DirectionalLightComponent RelativeRotation", "getall DirectionalLightComponent LightSourceAngle"]
        return original_macro(out, name, lines)

    def settings(quality, path):
        commands = original_settings(quality, path)
        first = json.loads((ROOT / "inputs/sky-walk.json").read_text())["frames"][0]
        commands += tr0.pose_commands(dict(first, to_light=sun.tolist()))[-1:]
        commands += ["r.DumpGPU.CameraCut 0", "getall SkyLightComponent Cubemap", "getall SkyLightComponent Intensity",
                     "getall StaticMeshComponent OverrideMaterials", "r.OneFrameThreadLag 0"]
        return commands

    def guarded(command, *a, **kw):
        command_file = next((str(v).split("=", 1)[1] for v in command if str(v).startswith("-CmdLineFile=")), None)
        if command_file:
            p = Path(command_file)
            p.write_text(p.read_text().replace("/Game/M6Quarry/View0", level), encoding="utf-8")
        return original_guarded(command, *a, **kw)

    @contextlib.contextmanager
    def own_lease(label, minutes, wait_seconds=0, **kw):
        if quiet_conflicts(u):
            raise SystemExit("foreign QUIET reservation")
        u.schedule_line(f"[ue-sky request] label={label} minutes={minutes}; hidden native capture, no timing claim; {u.utc().isoformat()}")
        with u.lease(label, minutes) as owner:
            yield owner

    tr0.macro, tr0.settings, u.guarded, tr0.queued_lease = macro, settings, guarded, own_lease
    return tr0, helper


def capture(args):
    level = "/Game/UESky/SkyCalibration" if args.calibration else "/Game/UESky/SkyWalk"
    tr0, _ = capture_tool(level)
    tr0.run(argparse.Namespace(label=args.label, track="sky-walk", path=args.path, quality=args.quality,
        capture=True, smoke=False, program="full", aa="none" if args.calibration else "tsr",
        static_frame=0 if args.calibration else None, settle_frames=0, synchronous_render=True,
        temporal_readback=not args.calibration, exposure_offset=0, dump_root="PostProcessing*",
        minutes=args.minutes, wait_seconds=0, package_dir=PACKAGE))
    recipe_path = ROOT / "runs" / args.label / "recipe.json"
    recipe = json.loads(recipe_path.read_text())
    save(recipe_path, dict(recipe, level=level, ue_sky_script_sha256=sha(Path(__file__)), calibration=args.calibration))
    (recipe_path.parent / "ue-sky-source.py").write_bytes(Path(__file__).read_bytes())
    log = (recipe_path.parent / "ue.log").read_text(encoding="utf-8-sig", errors="replace")
    if any(marker in log for marker in ("Failed to compile Material", "invalid ShaderMap", "uncooked shader map")):
        raise ValueError("runtime material fallback detected")


def decode(args):
    announce_cpu("decode native HDR and per-frame exposure/jitter/cut proof: " + args.label)
    tr0, _ = capture_tool("/Game/UESky/SkyWalk")
    out = ROOT / "runs" / args.label
    recipe = json.loads((out / "recipe.json").read_text())
    tr0.summarize(argparse.Namespace(label=args.label))
    capture = json.loads((out / "capture-readback.json").read_text())
    if capture["exposure_readback_frames"] != len(recipe["frames"]):
        raise ValueError("per-frame exposure proof incomplete")
    proof = []
    for frame in capture["identities"]:
        jitter = [v for entry in frame["temporal"] for k, v in entry["values"].items() if k.endswith("InputJitter")]
        cut = [v for entry in frame["temporal"] for k, v in entry["values"].items() if k.endswith("bCameraCut")]
        indices = [v for entry in frame["temporal"] for k, v in entry["values"].items() if k.endswith("PrevFrameIndex")]
        if not recipe.get("calibration") and (not jitter or not cut or not all(np.allclose(v, jitter[0], rtol=0, atol=1e-7) for v in jitter)):
            raise ValueError("incomplete per-frame TSR temporal proof")
        proof.append(dict(frame=frame["frame"], jitter=jitter, cuts=cut, previous_indices=indices))
    if not recipe.get("calibration"):
        # TR0 resets at CSV95: one preparation sample precedes recorded CSV96.
        # Its first captured sample is Halton index 3, with bCameraCut already 0.
        if not all(all(int(v) == 0 for v in frame["cuts"]) for frame in proof):
            raise ValueError("unexpected camera cut in TR0's recorded timeline")
        if not np.allclose(proof[0]["jitter"][0], [.25, -7 / 18], rtol=0, atol=1e-6):
            raise ValueError("initial jitter differs from synchronized TR0")
        def radical_inverse(index, base):
            value, factor = 0., 1. / base
            while index:
                index, digit = divmod(index, base)
                value += digit * factor
                factor /= base
            return value
        for i, frame in enumerate(proof):
            # Native-resolution TSR uses an 11-sample prime cycle in this installed UE.
            sample = (i + 2) % 11 + 1
            expected = [radical_inverse(sample, base) - .5 for base in (2, 3)]
            if not np.allclose(frame["jitter"][0], expected, rtol=0, atol=1e-6):
                raise ValueError(f"incorrect jitter at frame {i}")
            frame["expected_jitter"] = expected
        if len({tuple(frame["jitter"][0]) for frame in proof}) < 8:
            raise ValueError("jitter sequence is not advancing")
    log = (out / "ue.log").read_text(encoding="utf-8-sig", errors="replace")
    rotations = re.findall(r"DirectionalLightComponent.*\.RelativeRotation = \(Pitch=([^,]+),Yaw=([^,]+),Roll=([^\)]+)\)", log)
    angles = [float(v) for v in re.findall(r"DirectionalLightComponent.*\.LightSourceAngle = ([0-9.eE+-]+)", log)]
    n = len(recipe["frames"])
    if len(rotations) != n or len(angles) != n + 1:
        raise ValueError("per-frame sun readback incomplete")
    wanted = np.array([.4, .8, .3]); wanted /= np.linalg.norm(wanted)
    sun_frames = []
    for i, (rotation, angle) in enumerate(zip(rotations, angles[-n:], strict=True)):
        pitch, yaw, roll = np.radians(list(map(float, rotation)))
        actual = -np.array([np.cos(pitch) * np.sin(yaw), np.sin(pitch), -np.cos(pitch) * np.cos(yaw)])
        if np.max(np.abs(actual - wanted)) > 1e-6 or abs(angle - np.degrees(.08)) > 1e-5:
            raise ValueError("sun mismatch")
        sun_frames.append(dict(frame=i, to_light=actual.tolist(), diameter_degrees=angle))
    save(out / "frame-proof.json", dict(temporal=proof, sun=sun_frames, per_frame_exposure=True,
         camera_readback_sha256=sha(out / "readback.json"), expected_sun=wanted.tolist(),
         sequence_sha256=recipe["sequence_sha256"]))
    dump = tr0.NATIVE.s1.native.Dump(out / "dump")
    grouped = {}
    for event in dump.passes:
        frame = next(int(m[1]) for scope in event["ParentEventScopes"] if (m := re.match(r"Frame (\d+)", scope)))
        grouped.setdefault(frame, []).append(event)
    frames, factors = {}, {}
    for i, (_, events) in enumerate(sorted(grouped.items())):
        consumer, name = tr0.colour_input(dump, events, recipe)
        hdr, record = tr0.NATIVE.resource(dump, name, consumer)
        if record["sha256"] != capture["identities"][i]["hdr"]["sha256"]:
            raise ValueError("changed native HDR")
        rgb = hdr[..., :3]
        if not np.array_equal(rgb.astype("f2").astype("f4"), rgb):
            raise ValueError("native HDR not lossless in float16")
        frames[f"f{i:03}"] = np.concatenate([rgb, np.ones(rgb.shape[:2] + (1,), "f4")], -1).astype("<f2")
        factors[str(i)] = 2 ** recipe["registered_exposure_stops"] / capture["identities"][i]["exposure"]["values"][0]
    dest = out / "frames"
    dest.mkdir(exist_ok=True)
    np.savez_compressed(dest / "frames.npz", **frames)
    with np.load(dest / "frames.npz") as check:
        if not all(np.array_equal(check[k], v) for k, v in frames.items()):
            raise ValueError("lossless archive round trip failed")
    save(dest / "exposure.json", dict(factors=factors, definition="registered 2^stops / measured native pre-exposure; no gain fit"))
    save(out / "native-archive.json", dict(archive_sha256=sha(dest / "frames.npz"), frames=n,
         per_frame={k: hashlib.sha256(v.tobytes()).hexdigest() for k, v in frames.items()}, lossless=True))
    print("Retained lossless native frames:", n, flush=True)


def score(args):
    announce_cpu("TR1b population score: " + args.label)
    scorer = load("ue-sky-tr1-score")
    scorer.score(argparse.Namespace(candidate=str(ROOT / "runs" / args.label / "frames"), case="w-e-n-c2" if args.label.startswith("nonrt") else "w-e-r-c1",
        out=str(ROOT / "runs" / args.label / "score.json"), track="sky-walk", sequence=str(ROOT / "inputs/sky-walk.json"),
        references=str(REFERENCES), ages=str(REFERENCES / "ages"), sky=True))


def freeze(_):
    """Pin references and baseline scores before their producer advances to a new campaign."""
    announce_cpu("freeze TR1b release references and original baseline scores by recorded hashes")
    if (ROOT / "frozen-inputs.json").exists():
        raise SystemExit("immutable reference snapshot already exists")
    original_unit = Path("D:/Coding/renderlab-work/ue-sky")
    baseline_source = original_unit / "baseline-scores"
    if not baseline_source.exists():
        baseline_source = Path("D:/Coding/renderlab-work/m7-tr1b/scores")
    accepted_path = ROOT / "runs/nonrt-tsr-epic/score.json"
    if not accepted_path.exists():
        accepted_path = baseline_source / "sky-nonrt-tr1b.json"
    accepted = json.loads(accepted_path.read_text())
    expected = {Path(row["reference"]).name: row["reference_sha256"] for row in accepted["rows"]}
    source = SKY / "release-tr1b-k" if (SKY / "release-tr1b-k").exists() else SKY
    if (original_unit / "reference-tr1b-k").exists():
        source = original_unit / "reference-tr1b-k"
    REFERENCES.mkdir(exist_ok=True)
    inventory = []
    for name, digest in expected.items():
        p = source / name
        if sha(p) != digest:
            raise ValueError("release reference differs from accepted TSR/TR1b rows")
        for f in (p, p.with_suffix(".json")):
            target = REFERENCES / f.name
            shutil.copyfile(f, target)
            if sha(target) != sha(f):
                raise ValueError("reference copy differs")
            inventory.append(dict(source=str(f), path=str(target), bytes=target.stat().st_size, sha256=sha(target)))
    shutil.copytree(source / "ages", REFERENCES / "ages", dirs_exist_ok=True)
    for p in (REFERENCES / "ages").rglob("*"):
        if p.is_file():
            original = source / p.relative_to(REFERENCES)
            if sha(p) != sha(original):
                raise ValueError("age copy differs")
            inventory.append(dict(source=str(original), path=str(p), bytes=p.stat().st_size, sha256=sha(p)))
    BASELINES.mkdir(exist_ok=True)
    for path in ("rt", "nonrt"):
        for variant in ("tr1b", "off"):
            source_score = baseline_source / f"sky-{path}-{variant}.json"
            target = BASELINES / source_score.name
            shutil.copyfile(source_score, target)
            score = json.loads(target.read_text())
            if {Path(row["reference"]).name: row["reference_sha256"] for row in score["rows"]} != expected:
                raise ValueError("TR1b baseline reference identities differ")
            inventory.append(dict(source=str(source_score), path=str(target), bytes=target.stat().st_size, sha256=sha(target)))
    save(ROOT / "frozen-inputs.json", dict(records=inventory, reference_states=len(expected),
        source_sequence_sha256=sha(ROOT / "inputs/sky-walk.json"), note="Immutable original TR1b-k release; not the producer's subsequent regeneration."))


def calibration(args):
    """Centre-sampled native no-AA sky against a CPU bilinear lookup of RLE2 level zero."""
    import struct
    from PIL import Image
    announce_cpu("no-geometry sky calibration, fixed physical scale, no gain fit")
    out = ROOT / "runs" / args.label
    transport = load("ue-quarry-environment")
    data = transport.SOURCE.read_bytes()
    if hashlib.sha256(data).hexdigest() != transport.SHA:
        raise ValueError("RLE2 changed")
    diffuse, size = struct.unpack_from("<2I", data, 64)
    cube = np.frombuffer(data, "<f4", count=6 * size * size * 3, offset=388 + diffuse * diffuse * 72).reshape(6, size, size, 3)
    pose = json.loads((ROOT / "inputs/sky-walk.json").read_text())["frames"][0]
    forward = np.array(pose["target"]) - pose["eye"]; forward /= np.linalg.norm(forward)
    right = np.cross(forward, pose["up"]); right /= np.linalg.norm(right)
    up = np.cross(right, forward)
    yy, xx = np.meshgrid(np.arange(1080) + .5, np.arange(1920) + .5, indexing="ij")
    extent = np.tan(pose["fov_y_radians"] / 2)
    direction = forward + ((xx / 1920 * 2 - 1) * extent * 1920 / 1080)[..., None] * right + ((1 - yy / 1080 * 2) * extent)[..., None] * up
    x, y, z = np.moveaxis(direction, -1, 0)
    major = np.argmax(np.abs(direction), -1)
    face = np.choose(major, [np.where(x > 0, 0, 1), np.where(y > 0, 2, 3), np.where(z > 0, 4, 5)])
    den = np.max(np.abs(direction), -1)
    u = np.choose(face, [-z, z, x, x, x, -x]) / den
    v = np.choose(face, [-y, -y, z, -z, -y, -y]) / den
    sx, sy = (u + 1) * size / 2 - .5, (v + 1) * size / 2 - .5
    ix, iy = np.floor(sx).astype(int), np.floor(sy).astype(int)
    fx, fy = (sx - ix)[..., None], (sy - iy)[..., None]
    interior = (ix >= 0) & (iy >= 0) & (ix < size - 1) & (iy < size - 1)
    def tap(tx, ty):
        values = cube[face, ty.clip(0, size - 1), tx.clip(0, size - 1)].copy()
        border = (tx < 0) | (ty < 0) | (tx >= size) | (ty >= size)
        f = face[border]
        a, b = (tx[border] + .5) * 2 / size - 1, (ty[border] + .5) * 2 / size - 1
        one = np.ones_like(a)
        dx = np.choose(f, [one, -one, a, a, a, -a])
        dy = np.choose(f, [-b, -b, one, -one, -b, -b])
        dz = np.choose(f, [-a, a, b, -b, one, -one])
        axis = np.argmax(np.abs(np.stack([dx, dy, dz], -1)), -1)
        nf = np.choose(axis, [np.where(dx > 0, 0, 1), np.where(dy > 0, 2, 3), np.where(dz > 0, 4, 5)])
        denominator = np.maximum.reduce([np.abs(dx), np.abs(dy), np.abs(dz)])
        nu = np.choose(nf, [-dz, dz, dx, dx, dx, -dx]) / denominator
        nv = np.choose(nf, [-dy, -dy, dz, -dz, -dy, -dy]) / denominator
        nx = np.rint((nu + 1) * size / 2 - .5).astype(int).clip(0, size - 1)
        ny = np.rint((nv + 1) * size / 2 - .5).astype(int).clip(0, size - 1)
        values[border] = cube[nf, ny, nx]
        return values
    expected = ((tap(ix, iy) * (1 - fx) + tap(ix + 1, iy) * fx) * (1 - fy) +
                (tap(ix, iy + 1) * (1 - fx) + tap(ix + 1, iy + 1) * fx) * fy) * 2 ** -14
    with np.load(out / "frames/frames.npz") as native:
        actual = native["f000"][..., :3].astype("f4")
    codes = load("ue-sky-tr1-score").codes
    report = dict(source_sha256=transport.SHA, native_archive_sha256=sha(out / "frames/frames.npz"),
        units="cd/m2 times 2^-14", radiance_multiplier=64, fitted_gain=False,
        border_exclusion=False, valid_pixels=1920 * 1080, populations={})
    peak = np.unravel_index(np.argmax(cube.sum(-1)), cube.shape[:3])
    disk_direction = transport.directions(size)[peak]
    disk_direction /= np.linalg.norm(disk_direction)
    report["camera_to_peak_sky_degrees"] = float(np.degrees(np.arccos(np.dot(forward, disk_direction))))
    report["sky_peak_direction"] = disk_direction.tolist()
    report["source_sky_axis"] = [.36, .48, .8]
    report["camera_to_source_sky_axis_degrees"] = float(np.degrees(np.arccos(np.dot(forward, report["source_sky_axis"]))))
    report["source_axis_definition"] = "crates/cook/src/environment_fixture.rs::sun_sky; direction fixed, no fitted axis"
    for name, mask in (("full-image", np.ones_like(interior)), ("face-interior", interior),
                       ("cube-seams", ~interior), ("sun-disk", expected.max(-1) > 1), ("dim-sky", expected.max(-1) <= 1)):
        delta = actual[mask] - expected[mask]
        dc = codes(actual, 0)[mask] - codes(expected, 0)[mask]
        report["populations"][name] = dict(n=int(mask.sum()), linear_relative_rms=float(np.sqrt(np.mean(delta ** 2) / np.mean(expected[mask] ** 2))),
            mean_actual=actual[mask].mean(0).tolist(), mean_expected=expected[mask].mean(0).tolist(),
            codes_rms=float(np.sqrt(np.mean(dc ** 2))), codes_p99=float(np.quantile(np.abs(dc), .99)), codes_max=float(np.abs(dc).max()))
    report["accepted"] = all(p["linear_relative_rms"] < .02 and p["codes_rms"] < .5 for p in report["populations"].values())
    report["acceptance"] = "<2% linear relative RMS and <0.5 F0 code RMS on each registered calibration population; no fit"
    sheet = Image.new("RGB", (1440, 270))
    for i, image in enumerate((codes(actual, 0), codes(expected, 0), np.abs(codes(actual, 0) - codes(expected, 0)) * 16)):
        sheet.paste(Image.fromarray(np.clip(image, 0, 255).astype("u1")).resize((480, 270)), (i * 480, 0))
    sheet.save(out / "calibration.png")
    save(out / "calibration.json", report)
    print(json.dumps(report["populations"], indent=2), flush=True)
    if not report["accepted"]:
        raise ValueError("no-geometry map calibration failed")


def cleanup(_):
    """Remove only this cook's derived/build/staging output after native verification."""
    announce_cpu("remove verified cook's DDC, intermediate, cooked and duplicate staged/source content")
    accepted = list((ROOT / "runs").glob("*/calibration.json"))
    if not accepted or not (ROOT / "cook/identity.json").exists():
        raise ValueError("package identity and numeric calibration required")
    if not any(json.loads(p.read_text()).get("accepted") is True for p in accepted):
        raise ValueError("no accepted calibration")
    original = json.loads((ROOT / "cook/identity.json").read_text())
    if identity()["files"] != original["files"]:
        raise ValueError("packaged files changed")
    cloned = json.loads((ROOT / "clone.json").read_text())["files"]
    for entry in cloned:
        if sha(SOURCE / entry["path"]) != entry["sha256"]:
            raise ValueError("original source changed since copy: " + entry["path"])
    removed = []
    for name in ("DerivedDataCache", "Intermediate", "Saved", "Build", "Content"):
        p = (PROJECT.parent / name).resolve()
        if p.parent != PROJECT.parent.resolve() or ROOT.resolve() not in p.parents:
            raise ValueError("cleanup path escaped owned project")
        if p.exists():
            files = [f for f in p.rglob("*") if f.is_file()]
            record = dict(path=str(p), files=len(files), bytes=sum(f.stat().st_size for f in files))
            shutil.rmtree(p)
            removed.append(record)
    rejected = (ROOT / "rejected/package-a").resolve()
    if rejected.exists():
        if rejected.parent != (ROOT / "rejected").resolve() or ROOT.resolve() not in rejected.parents:
            raise ValueError("rejected-package cleanup escaped owned output")
        files = [p for p in rejected.rglob("*") if p.is_file()]
        removed.append(dict(path=str(rejected), files=len(files), bytes=sum(p.stat().st_size for p in files),
            identity=str(ROOT / "rejected/cook-a/identity.json"), reason="rejected black-sky package; logs and identity retained"))
        shutil.rmtree(rejected)
    save(ROOT / "cook/cleanup.json", dict(removed=removed, retained_package_verified=True,
         source_content=str(SOURCE / "Content"), original_source_files_unchanged=len(cloned), rebuilt_from_scripts=True))


def prune(args):
    announce_cpu("hash and delete intermediate GPU dumps: " + args.label)
    out = (ROOT / "runs" / args.label).resolve()
    if ROOT.resolve() not in out.parents or not (out / "native-archive.json").exists():
        raise ValueError("owned, verified native archive required")
    removed = []
    for p in sorted((out / "dump").rglob("*")):
        if p.is_file() and p.stat().st_size > 5_000_000:
            removed.append(dict(path=str(p), bytes=p.stat().st_size, sha256=sha(p)))
            p.unlink()
    save(out / "deleted-raw.json", removed)
    print("Removed intermediate bytes:", sum(p["bytes"] for p in removed), flush=True)


def timing(args):
    helper, u = host()
    # Acquire fairly between functional slots. The continuous compiler-free
    # minute is measured inside the lease, before launching any GPU child.
    # The inherited queued wrapper otherwise restarts that minute every time
    # another worker wins a lock race after a long wait.
    u.WAIT_FOR_FOREIGN_CPU_CLEAR = False
    if helper.game_state() != "released":
        raise SystemExit("quiet cost curve waits for GAME HOLD RELEASED")
    if quiet_conflicts(u):
        raise SystemExit("foreign QUIET window active")
    root = ROOT / "timing" / args.label
    root.mkdir(parents=True, exist_ok=False)
    (root / "measurement-source.py").write_bytes(Path(__file__).read_bytes())
    visits, done, inherited = [], set(), None
    if args.resume:
        prior = ROOT / "timing" / args.resume / "certificate.json"
        inherited = dict(path=str(prior), sha256=sha(prior))
        failures = [json.loads(p.read_text()) for p in (ROOT / "runs").glob(args.resume + "-*/process.json")
                    if json.loads(p.read_text()).get("failures")]
        cutoff = min((p["start_utc"] for p in failures), default="9999")
        for label in json.loads(prior.read_text())["visits"]:
            out = ROOT / "runs" / label
            process = json.loads((out / "process.json").read_text())
            if process["exit_code"] or process["failures"] or process["end_utc"] >= cutoff:
                raise ValueError("cannot reuse a failed or overlapping timing process")
            recipe = json.loads((out / "recipe.json").read_text())
            key = (int(label.rsplit("-", 1)[1]), recipe["path"], recipe["quality"])
            if key in done:
                raise ValueError("duplicate inherited visit")
            done.add(key); visits.append(label)
        inherited["qualified_before_rejected_process_start"] = cutoff
        print("Reusing independently guarded clean visits:", len(visits), flush=True)
    with u.lease("sky-quiet-" + args.label, 8) as owner:
        u.WAIT_FOR_FOREIGN_CPU_CLEAR = True
        u.schedule_line(f"[ue-sky QUIET active] owner=ue-sky label=sky-quiet-{args.label} end={(u.utc() + dt.timedelta(minutes=8)).isoformat()}; no foreign GPU/compiler/heavy CPU work until owned release.")
        complete = False
        try:
            clear_since, samples = None, []
            while True:
                if helper.game_state() != "released" or u.remaining_seconds(owner) < 150:
                    raise RuntimeError("quiet readiness ended before the matrix could start")
                processes = [p for p in helper.native_process_inventory() if p["Name"].lower() in
                    ("cargo.exe", "rustc.exe", "clippy-driver.exe", "link.exe", "unrealbuildtool.exe")]
                available = u.E2E.available_gib()
                now = time.monotonic()
                if processes or available < 4:
                    clear_since = None
                elif clear_since is None:
                    clear_since = now
                samples.append(dict(utc=u.utc().isoformat(), compilers=processes, available_gib=available,
                    clear_seconds=0 if clear_since is None else now - clear_since))
                if clear_since is not None and now - clear_since >= 60:
                    save(root / "compiler-free.json", dict(samples=samples, required_seconds=60, before_gpu_launch=True))
                    break
                time.sleep(1)
            with helper.quiet_desktop(u, root) as quiet:
                for visit in range(4):
                    configs = [(p, q) for p in ("vsm-nort", "hrt") for q in (3, 2)]
                    for path, quality in configs if visit % 2 == 0 else reversed(configs):
                        if (visit, path, quality) in done:
                            continue
                        if u.remaining_seconds(owner) < 45:
                            raise RuntimeError("quiet lease has insufficient time for another bounded visit")
                        label = f"{args.label}-{path}-q{quality}-{visit}"
                        tr0, _ = capture_tool("/Game/UESky/SkyWalk", all_parameters=False)
                        # Use the outer guarded host so its deadline bounds every child.
                        wrapped = tr0.U0.guarded
                        tr0.U0.guarded = lambda command, *a, **kw: wrapped(command, *a, **dict(kw, timeout=min(kw.get("timeout", 120), int(u.remaining_seconds(owner)) - 15)))
                        tr0.queued_lease = lambda *a, **kw: contextlib.nullcontext(owner)
                        tr0.U0.WAIT_FOR_FOREIGN_CPU_CLEAR = True
                        tr0.run(argparse.Namespace(label=label, track="sky-walk", path=path, quality=quality,
                            capture=False, smoke=False, program="full", minutes=3, wait_seconds=0, package_dir=PACKAGE))
                        out = ROOT / "runs" / label
                        save(out / "activity-policy.json", dict(state="quiet", speed_claim_eligible=True, quiet_window=str(quiet), one_frame_thread_lag=0))
                        tr0.summarize(argparse.Namespace(label=label))
                        visits.append(label)
                        save(root / "visits.json", visits)
                complete = True
        finally:
            save(root / "certificate.json", dict(complete=complete, lease=owner, visits=visits if 'visits' in locals() else [],
                 inherited=inherited, one_frame_thread_lag=0, repeats="4 processes x 2 no-AA/TSR pairs per path/quality", instrumented_capture=False))
            u.schedule_line(f"[ue-sky QUIET completed] label=sky-quiet-{args.label} complete={complete}; {u.utc().isoformat()}")


def report(_):
    announce_cpu("pool all TSR/TR1b populations and export comparison tables")
    import csv
    scorer = load("ue-sky-tr1-score")
    segments = {"all": set(range(40)), "hold-start": set(range(8)), "moving": set(range(8, 32)), "hold-end": set(range(32, 40))}
    pooled, identities, population_proof = {}, {}, {}
    for path in ("rt", "nonrt"):
        pooled[path] = {}
        expected_counts = None
        for variant in ("off", "tr1b", "tsr-epic", "tsr-high"):
            source = (BASELINES / f"sky-{path}-{variant}.json" if variant in ("off", "tr1b")
                      else ROOT / "runs" / f"{path}-{variant}" / "score.json")
            score = json.loads(source.read_text())
            if len(score["rows"]) != 40 or score.get("missing_reference_frames"):
                raise ValueError("incomplete score: " + str(source))
            counts = [dict(frame=row["frame"], reference_sha256=row["reference_sha256"],
                populations={channel: {name: v["n"] for name, v in pops.items()} for channel, pops in row["measures"].items()})
                for row in score["rows"]]
            if expected_counts is None:
                expected_counts = counts
            elif counts != expected_counts:
                raise ValueError("reference identities or population counts differ: " + str(source))
            population_proof[f"{path}/{variant}"] = hashlib.sha256(json.dumps(counts, sort_keys=True).encode()).hexdigest()
            pooled[path][variant] = scorer.pooled_rows(score, segments)
            identities[f"{path}/{variant}"] = dict(path=str(source), sha256=sha(source))
    out = ROOT / "comparison"
    out.mkdir(exist_ok=True)
    records, verdicts = [], []
    for path, variants in pooled.items():
        baseline = variants["tr1b"]
        for segment, base in baseline.items():
            for key in sorted(base):
                record = dict(path=path, segment=segment, population=key[0], channel=key[1], measure=key[2])
                for name, variant in variants.items():
                    if key not in variant[segment]:
                        raise ValueError("population mismatch")
                    record[name] = variant[segment][key]
                records.append(record)
            for comparator in ("tsr-epic", "tsr-high"):
                other = variants[comparator][segment]
                if base.keys() != other.keys():
                    raise ValueError("row set mismatch")
                better = sum(v < other[k] for k, v in base.items())
                worse = sum(v > other[k] for k, v in base.items())
                verdicts.append(dict(path=path, segment=segment, comparator=comparator, rows=len(base),
                    tr1b_better=better, tr1b_worse=worse, tied=len(base) - better - worse))
    with (out / "all-rows.csv").open("w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=list(records[0])); writer.writeheader(); writer.writerows(records)
    save(out / "summary.json", dict(scores=identities, verdicts=verdicts, scorer_sha256=sha(Path(scorer.__file__)),
        pooled_rows_sha256=sha(out / "all-rows.csv"), same_populations=True, population_proof=population_proof,
        segments={k: sorted(v) for k, v in segments.items()}))
    print(json.dumps(verdicts, indent=2), flush=True)


def inspect(args):
    from PIL import Image, ImageDraw, ImageFont
    announce_cpu("full-image and native-size silhouette comparison sheets: " + args.label)
    path = args.label
    if path not in ("rt", "nonrt"):
        raise ValueError("inspection path must be rt or nonrt")
    scorer = load("ue-sky-tr1-score")
    frames = json.loads((ROOT / "inputs/sky-walk.json").read_text())["frames"]
    cases = (0, 7, 8, 16, 31, 39)
    out = ROOT / "inspection"
    out.mkdir(exist_ok=True)
    refs = {}
    for p in REFERENCES.glob("sky-walk-*.json"):
        meta = json.loads(p.read_text())
        if "frame" in meta:
            key = np.asarray(meta["frame"]["open"]["eye"] + meta["frame"]["open"]["target"], "<f4").tobytes()
            refs[key] = p
    font = ImageFont.truetype("C:/Windows/Fonts/segoeui.ttf", 16)
    with np.load(ROOT / "runs" / (path + "-tsr-epic") / "frames/frames.npz") as epic, \
         np.load(ROOT / "runs" / (path + "-tsr-high") / "frames/frames.npz") as high:
        for stops in (0, 4):
            sheet = Image.new("RGB", (1920, 5 * 204))
            crop = Image.new("RGB", (3 * 512, 4 * 280))
            for column, i in enumerate(cases):
                p = frames[i]
                key = np.asarray(p["eye"] + p["target"], "<f4").tobytes()
                ref_file = refs[key]
                ref, _, scale = scorer.load_reference(ref_file.with_suffix(".npz"), json.loads(ref_file.read_text()))
                reference = (ref["L0"] + ref["L1"]) * (.5 * scale)
                values = [scorer.codes(v, stops) for v in (reference, epic[f"f{i:03}"][..., :3].astype("f4"), high[f"f{i:03}"][..., :3].astype("f4"))]
                tiles = values + [abs(values[1] - values[0]) * 4, abs(values[2] - values[0]) * 4]
                labels = ("reference", "TSR Epic", "TSR High", "TSR Epic error x4", "TSR High error x4")
                for row, (v, name) in enumerate(zip(tiles, labels, strict=True)):
                    tile = Image.fromarray(v.clip(0, 255).astype("u1")).resize((320, 180), Image.Resampling.LANCZOS)
                    sheet.paste(tile, (column * 320, row * 204 + 24))
                    ImageDraw.Draw(sheet).text((column * 320 + 4, row * 204 + 2), f"{name}, f{i}, L{stops}", font=font, fill="white")
                if i in (8, 16, 31, 39):
                    row = (8, 16, 31, 39).index(i)
                    for col, (v, name) in enumerate(zip(values, labels[:3], strict=True)):
                        tile = Image.fromarray(v[440:696, 544:1056].clip(0, 255).astype("u1"))
                        crop.paste(tile, (col * 512, row * 280 + 24))
                        ImageDraw.Draw(crop).text((col * 512 + 4, row * 280 + 2), f"{name}, f{i}, L{stops}", font=font, fill="white")
            sheet.save(out / f"{path}-L{stops}-full.png")
            crop.save(out / f"{path}-L{stops}-silhouette.png")
    save(out / (path + "-images.json"), {p.name: sha(p) for p in out.glob(path + "-*.png")})


def costs(args):
    import csv
    announce_cpu("aggregate eight quiet A/B repeats per path and quality")
    root = ROOT / "timing" / args.label
    certificate = json.loads((root / "certificate.json").read_text())
    if not certificate["complete"]:
        raise ValueError("quiet timing campaign incomplete")
    groups = {}
    for label in certificate["visits"]:
        out = ROOT / "runs" / label
        recipe = json.loads((out / "recipe.json").read_text())
        proof = json.loads((out / "readback.json").read_text())
        if not proof["ok"] or not proof["d3d12_sm6"] or not proof["sun_angle"]["ok"]:
            raise ValueError("invalid cost readback")
        actual = load("m7-u0-ue").cvar_rows(out / "cvars.csv")
        if float(actual["r.OneFrameThreadLag"]["value"]) != 0:
            raise ValueError("cost thread-lag mismatch")
        process = json.loads((out / "process.json").read_text())
        if process["exit_code"] != 0 or process["failures"]:
            raise ValueError("invalid cost process")
        rows = json.loads((out / "timing.json").read_text())["per_frame"]
        if len(rows) != 64 or any(r["tsr_pass_ms"] <= 0 or not all(np.isfinite(r[k]) for k in
                ("no_aa_gpu_ms", "tsr_gpu_ms", "tsr_pass_ms")) for r in rows):
            raise ValueError("missing native TSR cost samples")
        with (out / "profile.csv").open(encoding="utf-8-sig") as stream:
            csv_rows = list(csv.reader(stream))
        column = csv_rows[0].index("GPU/TemporalSuperResolution")
        for repeat in (0, 2):
            for i in range(8, len(recipe["frames"])):
                frame = recipe["start"] + repeat * (len(recipe["frames"]) + recipe["gap"]) + i
                if float(csv_rows[frame + 1][column] or 0) != 0:
                    raise ValueError("no-AA timing phase has a live TSR scope")
        groups.setdefault((recipe["path"], recipe["quality"]), []).extend(rows)
    result = []
    for (path, quality), rows in groups.items():
        stats = {}
        for name, values in (("no_aa_total", [r["no_aa_gpu_ms"] for r in rows]),
                             ("tsr_total", [r["tsr_gpu_ms"] for r in rows]),
                             ("tsr_pass", [r["tsr_pass_ms"] for r in rows]),
                             ("paired_delta", [r["tsr_gpu_ms"] - r["no_aa_gpu_ms"] for r in rows])):
            stats[name] = dict(mean=float(np.mean(values)), p50=float(np.quantile(values, .5)),
                p95=float(np.quantile(values, .95)), p99=float(np.quantile(values, .99)))
        if len(rows) != 256:
            raise ValueError("eight paired repeats required")
        result.append(dict(path=path, quality=quality, samples=len(rows), stats=stats))
    if len(result) != 4:
        raise ValueError("both paths and quality levels required")
    save(root / "costs.json", dict(rows=result, native_resolution=[1920, 1080], one_frame_thread_lag=0,
        excluded_frames=list(range(8)), claim="TSR costs on this sky walk only; no Renderlab speed or quality acceptance"))
    print(json.dumps(result, indent=2), flush=True)


def main():
    global ROOT, PACKAGE, PROJECT, REFERENCES, BASELINES
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT, help="fresh evidence/project root on D: for rebuilding without replacing this campaign")
    parser.add_argument("--package", type=Path, default=PACKAGE, help="fresh package destination on D:; existing packages are never overwritten")
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("clone", "prepare", "cook", "cleanup", "report", "freeze"):
        commands.add_parser(name)
    p = commands.add_parser("capture")
    p.add_argument("label")
    p.add_argument("--path", choices=("vsm-nort", "hrt"), default="vsm-nort")
    p.add_argument("--quality", type=int, choices=(2, 3), default=3)
    p.add_argument("--minutes", type=int, default=12)
    p.add_argument("--calibration", action="store_true")
    for name in ("decode", "score", "calibration", "prune", "inspect", "costs"):
        commands.add_parser(name).add_argument("label")
    p = commands.add_parser("timing")
    p.add_argument("label")
    p.add_argument("--resume", help="reuse independently guarded successful visits before the earlier window's failed process")
    args = parser.parse_args()
    ROOT, PACKAGE = args.root.resolve(), args.package.resolve()
    bulk = Path("D:/Coding/renderlab-work").resolve()
    if bulk not in ROOT.parents or bulk not in PACKAGE.parents or ROOT == PACKAGE or PACKAGE in ROOT.parents:
        raise ValueError("use distinct owned roots beneath D:/Coding/renderlab-work")
    PROJECT = ROOT / "ue-project/M5Calibration.uproject"
    REFERENCES, BASELINES = ROOT / "reference-tr1b-k", ROOT / "baseline-scores"
    if PACKAGE == PROJECT.parent or PROJECT.parent in PACKAGE.parents:
        raise ValueError("package must be outside the disposable project tree")
    ROOT.mkdir(parents=True, exist_ok=True)
    dict(clone=clone, prepare=prepare, cook=cook, capture=capture, decode=decode, score=score,
         calibration=calibration, prune=prune, timing=timing, cleanup=cleanup, report=report,
         inspect=inspect, costs=costs, freeze=freeze)[args.command](args)


if __name__ == "__main__":
    main()
