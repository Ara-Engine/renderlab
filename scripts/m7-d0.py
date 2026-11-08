"""D0: bounded, serial CPU replay. Never creates a GPU device or changes renderer code.

All spatial passes consume current-frame geometry and radiance only. Temporal
accumulation, where a sequence exists, follows spatial filtering and exposes no
auxiliary information to it. Saved final images are explicitly diagnostic inputs,
not an implementation of the incident-radiance API.
"""
import argparse
import contextlib
import ctypes
from ctypes import wintypes
import datetime as dt
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import time

os.environ.update(OPENBLAS_NUM_THREADS="1", OMP_NUM_THREADS="1", MKL_NUM_THREADS="1")
REPO = Path(__file__).resolve().parent.parent
OUT = Path("E:/renderlab-work/m7-d0")
SCHEDULE = OUT.parent / "gpu-schedule.md"
CPU_LOCK = OUT.parent / "heavy-cpu.lock"
CASES = ("RF1-pitch30", "RF2-sweep", "RF4-pitch30", "RF8-pitch30")
CANDIDATES = ("none", "bilateral", "atrous", "hit-radius")


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


def note(action):
    with SCHEDULE.open("a", encoding="utf-8") as stream:
        stream.write(f"\n[m7-d0 CPU-only] {dt.datetime.now(dt.timezone.utc).isoformat()} owner=m7-d0 {action}; one worker, <1.5 GiB, no GPU/window.\n")


class MemoryStatus(ctypes.Structure):
    _fields_ = [("length", wintypes.DWORD), ("load", wintypes.DWORD)] + [
        (name, ctypes.c_ulonglong) for name in
        ("total", "available", "page_total", "page_available", "virtual_total", "virtual_available", "extended")
    ]


class ProcessMemory(ctypes.Structure):
    _fields_ = [("cb", wintypes.DWORD), ("faults", wintypes.DWORD)] + [
        (name, ctypes.c_size_t) for name in
        ("peak_rss", "rss", "peak_paged", "paged", "peak_nonpaged", "nonpaged", "pagefile", "peak_pagefile", "private")
    ]


def memory():
    state = MemoryStatus()
    state.length = ctypes.sizeof(state)
    if not ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(state)):
        raise OSError("GlobalMemoryStatusEx failed")
    process = ProcessMemory()
    process.cb = ctypes.sizeof(process)
    ctypes.windll.kernel32.GetCurrentProcess.restype = wintypes.HANDLE
    ctypes.windll.psapi.GetProcessMemoryInfo.argtypes = [wintypes.HANDLE, ctypes.POINTER(ProcessMemory), wintypes.DWORD]
    handle = ctypes.windll.kernel32.GetCurrentProcess()
    if not ctypes.windll.psapi.GetProcessMemoryInfo(handle, ctypes.byref(process), process.cb):
        raise OSError("GetProcessMemoryInfo failed")
    return dict(available_gib=state.available / 2**30, rss_mib=process.rss / 2**20,
                peak_rss_mib=process.peak_rss / 2**20, private_mib=process.private / 2**20)


RESOURCE = []


def guard():
    sample = memory()
    RESOURCE.append(sample)
    if max(sample["rss_mib"], sample["private_mib"]) >= 1536:
        raise RuntimeError("D0 1.5 GiB process memory cap reached")
    if sample["available_gib"] < 2:
        raise RuntimeError("D0 runtime host memory emergency floor reached")
    return sample


