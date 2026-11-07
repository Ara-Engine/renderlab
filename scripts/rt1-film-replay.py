"""CPU replay of RT1's frozen F0 false-shadow witnesses and current-source capture.

Reads private evidence without changing it; writes summaries/crops only beneath
--output. The current capture is gpu_rt1_film_defects, under scripts/n8-run.py.
No metric threshold, reference population or native validity mask is changed.
"""
import argparse
import csv
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sys
import time

for variable in ("OPENBLAS_NUM_THREADS", "OMP_NUM_THREADS", "NUMBA_NUM_THREADS"):
    os.environ[variable] = "2"

import numpy as np
from PIL import Image, ImageDraw
from scipy import ndimage

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "support"))
import fresh_gate_metrics as gate
import fresh_level_b_metrics as lb

SHAPE = (1080, 1920)
FILM = np.dtype([("counts", "<u4", (4,)), ("rest", "<f4", (8,))])


def read(path, dtype, shape=SHAPE):
    return np.memmap(path, mode="r", dtype=dtype, shape=shape)


def film_reference(directory, pose):
    a = read(directory / f"pose-{pose}.film.bin", FILM)
    c = a["counts"].astype(float)
    n = c[..., 2] + c[..., 3]
    ref = np.divide(c[..., 0] + c[..., 1], 256 * n,
                    out=np.ones(SHAPE), where=n > 0)
    hit = read(directory / f"pose-{pose}.hit.u32", "<u4", (*SHAPE, 4))
    depth = read(directory / f"pose-{pose}.world.f32", "<f4", (*SHAPE, 4))[..., 3]
    return dict(ref=ref, n=n, cover=hit[..., 0] != 0, depth=depth)


def metrics(candidate, d, valid):
    cover = d["cover"] & (d["n"] > 0)
    pops = gate.populations(d["ref"], d["depth"], cover)
    keep = cover & valid
    error = candidate - d["ref"]
    partial = pops["partial"] & valid
    solid = (pops["lit"] | pops["occluded"]) & valid
    edge = pops["occluded"] & ndimage.binary_dilation(
        pops["lit"] | pops["partial"], np.ones((7, 7), bool))
    return dict(
        covered=int(cover.sum()), missing=int((cover & ~valid).sum()),
        partial_n=int(partial.sum()), partial_rms=float(np.sqrt(np.mean(error[partial] ** 2))),
        solid_flips=int((solid & (np.abs(error) > .5)).sum()),
        umbra_leaks=int((pops["occluded"] & valid & (candidate > .1)).sum()),
        lit_darkened=int((pops["lit"] & valid & (candidate < .9)).sum()),
        lost_umbra_edges=int((edge & valid & (candidate > .5)).sum()),
        missed_shadows=int((keep & (d["ref"] <= .9) & (candidate >= .99)).sum()),
        false_shadows=int((keep & (d["ref"] >= .99) & (candidate <= .9)).sum()),
    )


def crops(path, witnesses, panels):
    radius, scale, width, height = 8, 5, 145, 120
    sheet = Image.new("RGB", (len(panels) * width, len(witnesses) * height), "#161616")
    draw = ImageDraw.Draw(sheet)
    for row, witness in enumerate(witnesses):
        x, y = witness["x"], witness["y"]
        for col, (name, values) in enumerate(panels):
            crop = values[y-radius:y+radius+1, x-radius:x+radius+1]
            image = Image.fromarray(np.uint8(np.clip(crop, 0, 1) * 255)).convert("RGB")
            image = image.resize((85, 85), Image.Resampling.NEAREST)
            sheet.paste(image, (col * width, row * height + 30))
            draw.text((col * width, row * height), f"{name} ({x},{y})", fill="white")
            draw.text((col * width, row * height + 14), f"V={values[y,x]:.6f}", fill="white")
            draw.rectangle((col * width + 40, row * height + 70,
                            col * width + 44, row * height + 74), outline="red")
    sheet.save(path)


