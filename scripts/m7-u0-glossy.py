"""M7-U0c: localize the roughness-0.1 glossy disagreement with Unreal (CPU only).

Reads the retained M5 directional-black fixture evidence (UE 5.7.4 Vulkan SM5 SceneCapture
radiance plus the decoded normal/base-colour/roughness probe buffers) and tests one
hypothesis: UE's Vulkan SM5 shaders compile `half` as relaxed precision, so the BxDF
context's N.H lands on the fp16 grid (step 2^-11 just below 1). At alpha^2 ~ 1e-4 the GGX
peak then collapses from ~3400 to ~26 (N.H = 1 - 2^-11) or ~7 (N.H = 1 - 2^-10).

Two independent tests, no fitted parameters:
1. Implied N.H: invert UE's observed specular for the N.H that the physical model would need,
   holding every other factor at its decoded value. Relaxed precision predicts clusters at
   integer multiples of 2^-11 below 1; a physical lobe predicts a continuum.
2. Emulation: re-evaluate the direct BRDF with N, V, L, the dot products and N.H rounded to
   fp16, and compare full-object interiors against UE.

Usage: py -3 -B scripts/m7-u0-glossy.py [--output E:/renderlab-work/m7-u0/glossy] [--d3d12 <fixture capture dir>]
With --d3d12 it runs the control instead: the same fixture on D3D12 SM6 (half compiles as float there),
against delta and exactly disk-integrated physical references at sun angles 0, 0.5357 and 4.584 degrees.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import sys

import numpy as np

BASE = Path("D:/Coding/renderlab-work/ue-m5")
RUN = BASE / "Probe/Artifacts/run-d8b967e0bd1f4c2d85ca12101701f897"
EVIDENCE = Path("D:/Coding/projects/desktop/renderlab/.refs/m5-m6/ue-runtime")
STEP = 2.0 ** -11  # fp16 spacing in [0.5, 1)


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def exr(path):
    sys.path.insert(0, str(EVIDENCE / "python"))
    import OpenEXR  # private wheel copy retained with the M5 evidence

    with OpenEXR.File(str(path), separate_channels=True) as image:
        channels = image.channels()
        return np.stack([channels[c].pixels for c in "RGB"], axis=-1).astype(np.float64)


def unit(v):
    return v / np.linalg.norm(v, axis=-1, keepdims=True)


def from_ue(v):
    """UE world (x, y, z) -> fixture source space (y, z, -x)."""
    return np.stack((v[..., 1], v[..., 2], -v[..., 0]), axis=-1)


def views(camera, width, height):
    focal = (width / 2) / np.tan(np.radians(camera["fov_x_degrees"]) / 2)
    y, x = np.mgrid[0:height, 0:width]
    f, r, u = (np.array(camera[k]) for k in ("forward", "right", "up"))
    ray = f + ((x + 0.5 - width / 2) / focal)[..., None] * r - ((y + 0.5 - height / 2) / focal)[..., None] * u
    return unit(-ray)


def h(x):
    return np.asarray(x, dtype=np.float64).astype(np.float16).astype(np.float64)


def dot16(a, b):
    """fp16 dot product: rounded products, fused-multiply-add accumulation rounded to fp16."""
    a, b = h(a), h(b)
    acc = h(a[..., 0] * b[..., 0])
    for k in (1, 2):
        acc = h(a[..., k] * b[..., k] + acc)
    return acc


def brdf(n, v, l, rough, base, metal, spec, lux, relaxed):
    """Lambert + GGX direct light. relaxed=True rounds the BxDF context to fp16.

    The physical branch uses exact height-correlated Smith; the relaxed branch uses the
    published joint-Smith approximation that UE's default-lit model documents (Karis 2013 /
    Heitz 2014), so model differences outside precision are visible in the physical branch.
    """
    if relaxed:
        n, v, l = h(n), h(v), h(l)
        nl, nv, vl = dot16(n, l), dot16(n, v), dot16(v, l)
        inv = 1 / np.sqrt(2 + 2 * vl)
        nh = h(np.clip(h(nl + nv) * inv, 0, 1))
        vh = h(np.clip(inv + inv * vl, 0, 1))
    else:
        n = unit(n)
        nl, nv = np.sum(n * l, -1), np.sum(n * v, -1)
        hv = unit(v + l)
        nh, vh = np.clip(np.sum(n * hv, -1), 0, 1), np.clip(np.sum(v * hv, -1), 0, 1)
    nl = np.maximum(nl, 0)
    nv_c = np.maximum(nv, 1e-5)
    a2 = rough ** 4
    d = a2 / (np.pi * np.maximum((nh * a2 - nh) * nh + 1, 1e-30) ** 2)
    if relaxed:
        a = np.sqrt(a2)
        vis = 0.5 / (nl * (nv_c * (1 - a) + a) + nv_c * (nl * (1 - a) + a))
    else:
        vis = 0.5 / (nl * np.sqrt(a2 + (1 - a2) * nv_c ** 2) + nv_c * np.sqrt(a2 + (1 - a2) * nl ** 2))
    f0 = 0.08 * spec[..., None] * (1 - metal[..., None]) + base * metal[..., None]
    fresnel = f0 + (1 - f0) * ((1 - vh) ** 5)[..., None]
    diffuse = base * ((1 - metal) / np.pi)[..., None]
    return (diffuse + (d * vis)[..., None] * fresnel) * nl[..., None] * lux, nh, (d * vis * nl)


def rel_l1(a, b, mask):
    return float(np.abs(a[mask] - b[mask]).sum() / max(np.abs(b[mask]).sum(), 1e-30))


def interior(mask, radius=2):
    out = mask.copy()
    for dy in range(-radius, radius + 1):
        for dx in range(-radius, radius + 1):
            s = np.roll(mask, (dy, dx), (0, 1))
            if dy > 0: s[:dy] = False
            if dy < 0: s[dy:] = False
            if dx > 0: s[:, :dx] = False
            if dx < 0: s[:, dx:] = False
            out &= s
    return out


def implied_noh(ue, normal, v, l, rough, base, metal, spec, lux, mask):
    """Invert observed red specular for the N.H the physical NDF needs; distance of that level to the fp16 grid."""
    n = unit(normal[mask]); vv = v[mask]; ll = l[mask]
    a2 = rough[mask] ** 4
    nl = np.sum(n * ll, -1); nv = np.sum(n * vv, -1)
    hv = unit(vv + ll); vh = np.clip(np.sum(vv * hv, -1), 0, 1)
    nh = np.clip(np.sum(n * hv, -1), 0, 1)
    vis = 0.5 / (nl * np.sqrt(a2 + (1 - a2) * nv ** 2) + nv * np.sqrt(a2 + (1 - a2) * nl ** 2))
    b = base[mask, 0]; m = metal[mask]; s = spec[mask]
    f0 = 0.08 * s * (1 - m) + b * m
    fr = f0 + (1 - f0) * (1 - vh) ** 5
    spec_obs = ue[mask, 0] / (nl * lux[0]) - b * (1 - m) / np.pi
    strong = (spec_obs > 0) & (nl > 0.2) & ((1 - nh) < 6 * STEP) & (spec_obs / (vis * fr) > 1.0)
    dd = spec_obs[strong] / (vis[strong] * fr[strong])
    den = np.sqrt(a2[strong] / (np.pi * dd))
    implied = (1 - np.sqrt(np.clip((1 - den) / (1 - a2[strong]), 0, 1))) / STEP
    physical = (1 - nh[strong]) / STEP
    return dict(pixels=int(strong.sum()), implied_mean_distance_to_integer_step=float(np.abs(implied - np.rint(implied)).mean()),
                physical_mean_distance_to_integer_step=float(np.abs(physical - np.rint(physical)).mean()), uniform_expectation=0.25,
                implied_minus_physical_steps_rms=float(np.sqrt(((implied - physical) ** 2).mean())) if strong.any() else None)


def disk_reference(normal, v, l, rough, base, metal, spec, lux, radius, mask, k=32):
    """Exact sun-disk integration (uniform radiance, angular radius `radius`) by stratified cone quadrature."""
    out = np.zeros(mask.shape + (3,))
    n, vv, ll = normal[mask], v[mask], l[mask]
    if radius == 0:
        out[mask], _, _ = brdf(n, vv, ll, rough[mask], base[mask], metal[mask], spec[mask], lux, relaxed=False)
        return out
    cos_r = math.cos(radius)
    solid = 2 * math.pi * (1 - cos_r)
    radiance = np.asarray(lux) / (math.pi * math.sin(radius) ** 2)
    axis = ll[0] / np.linalg.norm(ll[0])
    t = np.cross(axis, [0, 1, 0] if abs(axis[1]) < .9 else [1, 0, 0]); t /= np.linalg.norm(t); b = np.cross(axis, t)
    acc = np.zeros((int(mask.sum()), 3))
    for i in range(k):
        for j in range(k):
            z = 1 - (i + .5) / k * (1 - cos_r); phi = 2 * math.pi * (j + .5) / k; s = math.sqrt(max(0, 1 - z * z))
            d = z * axis + s * math.cos(phi) * t + s * math.sin(phi) * b
            # brdf() includes the cosine and multiplies by its "lux" argument: pass radiance * dOmega.
            value, _, _ = brdf(n, vv, np.broadcast_to(d, vv.shape), rough[mask], base[mask], metal[mask], spec[mask],
                               radiance * solid / (k * k), relaxed=False)
            acc += value
    out[mask] = acc
    return out


def d3d12(directory, output):
    """U0c control: the same fixture captured on D3D12 SM6, where UE compiles half as float for deferred lighting."""
    manifest = json.loads((BASE / "manifest.json").read_text())
    capture = json.loads((directory / "capture.json").read_text())
    width, height = manifest["resolution"]
    camera = dict(forward=from_ue(np.array(capture["camera_forward"])).tolist(), right=from_ue(np.array(capture["camera_right"])).tolist(),
                  up=from_ue(np.array(capture["camera_up"])).tolist(), fov_x_degrees=capture["horizontal_fov_deg"])
    v = views(camera, width, height)
    travel = from_ue(np.array(capture["light_forward_ue"], dtype=np.float64))
    l = np.broadcast_to(-travel / np.linalg.norm(travel), v.shape)
    lux = np.array(capture["light_rgb"])
    normal = from_ue(exr(directory / "normal-angle-0.0000.exr"))
    base = exr(directory / "basecolor-angle-0.0000.exr")
    rms = exr(RUN / "rms.exr")  # material constants; same 8-bit GBuffer encoding on both RHIs
    rough, metal, spec = rms[..., 0], rms[..., 1], rms[..., 2]
    with np.load(EVIDENCE / "source-reference.npz") as cache:
        ids = cache["ids"]
    report = dict(control="D3D12 SM6 recapture of the directional-black fixture; half compiles as float in deferred lighting",
                  capture=capture, angles=[], implied_noh={})
    names = [o["source"]["name"] for o in manifest["objects"]]
    glossy = [n for n in names if n.endswith(("0.1", "0.5", "-1"))]
    masks = {n: interior(ids == names.index(n)) for n in glossy}
    union = np.any(np.stack(list(masks.values())), 0)
    for angle in (0.0, 0.5357, 4.583662361):
        path = directory / f"scene-linear-angle-{angle:.4f}.exr"
        if not path.exists():
            continue
        ue = exr(path)
        reference = disk_reference(normal, v, l, rough, base, metal, spec, lux, math.radians(angle) / 2, union)
        delta = disk_reference(normal, v, l, rough, base, metal, spec, lux, 0.0, union)
        rows = {}
        unclipped = (ue < 65504).all(-1)  # the RGBA16F capture target saturates at the fp16 maximum
        for name, mask in masks.items():
            keep = mask & unclipped
            rows[name] = dict(ue_vs_disk_reference=rel_l1(ue, reference, keep), ue_vs_delta=rel_l1(ue, delta, keep),
                              clipped_pixels=int((mask & ~unclipped).sum()), pixels=int(keep.sum()),
                              peak_red=dict(ue=float(ue[mask, 0].max()), disk=float(reference[mask, 0].max()), delta=float(delta[mask, 0].max())))
        report["angles"].append(dict(angle_degrees=angle, objects=rows))
        if angle == 0.0:
            for name in ("metal-rough-0.1", "dielectric-rough-0.1"):
                report["implied_noh"][name] = implied_noh(ue, normal, v, l, rough, base, metal, spec, lux, masks[name])
    (output / "glossy-d3d12.json").write_text(json.dumps(report, indent=1, default=float) + "\n")
    print(json.dumps(dict(angles=report["angles"], implied=report["implied_noh"]), indent=1))


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--output", type=Path, default=Path("E:/renderlab-work/m7-u0/glossy"))
    p.add_argument("--d3d12", type=Path, help="fixture capture directory from m7-u0-ue.py fixture")
    opts = p.parse_args()
    opts.output.mkdir(parents=True, exist_ok=True)
    if opts.d3d12:
        return d3d12(opts.d3d12, opts.output)
    manifest = json.loads((BASE / "manifest.json").read_text())
    capture = json.loads((RUN / "capture.json").read_text())
    width, height = manifest["resolution"]
    ue = exr(RUN / "scene-linear.exr")
    normal = from_ue(exr(RUN / "normal.exr"))
    base = exr(RUN / "basecolor.exr")
    rms = exr(RUN / "rms.exr")
    rough, metal, spec = rms[..., 0], rms[..., 1], rms[..., 2]
    with np.load(EVIDENCE / "source-reference.npz") as cache:
        ids = cache["ids"]
    raw_path = BASE / "renderlab-directional-black/pose-000.rgba16f"
    raw_meta = json.loads(raw_path.with_suffix(".json").read_text())
    renderlab = np.fromfile(raw_path, dtype="<f2").reshape(height, width, 4)[..., :3].astype(np.float64) / raw_meta["pre_exposure"]
    v = views(capture["camera_source_space"], width, height)
    travel = from_ue(np.array(capture["light_travel_ue"], dtype=np.float64))
    l = np.broadcast_to(-travel / np.linalg.norm(travel), v.shape)
    lux = np.array(capture["light_rgb_lux"])
    physical, nh_phys, _ = brdf(normal, v, l, rough, base, metal, spec, lux, relaxed=False)
    relaxed, nh_rel, _ = brdf(normal, v, l, rough, base, metal, spec, lux, relaxed=True)

    report = dict(
        hypothesis="UE Vulkan SM5 compiles half as min16float (SPIR-V RelaxedPrecision); BxDFContext N.H on the fp16 grid",
        inputs={str(q): sha(q) for q in (RUN / "scene-linear.exr", RUN / "normal.exr", RUN / "basecolor.exr", RUN / "rms.exr", raw_path)},
        objects=[], implied_noh=[])
    for i, obj in enumerate(manifest["objects"]):
        mask = interior(ids == i)
        if not mask.any():
            continue
        report["objects"].append(dict(
            name=obj["source"]["name"], pixels=int(mask.sum()),
            ue_vs_renderlab=rel_l1(ue, renderlab, mask),
            ue_vs_physical_effective=rel_l1(ue, physical, mask),
            ue_vs_relaxed_emulation=rel_l1(ue, relaxed, mask),
            renderlab_vs_physical_effective=rel_l1(renderlab, physical, mask),
            peak_red=dict(ue=float(ue[mask, 0].max()), renderlab=float(renderlab[mask, 0].max()),
                          physical=float(physical[mask, 0].max()), relaxed=float(relaxed[mask, 0].max()))))

    # Test 1: invert UE's observed red specular for the N.H the physical NDF would need.
    for name in ("metal-rough-0.1", "dielectric-rough-0.1"):
        i = next(k for k, o in enumerate(manifest["objects"]) if o["source"]["name"] == name)
        mask = interior(ids == i)
        n = unit(normal[mask]); vv = v[mask]; ll = l[mask]
        a2 = rough[mask] ** 4
        nl = np.sum(n * ll, -1); nv = np.sum(n * vv, -1)
        hv = unit(vv + ll); vh = np.clip(np.sum(vv * hv, -1), 0, 1)
        nh = np.clip(np.sum(n * hv, -1), 0, 1)
        vis = 0.5 / (nl * np.sqrt(a2 + (1 - a2) * nv ** 2) + nv * np.sqrt(a2 + (1 - a2) * nl ** 2))
        b = base[mask, 0]; m = metal[mask]; s = spec[mask]
        f0 = 0.08 * s * (1 - m) + b * m
        fr = f0 + (1 - f0) * (1 - vh) ** 5
        spec_obs = ue[mask, 0] / (nl * lux[0]) - b * (1 - m) / np.pi
        # Only pixels whose specular is far above the off-peak tail resolve N.H uniquely.
        strong = (spec_obs > 0) & (nl > 0.2) & ((1 - nh) < 6 * STEP) & (spec_obs / (vis * fr) > 1.0)
        dd = spec_obs[strong] / (vis[strong] * fr[strong])
        den = np.sqrt(a2[strong] / (np.pi * dd))
        nh_implied = np.sqrt(np.clip((1 - den) / (1 - a2[strong]), 0, 1))
        steps_implied = (1 - nh_implied) / STEP
        steps_physical = (1 - nh[strong]) / STEP
        frac = np.abs(steps_implied - np.rint(steps_implied))
        report["implied_noh"].append(dict(
            object=name, pixels=int(strong.sum()),
            implied_steps_below_one=np.round(np.sort(steps_implied), 3).tolist(),
            physical_steps_below_one=np.round(np.sort(steps_physical), 3).tolist(),
            implied_mean_distance_to_integer_step=float(frac.mean()),
            physical_mean_distance_to_integer_step=float(np.abs(steps_physical - np.rint(steps_physical)).mean()),
            uniform_expectation=0.25,
            emulated_noh_matches_implied_level=float(np.mean(np.rint((1 - nh_rel[mask][strong]) / STEP) == np.rint(steps_implied)))))
    (opts.output / "glossy-cause.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(dict(objects=report["objects"], implied=[{k: r[k] for k in r if "steps_below" not in k} for r in report["implied_noh"]]), indent=1))


if __name__ == "__main__":
    main()