@contextlib.contextmanager
def cpu_job(label):
    OUT.mkdir(parents=True, exist_ok=True)
    sample = memory()
    if sample["available_gib"] < 8:
        save(OUT / "deferred.json", dict(reason="less than 8 GiB available at start", memory=sample))
        raise SystemExit("DEFERRED: CPU replay requires >=8 GiB available")
    # Exclusive directory creation coordinates every D0 invocation without a stale-lock bypass.
    CPU_LOCK.mkdir()
    start = time.perf_counter()
    save(CPU_LOCK / "owner.json", dict(owner="m7-d0", pid=os.getpid(), label=label,
         utc=dt.datetime.now(dt.timezone.utc).isoformat(), memory=sample))
    note(f"action={label} started available={sample['available_gib']:.3f} GiB lock={CPU_LOCK}")
    try:
        yield
    finally:
        try:
            RESOURCE.append(memory())
            save(OUT / f"resources-{label}.json", dict(seconds=time.perf_counter()-start, start=sample,
                 peak_rss_mib=max(x["peak_rss_mib"] for x in RESOURCE),
                 max_private_mib=max(x["private_mib"] for x in RESOURCE), samples=RESOURCE))
        finally:
            (CPU_LOCK / "owner.json").unlink()
            CPU_LOCK.rmdir()
            note(f"action={label} finished; heavy CPU lock released")


def camera(scene, pose, shape):
    import numpy as np
    eye = np.asarray(pose["eye"], dtype="f4")
    f = np.asarray(pose["target"], dtype="f4") - eye
    f /= np.linalg.norm(f)
    right = np.cross(f, np.array([0, 1, 0], dtype="f4"))
    right /= np.linalg.norm(right)
    up = np.cross(right, f)
    h, w = shape
    tan = math.tan(math.radians(scene["fov_y_degrees"]) / 2)
    x = ((np.arange(w, dtype="f4") + .5) / w * 2 - 1) * tan * w/h
    y = (1 - (np.arange(h, dtype="f4") + .5) / h * 2) * tan
    direction = f + x[None, :, None]*right + y[:, None, None]*up
    direction /= np.linalg.norm(direction, axis=-1, keepdims=True)
    return eye, f, direction


def reference_guides(ref, scene, pose):
    """Recover the exact centre-ray plane intersection from saved primitive IDs.

    R0a retains material/primitive identity and geometric normal, but not primary
    world positions. Secondary world is the ideal mirror ray, not a K-sample hit.
    """
    import numpy as np
    ids, n = ref["hit"], ref["normal"]
    valid = ids[..., 0] != 0
    eye, forward, direction = camera(scene, pose, valid.shape)
    distance = np.zeros(valid.shape, dtype="f4")
    for i, mesh in enumerate(scene["meshes"]):
        mask = ids[..., 0] == i+1
        if not mask.any():
            continue
        vertices = np.asarray(mesh["positions"], dtype="f4")
        triangles = np.asarray(mesh["indices"], dtype="u4").reshape(-1, 3)
        a = vertices[triangles[ids[..., 1][mask]-1, 0]]
        distance[mask] = np.sum((a-eye)*n[mask], axis=-1) / np.sum(direction[mask]*n[mask], axis=-1)
    world = eye + direction * distance[..., None]
    depth = np.sum((world-eye)*forward, axis=-1)
    secondary = ref["reflection_world"]
    secondary_valid = ref["reflection_id"] != 0
    hit_t = np.where(secondary_valid, np.linalg.norm(secondary-world, axis=-1), 0).astype("f4")
    return dict(world=world, normal=n, material=ids[..., 2], instance=ids[..., 0],
                depth=depth, valid=valid, hit_t=hit_t, secondary_valid=secondary_valid)


def paired_slices(shape, dy, dx):
    h, w = shape
    a = (slice(max(0, -dy), min(h, h-dy)), slice(max(0, -dx), min(w, w-dx)))
    b = (slice(max(0, dy), min(h, h+dy)), slice(max(0, dx), min(w, w+dx)))
    return a, b