def load_source(asset):
    spec = importlib.util.spec_from_file_location(
        "rt1_source", Path(__file__).with_name("quarry-shadow-probe.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.Source(asset)


def source_check(asset, f0):
    """Geometric corroboration, not a replay of unavailable hardware ray bits.

    Reuses the read-only source loader. Camera rays pass through the saved GPU
    points; the shadow diagnostic uses the disk axis, not an estimator direction.
    Only positive hit witnesses are evidence; a missing hit proves no empty space.
    """
    source = load_source(asset)
    directory = f0 / "quarry-policy-p0"
    info = json.loads((directory / "pose-0.json").read_text())
    eye = np.asarray(info["eye"][:3])
    world = read(directory / "pose-0.world.f32", "<f4", (*SHAPE, 4))
    normal = read(directory / "pose-0.normal.f32", "<f4", (*SHAPE, 4))
    hits = read(directory / "pose-0.hit.u32", "<u4", (*SHAPE, 4))
    light = lb.TO_LIGHT
    rows = []
    for x, y in ((1002, 491), (910, 635), (1212, 653)):
        point = world[y, x, :3].astype(float)
        n = normal[y, x, :3].astype(float)
        n /= np.linalg.norm(n)
        n *= 1 if n @ light >= 0 else -1
        bary = hits[y, x, 2:].copy().view("<f4").astype(float)
        rays = {}
        for name, origin, direction in (
            ("camera_through_recorded_point", eye, (point-eye)/np.linalg.norm(point-eye)),
            ("light_axis_1mm", point + .001*n, light),
            ("light_axis_10um", point + .00001*n, light),
        ):
            t, identity, _ = source.intersect(origin, direction)
            rays[name] = dict(origin=origin.tolist(), direction=direction.tolist(),
                              t=t if np.isfinite(t) else None, source_mesh_primitive_triangle=identity)
        rows.append(dict(pixel=[x, y],
                         gpu_placement_primitive=hits[y, x, :2].tolist(),
                         receiver_barycentrics=[1-bary.sum(), *bary], rays=rays))
    return rows


def point_reference(directory, current, asset, f0):
    """Decode the existing solid-verification test's independent 4096 salts.

    Only selected failures are evaluated. These are not whole-image point scores.
    The optional source geometry check replays the conservative edge-strip test;
    it does not reconstruct unavailable hardware camera rays.
    """
    source = load_source(asset) if asset else None
    result = {}
    for pose in (0, 1):
        pixels = np.fromfile(directory / f"pose-{pose}-pixels.u32", dtype="<u4")
        stats = np.fromfile(directory / f"pose-{pose}-stats.u32", dtype="<u4").reshape(-1, 4).astype(float)
        assert len(pixels) == len(stats) and np.all(stats[:, 3] == 4096)
        means = 1 - stats[:, 0] / (256 * stats[:, 3])
        variance = np.maximum(0, stats[:, 1]/stats[:, 3] - (stats[:, 0]/stats[:, 3])**2)
        se = np.sqrt(variance / stats[:, 3]) / 256
        rows = []
        hits = read(current / f"pose-{pose}-interactive.hit.u32", "<u4", (*SHAPE, 4))
        with np.load(lb.native_dir("vsm", pose) / "native.npz") as native:
            native_valid = native["valid"]
        info = json.loads((f0 / f"quarry-policy-p{pose}/pose-{pose}.json").read_text())
        eye = np.asarray(info["eye"][:3])
        for pixel, mean, error in zip(pixels, means, se):
            x, y = int(pixel % 1920), int(pixel // 1920)
            row = dict(x=x, y=y, point_visibility=float(mean), replica_standard_error=float(error),
                       hit_record_u32=hits[y, x].tolist(), native_available=bool(native_valid[y, x]))
            if source:
                h = hits[y, x]
                mesh, model = source.placements[int(h[0])-1]
                _, vertices, indices = source.meshes[mesh][0]
                v = vertices[indices[h[1]], :3].astype(float) @ model[:3, :3].astype(float).T + model[:3, 3]
                u, w = h[2:].copy().view("<f4").astype(float)
                weights = np.array([1-u-w, u, w])
                ab, ac = v[1]-v[0], v[2]-v[0]
                n = np.cross(ab, ac)
                area = np.linalg.norm(n)
                n /= area
                n *= 1 if n @ (eye - weights @ v) >= 0 else -1
                magnitude = np.float32(np.abs(v).max())
                epsilon = max(1e-5, 16 * float(np.nextafter(magnitude, np.float32(np.inf))-magnitude))
                margins = np.minimum(.25, 4*epsilon*np.sqrt(1.000004)*np.linalg.norm(
                    np.array([ac-ab, ac, ab]), axis=1)/area)
                row.update(near_edge=bool(np.any(weights < margins)),
                           viewed_ndl=float(n @ lb.TO_LIGHT),
                           placement=int(h[0]), primitive=int(h[1]))
            rows.append(row)
        result[str(pose)] = dict(
            selected=len(rows), persistent=int((means <= .9).sum()),
            persistent_upper_95=int((means + 1.96*se <= .9).sum()),
            prefix_only_lower_95=int((means - 1.96*se > .9).sum()),
            selected_missing_native=sum(not r["native_available"] for r in rows), pixels=rows)
        if source:
            result[str(pose)]["persistent_outside_edge_strip"] = sum(
                r["point_visibility"] <= .9 and not r["near_edge"] for r in rows)
    result["return_stats_identical"] = (
        directory / "pose-0-stats.u32").read_bytes() == (directory / "pose-2-stats.u32").read_bytes()
    return result


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--f0", type=Path, default=Path("D:/Coding/renderlab-work/m6-exact-quality/f0"))
    p.add_argument("--point", type=Path, default=Path("D:/Coding/renderlab-work/m6-level-b-probe/gate-final/reference"))
    p.add_argument("--r1", type=Path, default=Path("E:/renderlab-work/m6-exact-quality/r1"))
    p.add_argument("--native", type=Path, default=Path("D:/Coding/renderlab-work/ue-m5/Quarry/native-shadow"))
    p.add_argument("--current", type=Path)
    p.add_argument("--source-check", type=Path, metavar="RLA", help="also trace three CPU geometry witnesses")
    p.add_argument("--point-capture", type=Path, help="selected-pixel solid-verification outputs")
    p.add_argument("--geometry-asset", type=Path, help="optional read-only edge-strip classification")
    p.add_argument("--output", type=Path, required=True)
    a = p.parse_args()
    if a.point_capture and not a.current:
        p.error("--point-capture needs --current")
    started = time.perf_counter()
    a.output.mkdir(parents=True, exist_ok=True)
    lb.NATIVE = a.native
    result = dict(witnesses={}, metrics={}, inputs={}, current=bool(a.current))
    for pose in (0, 1):
        point = np.load(a.point / f"pose-{pose}-reference.npz")["ref"]
        old_exact = read(a.f0 / f"quarry-policy-p{pose}/pose-{pose}.exact.u32", "<u4") / 256.
        native = {name: np.load(lb.native_dir(case, pose) / "native.npz")
                  for name, case in (("VSM", "vsm"), ("HRT", "hrt1"))}
        current = {}
        current_valid = {}
        if a.current:
            for mode in ("interactive", "exact", "separate"):
                path = a.current / f"pose-{pose}-{mode}.u32"
                current[mode] = read(path, "<u4") / 256.
                with path.open("rb") as f:
                    result["inputs"][str(path)] = hashlib.file_digest(f, "sha256").hexdigest()
                hit_mode = "exact" if mode == "separate" else mode
                hits = read(a.current / f"pose-{pose}-{hit_mode}.hit.u32", "<u4", (*SHAPE, 4))
                current_valid[mode] = hits[..., 0] != 0
        for policy in ("policy", "minimal"):
            key = f"{pose}/{policy}"
            csvpath = a.f0 / f"scores/pose-{pose}-{policy}-Interactive-defects.csv"
            with csvpath.open(newline="") as f:
                false = [r for r in csv.DictReader(f) if r["kind"] == "false"]
            reference_path = a.f0 / f"scores/pose-{pose}-{policy}-reference.npz"
            with np.load(reference_path) as z:
                d = {k: z[k] for k in ("ref", "n", "cover", "depth")}
            rows = []
            for row in false:
                x, y = int(row["x"]), int(row["y"])
                value = dict(x=x, y=y, film=float(d["ref"][y, x]),
                             old_interactive=float(row["candidate"]), old_exact=float(old_exact[y, x]),
                             point_16384=float(point[y, x]), accepted=int(d["n"][y, x]),
                             sigma=float(row["sigma"]))
                value.update({mode: float(v[y, x]) for mode, v in current.items()})
                rows.append(value)
            assert len(rows) == {(0,"policy"):9,(0,"minimal"):21,(1,"policy"):11,(1,"minimal"):17}[pose,policy]
            result["witnesses"][key] = dict(
                count=len(rows), point_also_false=sum(r["point_16384"] <= .9 for r in rows),
                exact_also_false=sum(r["old_exact"] <= .9 for r in rows), pixels=rows)
            candidates = dict(point_16384=point, old_exact=old_exact, **current)
            scored = {name: metrics(v, d, current_valid.get(name, np.ones(SHAPE, bool)))
                      for name, v in candidates.items()}
            scored.update({name: metrics(z["visibility"], d, z["valid"]) for name, z in native.items()})
            result["metrics"][key] = scored
            panels = [("Film", d["ref"]), ("Point 16384", point), ("Old Exact", old_exact)]
            if current:
                panels.append(("Current fast", current["interactive"]))
            panels.append(("Unreal VSM", native["VSM"]["visibility"]))
            crops(a.output / f"pose-{pose}-{policy}-false.png", rows, panels)
        d = film_reference(a.r1 / f"quarry-viewed-p{pose}", pose)
        scored = {name: metrics(v, d, current_valid[name]) for name, v in current.items()}
        scored.update({name: metrics(z["visibility"], d, z["valid"]) for name, z in native.items()})
        result["metrics"][f"{pose}/viewed"] = scored
        if current:
            values = current["interactive"]
            false = d["cover"] & (d["n"] > 0) & (d["ref"] >= .99) & (values <= .9)
            rows = [dict(x=int(x), y=int(y)) for y, x in zip(*np.nonzero(false))]
            if rows:
                crops(a.output / f"pose-{pose}-viewed-false.png", rows,
                      [("Film viewed", d["ref"]), ("Current fast", values),
                       ("Current Exact", current["exact"]), ("Unreal VSM", native["VSM"]["visibility"])])
    if a.source_check:
        result["source_geometry_witnesses"] = source_check(a.source_check, a.f0)
    if a.point_capture:
        result["selected_point_reference"] = point_reference(a.point_capture, a.current, a.geometry_asset, a.f0)
    result["seconds"] = time.perf_counter() - started
    (a.output / "replay.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    for key, row in result["witnesses"].items():
        print(key, {k: v for k, v in row.items() if k != "pixels"})
    for key, row in result["metrics"].items():
        print(key, {name: (round(m["partial_rms"], 6), m["false_shadows"]) for name, m in row.items()})
    print(f"CPU replay: {result['seconds']:.2f} s")


if __name__ == "__main__":
    main()
