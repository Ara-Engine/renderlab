"""CPU replay of N1b IDs, reconstructed receivers, high-sample shadows and timings.

No renderer or reference is changed. Pass --asset for exact shared-edge classification
on the quarry. Raw arrays remain local to E: and may be removed after this summary.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
from types import SimpleNamespace

for variable in ("OPENBLAS_NUM_THREADS", "OMP_NUM_THREADS", "NUMBA_NUM_THREADS"):
    os.environ[variable] = "2"
import numpy as np
from PIL import Image


def tone(hdr, stops=0):
    v = np.maximum(0, hdr[..., :3].astype(float) * 2**stops)
    v /= 1 + v
    v = np.where(v <= .0031308, 12.92 * v, 1.055 * v**(1/2.4) - .055)
    return np.rint(np.clip(v, 0, 1) * 255).astype("u1")


def stats(v):
    return dict(n=int(v.size), rms=float(np.sqrt(np.mean(v*v))), mean=float(np.mean(v)),
                p99_abs=float(np.quantile(np.abs(v), .99)), max_abs=float(np.max(np.abs(v)))) if v.size else dict(n=0)


def classify(a, b, source, matrix=None, width=1920, height=1080):
    covered = (a[:, 0] != 0) | (b[:, 0] != 0)
    common = (a[:, 0] != 0) & (b[:, 0] != 0)
    same = (a[:, :2] == b[:, :2]).all(1)
    different = np.flatnonzero(common & ~same)
    shared = []
    corners = []
    boundary = []
    unexplained = []
    edge_distances = []
    for i in different:
        if a[i, 0] != b[i, 0] or source is None:
            continue
        mesh, _ = source.placements[int(a[i, 0])-1]
        (_, vertices, indices), = source.meshes[mesh]
        pa = vertices[indices[a[i, 1]], :3]
        pb = vertices[indices[b[i, 1]], :3]
        matches = (pa[:, None] == pb[None]).all(-1).any(1).sum()
        if matches >= 2:
            shared.append(int(i))
        elif matches == 1:
            corners.append(int(i))
    for i in different:
        if int(i) in shared or int(i) in corners:
            continue
        distance = float("inf")
        if source is not None and matrix is not None:
            for record in (a[i], b[i]):
                mesh, model = source.placements[int(record[0])-1]
                (_, vertices, indices), = source.meshes[mesh]
                p = vertices[indices[record[1]], :3].astype(float) @ model[:3,:3].T + model[:3,3]
                clip = np.c_[p, np.ones(3)] @ matrix.T
                if np.any(clip[:,3] <= 0):
                    continue
                xy = (clip[:,:2]/clip[:,3,None] * [1,-1] + 1) * [width/2,height/2]
                point = np.array([i % width + .5, i // width + .5])
                edges = np.roll(xy,-1,axis=0)-xy
                edge = np.abs(edges[:,0]*(point-xy)[:,1]-edges[:,1]*(point-xy)[:,0])/np.maximum(np.linalg.norm(edges,axis=1),1e-30)
                distance = min(distance,float(edge.min()))
        if np.isfinite(distance):
            edge_distances.append(distance)
        if distance <= .01:
            boundary.append(int(i))
        else:
            unexplained.append(int(i))
    edge_margin = np.minimum.reduce([a[:, 2:].view("<f4")[:, 0], a[:, 2:].view("<f4")[:, 1],
                                    1-a[:, 2:].view("<f4").sum(1)])
    row = dict(union_covered=int(covered.sum()), common_covered=int(common.sum()),
               same_identity=int((same & common).sum()),
               bitwise_record_agreement_covered_percent=float(100*((a==b).all(1)&common).sum()/max(1,covered.sum())),
               identity_agreement_covered_percent=float(100*(same & common).sum()/max(1, covered.sum())),
               identity_agreement_frame_percent=float(100*same.mean()),
               coverage_disagreements=int((covered & ~common).sum()), identity_disagreements=len(different),
               exact_position_shared_edge=len(shared), exact_position_shared_vertex=len(corners), raster_boundary_le_001_pixel=len(boundary), other_identity=len(unexplained),
               boundary_pixel_distances=stats(np.asarray(edge_distances)),
               differing_near_edge_le_001=int((edge_margin[different] <= .01).sum()),
               witnesses=[dict(pixel=int(i), raster=a[i].tolist(), traced=b[i].tolist(), shared_edge=i in shared, shared_vertex=i in corners, raster_boundary=i in boundary) for i in [*unexplained,*boundary,*shared,*corners][:64]])
    return row, common, same


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("run", type=Path)
    p.add_argument("--control", type=Path)
    p.add_argument("--asset", type=Path)
    p.add_argument("--scene", type=Path)
    p.add_argument("--size", nargs=2, type=int, default=[1920, 1080])
    p.add_argument("--film-images", type=Path)
    a = p.parse_args()
    w, h = a.size
    control = a.control or a.run
    source = None
    if a.asset:
        spec = importlib.util.spec_from_file_location("probe", Path(__file__).with_name("quarry-shadow-probe.py"))
        probe = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(probe)
        source = probe.Source(a.asset)
    elif a.scene:
        scene = json.loads(a.scene.read_text())
        source = SimpleNamespace(placements=[(i, np.eye(4)) for i in range(len(scene["meshes"]))],
            meshes=[[(None, np.asarray(m["positions"], "<f4"), np.asarray(m["indices"], "<u4").reshape(-1,3))] for m in scene["meshes"]])
    summary = dict(size=a.size, run=str(a.run), control=str(control), poses={}, timings={})
    for path in sorted(a.run.glob("*.hdr")):
        hdr = np.fromfile(path, "<f2").reshape(h,w,4)
        Image.fromarray(tone(hdr)).save(path.with_suffix(".png"))
    timings = json.loads((a.run / "timing.json").read_text())
    for label in dict.fromkeys(t["label"] for t in timings if t["label"] != "warm"):
        summary["timings"][label] = {}
        for field in ("primary_ms", "shadow_ms", "span_ms"):
            v = [t[field] for t in timings if t["label"] == label and t.get(field) is not None]
            if v:
                summary["timings"][label][field] = dict(p50=float(np.median(v)), p95=float(np.quantile(v, .95)), n=len(v), min=min(v), max=max(v))
    for path in sorted(a.run.glob("p*-VisibilityBuffer.hit")):
        label = path.stem
        pose = int(label[1:].split("-", 1)[0])
        rt = control / f"p{pose}-Traced"
        if not rt.with_suffix(".hit").exists():
            continue
        raster = np.fromfile(path, "<u4").reshape(-1, 4)
        traced = np.fromfile(rt.with_suffix(".hit"), "<u4").reshape(-1, 4)
        camera = path.with_suffix(".camera.json")
        matrix = np.asarray(json.loads(camera.read_text())).reshape(4,4).T if camera.exists() else None
        row, common, same = classify(raster, traced, source, matrix, w, h)
        cull = path.with_suffix(".cull.json")
        if cull.exists():
            row["clusters_total_previous_pass_recovery_pass"] = json.loads(cull.read_text())
        rw = np.fromfile(path.with_suffix(".world"), "<f4").reshape(-1, 4)
        tw = np.fromfile(rt.with_suffix(".world"), "<f4").reshape(-1, 4)
        distance = np.linalg.norm(rw[:, :3].astype(float)-tw[:, :3], axis=1)
        # Distinct scans can put two surfaces within the existing receiver precision
        # radii. Keep these separate from true shared-edge and coverage ties.
        close = []
        for witness in row["witnesses"]:
            i = witness["pixel"]
            if witness["shared_edge"] or witness["shared_vertex"] or witness["raster_boundary"]:
                continue
            if distance[i] <= float(rw[i,3]+tw[i,3]):
                witness["near_coincident_world_gap_um"] = float(distance[i]*1e6)
                close.append(i)
        row["near_coincident_precision_winners"] = len(close)
        row["other_identity"] -= len(close)
        row["world_distance_mm_same_triangle"] = stats(distance[common & same] * 1000)
        row["same_triangle_over_01mm"] = int((common & same & (distance > .0001)).sum())
        worst = int(np.argmax(np.where(common & same, distance, 0)))
        row["largest_same_triangle_witness"] = dict(pixel=[worst % w,worst // w],world_gap_um=float(distance[worst]*1e6),
            raster_bary=raster[worst,2:].view("<f4").tolist(),traced_bary=traced[worst,2:].view("<f4").tolist())
        if source is not None and matrix is not None:
            mesh, model = source.placements[int(raster[worst,0])-1]
            (_, vertices, indices), = source.meshes[mesh]
            tri = vertices[indices[raster[worst,1]],:3].astype(float) @ model[:3,:3].T + model[:3,3]
            normal = np.cross(tri[1]-tri[0],tri[2]-tri[0]);normal /= max(np.linalg.norm(normal),1e-30)
            ndc = [(worst%w+.5)/w*2-1,1-(worst//w+.5)/h*2]
            inverse = np.linalg.inv(matrix)
            near = inverse @ [*ndc,1,1];far = inverse @ [*ndc,0,1]
            direction = far[:3]/far[3]-near[:3]/near[3];direction /= np.linalg.norm(direction)
            row["largest_same_triangle_witness"]["absolute_view_normal_dot"] = float(abs(normal@direction))
        rc = np.fromfile(path.with_suffix(".counts"), "<u4") / 256.
        tc = np.fromfile(rt.with_suffix(".counts"), "<u4") / 256.
        row["virtual_visibility_delta_same_triangle"] = stats((rc-tc)[common & same])
        rh = np.fromfile(path.with_suffix(".hdr"), "<f2").reshape(h, w, 4)
        th = np.fromfile(rt.with_suffix(".hdr"), "<f2").reshape(h, w, 4)
        row["hdr_delta"] = stats(rh[..., :3].astype(float)-th[..., :3].astype(float))
        Image.fromarray(tone(rh)).save(a.run / f"{label}.png")
        Image.fromarray(tone(rh, 4)).save(a.run / f"{label}-plus4.png")
        error = np.abs(tone(rh).astype(float)-tone(th).astype(float))
        Image.fromarray(np.clip(error*16, 0, 255).astype("u1")).save(a.run / f"{label}-difference-x16.png")
        refpath = control / f"p{pose}.reference"
        if refpath.exists():
            ref, sigma = np.fromfile(refpath, "<f4").reshape(-1, 2).T
            valid = common & same
            partial = valid & (ref > .001) & (ref < .999)
            lit = valid & (ref >= .999)
            umbra = valid & (ref <= .001)
            row["high_sample_center_reference"] = {}
            for name, values in (("raster_virtual", rc), ("traced_virtual", tc)):
                row["high_sample_center_reference"][name] = dict(partial=stats((values-ref)[partial]),
                    solid_flips=int((valid & ((lit | umbra)) & (np.abs(values-ref) > .5)).sum()),
                    umbra_leaks=int((umbra & (values > .1)).sum()), lit_darkened=int((lit & (values < .9)).sum()))
        if a.film_images and pose < 2:
            film_path = a.film_images / f"quarry-p{pose}-film.png"
            if not film_path.exists():
                film_path = a.film_images / f"p{pose}-e0-radiance.png"
            if film_path.exists():
                film = np.asarray(Image.open(film_path).convert("RGB"), float)
                if film.shape == (h,w,3):
                    row["archived_1024x32_film_codes"] = dict(raster=stats(tone(rh).astype(float)-film), traced=stats(tone(th).astype(float)-film),
                        reference=str(film_path), limitation="8-bit published radiance image, no original HDR arrays; no fresh Level-F certification")
                else:
                    row["archived_film_contact_sheet"] = dict(reference=str(film_path),limitation="published downscaled contact sheet; qualitative inspection only")
        summary["poses"][str(pose)] = row
    (a.run / "summary.json").write_text(json.dumps(summary, indent=2, allow_nan=False))
    print(json.dumps(dict(timings=summary["timings"], poses={k: {name: v for name, v in row.items() if name != "witnesses"} for k, row in summary["poses"].items()}), indent=2))


if __name__ == "__main__":
    main()