def spatial(image, guides, kind):
    """No colour, history, age, moments or rejection-mask weights are read here."""
    import numpy as np
    if kind == "none":
        return image.copy()
    steps = (1, 2, 4) if kind == "atrous" else (1,)
    result = image.copy()
    n, world = guides["normal"], guides["world"]
    for step in steps:
        guard()
        total = np.zeros_like(result)
        norm = np.zeros(image.shape[:2], dtype="f4")
        radius = 4 if kind == "hit-radius" else 2
        adaptive = np.clip(np.floor(1 + guides["hit_t"] / np.maximum(guides["depth"], .001)), 1, 4)
        for oy in range(-radius, radius+1):
            for ox in range(-radius, radius+1):
                a, b = paired_slices(image.shape[:2], oy*step, ox*step)
                same = ((guides["instance"][a] == guides["instance"][b]) &
                        (guides["material"][a] == guides["material"][b]) & guides["valid"][a] & guides["valid"][b])
                ndot = np.clip(np.sum(n[a]*n[b], axis=-1), 0, 1)
                plane = np.abs(np.sum((world[b]-world[a])*n[a], axis=-1))
                sigma = .002*np.maximum(guides["depth"][a], .1) + .0005
                weight = same * ndot**32 * np.exp(-plane/sigma)
                if kind == "atrous":
                    weight *= (1, 4, 6, 4, 1)[oy+2] * (1, 4, 6, 4, 1)[ox+2]
                    if "hit_t" in guides:
                        weight *= guides["secondary_valid"][a] == guides["secondary_valid"][b]
                        weight *= np.exp(-np.abs(np.log1p(guides["hit_t"][a])-np.log1p(guides["hit_t"][b]))/.2)
                elif kind == "hit-radius":
                    weight *= np.maximum(abs(oy), abs(ox)) <= adaptive[a]
                    weight *= np.exp(-(ox*ox+oy*oy)/(2*np.maximum(adaptive[a], 1)**2))
                else:
                    weight *= math.exp(-(ox*ox+oy*oy)/2)
                total[a] += result[b]*weight[..., None]
                norm[a] += weight
        filtered = total/np.maximum(norm[..., None], 1e-20)
        result = np.where(guides["valid"][..., None], filtered, result)
    return result


def joint_conservative(samples, directions):
    """Same-pixel, symmetric angular exchanges preserve equal-weight quadrature.

    This is a deliberately conservative direction-aware control, not cross-pixel
    sample reuse. Missing sample records cannot be recovered from an RGB sum.
    """
    import numpy as np
    output = samples.copy()
    k = len(samples)
    for a in range(k):
        for b in range(a+1, k):
            weight = np.maximum(0, np.sum(directions[a]*directions[b], axis=-1))**32 / k
            delta = (samples[b]-samples[a])*weight[..., None]
            output[a] += delta
            output[b] -= delta
    return output


def accumulate(previous, current, age, reset=False):
    # Age is sequence-wide engine state; no per-pixel temporal auxiliary output.
    return current.copy() if reset or age == 1 else previous + (current-previous)/min(age, 8)


def scores(image, h0, h1, populations, r0a):
    """R0a's exact metrics, with shared full-frame transforms computed once."""
    import numpy as np
    reference = (h0+h1)/2
    codes = r0a.load("m7-u0b-score").codes
    errors = {s:codes(image,s)-codes(reference,s) for s in (-14,-10)}
    noises = {s:(codes(h0,s)-codes(h1,s))/2 for s in (-14,-10)}
    result = {}
    lum = np.array([.2126,.7152,.0722])
    for name, mask in populations.items():
        if not mask.any():
            result[name] = dict(pixels=0, status="no visible primary pixels in registered population")
            continue
        c, r = image[mask], reference[mask]
        scale = max(1e-20, float(np.mean(r.astype("f8")**2)))
        signed = float(np.sum(c-r,dtype="f8")/max(1e-20,float(np.sum(r,dtype="f8"))))
        row = dict(pixels=int(mask.sum()),
             rgb_relative_rms=float(np.sqrt(np.mean((c-r).astype("f8")**2)/scale)),
             luminance_relative_l1=float(np.abs(c@lum-r@lum).sum()/max(1e-20,np.abs(r@lum).sum())),
             signed_energy_bias=signed,absolute_energy_bias=abs(signed),
             split_noise_relative_rms=float(np.sqrt(np.mean(((h0[mask]-h1[mask])/2).astype("f8")**2)/scale)))
        for s in (-14,-10):
            e = errors[s][mask]
            row[f"display_rms_{s}"] = float(np.sqrt(np.mean(e**2)))
            row[f"p99_absolute_display_error_{s}"] = float(np.quantile(np.max(np.abs(e),axis=-1),.99))
            row[f"reference_split_display_noise_rms_{s}"] = float(np.sqrt(np.mean(noises[s][mask]**2)))
        result[name] = row
    return result


def compare(measures, features, floor):
    keys = ("rgb_relative_rms", "luminance_relative_l1", "absolute_energy_bias", "display_rms_-14",
            "p99_absolute_display_error_-14", "display_rms_-10", "p99_absolute_display_error_-10")
    failures, passed = [], 0
    for pop, values in floor["measures"].items():
        for key in keys:
            if key in values and key in measures.get(pop, {}):
                success = measures[pop][key] <= values[key]
                passed += int(success)
                if not success:
                    failures.append(pop+"/"+key)
    feature_failures, feature_passed = [], 0
    for name, values in floor["features"].items():
        for key in ("footprint_energy_ratio", "chromatic_energy_ratio", "centroid_error_pixels"):
            current = features.get(name, {}).get(key)
            baseline = values.get(key)
            if current is not None and baseline is not None:
                error = (lambda x: abs(x-1)) if key.endswith("ratio") else abs
                success = error(current) <= error(baseline)
                feature_passed += int(success)
                if not success:
                    feature_failures.append(name+"/"+key)
        for key in ("candidate_missing_edges", "median_extra_pixels", "p95_extra_pixels"):
            current = features.get(name, {}).get("edge_spread", {}).get(key)
            baseline = values.get("edge_spread", {}).get(key)
            if current is not None and baseline is not None:
                success = abs(current) <= abs(baseline)
                feature_passed += int(success)
                if not success:
                    feature_failures.append(name+"/edge_spread/"+key)
    return dict(passed=passed, tested=passed+len(failures), failures=failures,
                features_passed=feature_passed, features_tested=feature_passed+len(feature_failures),
                feature_failures=feature_failures, passes=not failures and not feature_failures)


def replay_reflections(cases):
    import numpy as np
    from PIL import Image, ImageDraw
    r0a = load("m7-r0a")
    manifest_path = OUT.parent / "m7-r0a/reference/final/manifest.json"
    floor_path = OUT.parent / "m7-r0a/scores/c-static.json"
    manifest, floors = json.loads(manifest_path.read_text()), json.loads(floor_path.read_text())["rows"]
    rows, inputs = [], {str(manifest_path): sha(manifest_path), str(floor_path): sha(floor_path)}
    for case in cases:
        fixture, pose_name = case.split("-", 1)
        scene_path = OUT.parent / f"m7-r0a/scenes/{fixture}/scene.json"
        scene = json.loads(scene_path.read_text())
        archive = OUT.parent / "m7-r0a/reference/final" / manifest["cases"][case]["archive"]
        inputs[str(archive)], inputs[str(scene_path)] = sha(archive), sha(scene_path)
        assert inputs[str(archive)] == manifest["cases"][case]["archive_sha256"]
        assert inputs[str(scene_path)] == manifest["cases"][case]["scene_sha256"]
        with np.load(archive) as ref:
            h0, h1 = ref["L0"], ref["L1"]
            populations = {k[4:]: ref[k] for k in ref.files if k.startswith("pop_")}
            populations["complete-frame"] = np.ones(h0.shape[:2], dtype=bool)
            guides = reference_guides(ref, scene, next(p for p in scene["poses"] if p["name"] == pose_name))
        reference = (h0+h1)/2
        images = []
        for k in (4, 8):
            path = OUT.parent / f"m7-r0b/runs/final/{case}-k{k}.npz"
            inputs[str(path)] = sha(path)
            with np.load(path) as data:
                raw = data["L"]
            for candidate in CANDIDATES:
                start = time.perf_counter()
                image = spatial(raw, guides, candidate)
                measures = scores(image, h0, h1, populations, r0a)
                features = r0a.feature_measures(image, reference, populations)
                comparisons = {}
                for config in ("hw-hit-epic", "hw-sc-epic", "sw-mesh-epic"):
                    floor = next(x for x in floors if x["fixture"] == fixture and x["pose"] == pose_name and x["config"] == config)
                    assert floor["reference_archive_sha256"] == inputs[str(archive)]
                    comparisons[config] = compare(measures, features, floor)
                row = dict(case=case, k=k, candidate=candidate, measures=measures, features=features,
                           comparisons=comparisons, cpu_seconds=time.perf_counter()-start,
                           image_sha256=hashlib.sha256(image.tobytes()).hexdigest(),
                           static_history_frame8_identical=True,
                           scope="saved final radiance; ideal-centre secondary distance proxy; RT static diagnostic")
                rows.append(row)
                save(OUT / f"scores/{case}-k{k}-{candidate}.json", row)
                if k == 8:
                    image_codes = np.clip(r0a.load("m7-u0b-score").codes(image, -14), 0, 255).astype("u1")
                    images.append((candidate, image_codes))
                print(case, k, candidate, comparisons["hw-hit-epic"]["passed"], "/", comparisons["hw-hit-epic"]["tested"], flush=True)
                del image
        # Reduced sheet is for inspection only; scores above include every native pixel.
        panels = [("reference", np.clip(r0a.load("m7-u0b-score").codes(reference, -14), 0, 255).astype("u1"))] + images
        sheet = Image.new("RGB", (960, 300*math.ceil(len(panels)/2)), "#111820")
        draw = ImageDraw.Draw(sheet)
        for i, (name, value) in enumerate(panels):
            x, y = i % 2 * 480, i // 2 * 300
            draw.text((x+4, y+4), case+" "+name, fill="white")
            sheet.paste(Image.fromarray(value).resize((480, 270)), (x, y+22))
        sheet.save(OUT / f"{case}-inspection.png")
        del h0, h1, populations, guides, reference, raw, images, panels
        guard()
    save(OUT / "reflections.json", dict(rows=rows, inputs=inputs, script_sha256=sha(Path(__file__)),
         missing=["individual K directional samples", "K16/K32", "RF3/RF5 motion", "non-RT reflections"]))


def selfcheck():
    import numpy as np
    rng = np.random.default_rng(17)
    h, w = 19, 23
    yy, xx = np.mgrid[:h, :w].astype("f4")
    world = np.stack([xx*.01, yy*.01, np.ones_like(xx)], axis=-1)
    normals = np.zeros_like(world)
    normals[..., 2] = 1
    material = np.where(xx < w//2, 1, 2)
    guides = dict(world=world, normal=normals, instance=material, material=material,
                  depth=np.ones((h,w), "f4"), valid=np.ones((h,w), bool),
                  hit_t=np.ones((h,w), "f4"), secondary_valid=np.ones((h,w), bool))
    image = np.repeat(np.where(xx < w//2, .25, 20)[..., None], 3, axis=-1).astype("f4")
    result = {}
    for kind in CANDIDATES:
        filtered = spatial(image, guides, kind)
        np.testing.assert_allclose(filtered, image, rtol=2e-6, atol=1e-6)
        hashes = [hashlib.sha256(spatial(image, guides, kind).tobytes()).hexdigest() for _ in range(8)]
        assert len(set(hashes)) == 1
        result[kind] = dict(constant_and_id_edge_preserved=True, eight_hashes=hashes)
    samples = rng.random((8,h,w,3), dtype="f4")
    directions = rng.normal(size=(8,h,w,3)).astype("f4")
    directions /= np.linalg.norm(directions, axis=-1, keepdims=True)
    joint = joint_conservative(samples, directions)
    np.testing.assert_allclose(joint.mean(0), samples.mean(0), rtol=1e-6, atol=1e-6)
    history = np.ones((h,w,3), "f4")
    current = np.ones_like(history)*2
    for age in range(1, 9):
        history = accumulate(history, current, age, reset=age==1)
    assert np.array_equal(history, current)
    save(OUT / "selfcheck.json", dict(filters=result, joint_mean_preserved=True,
         engine_event_reset_immediate=True, script_sha256=sha(Path(__file__))))


def replay_shadow():
    import numpy as np
    from PIL import Image, ImageDraw
    root = OUT.parent / "rt1c/witness/camera-0.25px-r0.04"
    old = OUT.parent / "motion-benches/S2"
    image = np.fromfile(root/"Interactive/0000.rgba16f", "<f2").reshape(288,512,4)[...,:3].astype("f4")
    world = np.fromfile(root/"reference/pose-0.world.f32", "<f4").reshape(288,512,4)
    normal = np.fromfile(root/"reference/pose-0.normal.f32", "<f4").reshape(288,512,4)
    ids = np.fromfile(root/"reference/pose-0.hit.u32", "<u4").reshape(288,512,4)
    meta = json.loads((root/"reference/pose-0.json").read_text())
    eye, forward = np.asarray(meta["eye"][:3]), np.asarray(meta["forward"][:3])
    guides = dict(world=world[...,:3], normal=normal[...,:3], instance=ids[...,0], material=ids[...,0],
         depth=np.sum((world[...,:3]-eye)*forward, axis=-1).astype("f4"), valid=ids[...,0]>0,
         hit_t=np.zeros((288,512), "f4"), secondary_valid=np.ones((288,512), bool))
    paths = [root/"Interactive/0000.rgba16f", root/"reference/pose-0.world.f32",
             root/"reference/pose-0.normal.f32", root/"reference/pose-0.hit.u32"]
    rows, panels = [], []
    tone = load("rt1c-witness").tone
    for candidate in CANDIDATES:
        filtered = spatial(image, guides, candidate)
        for stops in (0,4):
            suffix = "-plus4" if stops else ""
            ref_path = old/f"renderlab/camera-0.25px-r0.04/film-full{suffix}.png"
            ue_path = old/f"unreal/camera-0.25px-r0.04/HRT1/0000/final-radiance{suffix}.png"
            ref = np.asarray(Image.open(ref_path).convert("RGB").crop(tuple(meta["crop"]))).astype("f4")
            ue = np.asarray(Image.open(ue_path).convert("RGB").crop(tuple(meta["crop"]))).astype("f4")
            paths += [ref_path, ue_path]
            display = tone(filtered, stops).astype("f4")
            masks = dict(complete_crop=np.ones((288,512), bool), covered=guides["valid"])
            # The retained guide's w is the footprint-acceptance count; checked
            # against RT1c's published witness population counts before use.
            if np.count_nonzero(ids[...,3] == 256) == 125629:
                masks.update(fully_accepted=ids[...,3] == 256, mixed_footprint=ids[...,3] < 256)
            else:
                # Never relabel an unknown channel as footprint coverage.
                masks["geometry_boundary"] = boundary_mask(ids[...,0])
            measures = {}
            for name, mask in masks.items():
                if not mask.any():
                    continue
                error, floor = display[mask]-ref[mask], ue[mask]-ref[mask]
                measures[name] = dict(pixels=int(mask.sum()), display_rms=float(np.sqrt(np.mean(error**2))),
                    hrt1_rms=float(np.sqrt(np.mean(floor**2))), display_mean_bias=float(np.mean(error)),
                    passes=float(np.mean(error**2)) <= float(np.mean(floor**2)))
            rows.append(dict(candidate=candidate, stops=stops, measures=measures))
            if stops==4:
                panels.append((candidate, display.astype("u1")))
        del filtered
    sheet = Image.new("RGB", (1024,624), "#111820")
    draw = ImageDraw.Draw(sheet)
    for i,(name,pixels) in enumerate(panels):
        x,y=i%2*512,i//2*312
        draw.text((x+4,y+3),"+4 stops: "+name,fill="white")
        sheet.paste(Image.fromarray(pixels),(x,y+22))
    sheet.save(OUT/"shadow-inspection.png")
    assert abs(rows[1]["measures"]["complete_crop"]["display_rms"]-14.083180)<1e-5
    save(OUT/"shadow.json",dict(rows=rows, inputs={str(p):sha(p) for p in set(paths)},
         scope="single saved final-image crop, not a visibility-only denoiser; HDR energy/motion references unavailable",
         hit_channel_w_unique=np.unique(ids[...,3]).tolist()))


def boundary_mask(ids):
    import numpy as np
    result = np.zeros(ids.shape, bool)
    result[1:] |= ids[1:] != ids[:-1]
    result[:-1] |= ids[1:] != ids[:-1]
    result[:,1:] |= ids[:,1:] != ids[:,:-1]
    result[:,:-1] |= ids[:,1:] != ids[:,:-1]
    return result


def replay_cache():
    import numpy as np
    from PIL import Image
    root = OUT.parent / "m7-g1c/runs/x40/gates-a"
    summary_path = root/"g1-sv.json"
    recorded = json.loads(summary_path.read_text())
    original = OUT.parent / "m7-u0/scenes/SV/scene.json"
    assert sha(original) == recorded["scene_sha256"]
    scene = json.loads(original.read_text())
    inputs = {str(original):sha(original), str(summary_path):sha(summary_path)}
    rows = []
    for pose in ("overview","gaps"):
        ref_path = OUT.parent/f"m7-u0b/reference/sv-sky/SVSky-{pose}.npz"
        inputs[str(ref_path)] = sha(ref_path)
        with np.load(ref_path) as ref:
            world, normal, ids = ref["world"], ref["geometric_normal"], ref["hit"]
            guides = dict(world=world, normal=normal, instance=ids[...,0], material=ids[...,2],
                 depth=ref["depth"], valid=ids[...,0]>0, hit_t=np.zeros(ids.shape[:2], "f4"),
                 secondary_valid=np.ones(ids.shape[:2], bool))
            populations = {k[4:]:ref[k] for k in ref.files if k.startswith("pop_")}
        populations["geometry_boundary"] = boundary_mask(ids[...,0])
        for name, meshes in scene["populations"].items():
            selected = [i+1 for i,m in enumerate(scene["meshes"]) if m["name"] in meshes]
            populations["mesh-"+name] = np.isin(ids[...,0], selected)
        reference_path = root/f"{pose}-reference.png"
        reference = np.asarray(Image.open(reference_path).convert("RGB")).astype("f4")
        inputs[str(reference_path)] = sha(reference_path)
        for path in ("rt","nonrt"):
            image_path = root/f"{path}-rotated-4px-{pose}.png"
            inputs[str(image_path)] = sha(image_path)
            image = np.asarray(Image.open(image_path).convert("RGB")).astype("f4")
            for kind in CANDIDATES:
                filtered = spatial(image, guides, kind)
                measures = {}
                for name, mask in populations.items():
                    if mask.any():
                        error = filtered[mask]-reference[mask]
                        measures[name] = dict(pixels=int(mask.sum()), display_rms=float(np.sqrt(np.mean(error**2))),
                             display_mean_bias=float(np.mean(error)))
                rows.append(dict(path=path,pose=pose,candidate=kind,measures=measures,
                         scope="quantized gamma-2.2 display replay only; not incident HDR or original G1 metrics"))
            del image
        del world, normal, ids, guides, populations, reference
        guard()
    save(OUT/"cache.json", dict(rows=rows,inputs=inputs, missing=["HDR cache outputs", "direction slots", "pan images", "per-frame cut/event images"],
         scope="static display diagnostic; retained G1c raw outputs were deleted; no reconstructed HDR energy scores"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["replay", "selfcheck", "shadow", "cache", "all"])
    parser.add_argument("--cases", nargs="+", default=list(CASES))
    parser.add_argument("--wait-seconds", type=int, default=0, help="bounded resource wait, no work below the start floor")
    args = parser.parse_args()
    deadline = time.monotonic()+args.wait_seconds
    while memory()["available_gib"] < 8 and time.monotonic() < deadline:
        time.sleep(1)
    with cpu_job(args.command):
        if args.command == "all":
            selfcheck()
            replay_shadow()
            replay_cache()
            replay_reflections(args.cases)
        elif args.command == "replay":
            replay_reflections(args.cases)
        elif args.command == "selfcheck":
            selfcheck()
        elif args.command == "shadow":
            replay_shadow()
        else:
            replay_cache()


if __name__ == "__main__":
    main()
