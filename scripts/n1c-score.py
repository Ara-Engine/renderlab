"""CPU scoring of N1c LOD-cut primaries against traced primaries, the 64-seed Exact
visibility reference and the archived film radiance images.

Labels are p{pose}-lod{tau}-sw{extent} against p{pose}-Traced from the same RT-device run.
Shadow metrics follow N1b/N8 (partial RMS, solid flips, umbra leaks, lit darkened, lost
umbra edges) but use every pixel both producers cover: LOD receivers sit on a different
triangle by design. Raw arrays stay on E:; PNGs and summary.json are kept.

    py -3 -B scripts/n1c-score.py <run>/quarry --package <virtual package> --film <dir>
"""
import argparse
import importlib.util
import json
import os
import re
from pathlib import Path

for variable in ("OPENBLAS_NUM_THREADS", "OMP_NUM_THREADS"):
    os.environ[variable] = "2"
import numpy as np
from PIL import Image
from scipy import ndimage

HERE = Path(__file__).parent


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), HERE / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


n1b = load("n1b-score")
probe = load("n1c-cut-probe")
tone, stats = n1b.tone, n1b.stats


def shadow(values, ref, valid):
    partial = valid & (ref > .001) & (ref < .999)
    lit = valid & (ref >= .999)
    umbra = valid & (ref <= .001)
    # N8's lost umbra edges: reference umbra within 3 px of a lit/partial receiver,
    # lit by the candidate more than half.
    edge = umbra & ndimage.binary_dilation((valid & ~umbra), structure=np.ones((7, 7), bool))
    return dict(partial=stats((values - ref)[partial]), solid_flips=int((valid & (lit | umbra) & (np.abs(values - ref) > .5)).sum()),
                umbra_leaks=int((umbra & (values > .1)).sum()), lit_darkened=int((lit & (values < .9)).sum()),
                lost_umbra_edges=int((edge & (values > .5)).sum()), umbra_edge_receivers=int(edge.sum()))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("run", type=Path)
    p.add_argument("--package", type=Path, required=True)
    p.add_argument("--film", type=Path)
    p.add_argument("--size", nargs=2, type=int, default=[1920, 1080])
    a = p.parse_args()
    w, h = a.size
    meshes, placements = probe.package(a.package)
    # Placement i draws package mesh placements[i].mesh; its first T primitives are source.
    source = np.array([int(meshes[pl["mesh"]][1]["span"][meshes[pl["mesh"]][1]["refined"] == 0xFFFFFFFF, 1].sum()) // 3 for pl in placements])
    summary = dict(run=str(a.run), configs={})
    timings = json.loads((a.run / "timing.json").read_text()) if (a.run / "timing.json").exists() else []
    for path in sorted(a.run.glob("p*-lod*.hit")):
        label = path.stem
        pose = int(label[1:].split("-", 1)[0])
        control = a.run / f"p{pose}-Traced"
        if not control.with_suffix(".hit").exists():
            continue
        lod = np.fromfile(path, "<u4").reshape(h, w, 4)
        traced = np.fromfile(control.with_suffix(".hit"), "<u4").reshape(h, w, 4)
        lc, tc = lod[..., 0] != 0, traced[..., 0] != 0
        common = lc & tc
        lod_hit = np.zeros((h, w), bool)
        lod_hit[lc] = lod[..., 1][lc] >= source[lod[..., 0][lc] - 1]
        same = common & (lod[..., :2] == traced[..., :2]).all(-1)
        # Coverage changes, and how far each lies from the traced silhouette.
        changed = lc ^ tc
        boundary = tc & ~ndimage.binary_erosion(tc, structure=np.ones((3, 3), bool), border_value=0)
        distance = ndimage.distance_transform_edt(~boundary)
        row = dict(covered=int(lc.sum()), traced_covered=int(tc.sum()), coverage_changes=int(changed.sum()),
                   coverage_change_distance_to_traced_silhouette_px=stats(distance[changed]),
                   coverage_changes_beyond_1px=int((changed & (distance > 1.5)).sum()),
                   lod_triangle_share=float(lod_hit[lc].mean()), same_triangle=int(same.sum()),
                   identity_agreement_on_full_detail_pixels=float(same[common & ~lod_hit].mean()) if (common & ~lod_hit).any() else None)
        rw = np.fromfile(path.with_suffix(".world"), "<f4").reshape(h, w, 4)
        tw = np.fromfile(control.with_suffix(".world"), "<f4").reshape(h, w, 4)
        gap = np.linalg.norm(rw[..., :3].astype(float) - tw[..., :3], axis=-1) * 1000
        row["receiver_gap_mm"] = dict(all=stats(gap[common]), lod=stats(gap[common & lod_hit]), full_detail=stats(gap[common & ~lod_hit]))
        cull = path.with_suffix(".cull.json")
        if cull.exists():
            row["cull"] = json.loads(cull.read_text())
        rv = np.fromfile(path.with_suffix(".counts"), "<u4").reshape(h, w) / 256.
        tv = np.fromfile(control.with_suffix(".counts"), "<u4").reshape(h, w) / 256.
        # Pose 2 returns to pose 0's camera; references exist for poses 0 and 1.
        ref_path = a.run / f"p{pose if pose < 2 else 0}.reference"
        if ref_path.exists():
            ref = np.fromfile(ref_path, "<f4").reshape(-1, 2)[:, 0].reshape(h, w)
            row["shadow_vs_reference"] = dict(lod_virtual=shadow(rv, ref, common), traced_virtual=shadow(tv, ref, common),
                                              lod_virtual_lod_pixels=shadow(rv, ref, common & lod_hit),
                                              traced_virtual_lod_pixels=shadow(tv, ref, common & lod_hit),
                                              reference=str(ref_path.name))
        rh = np.fromfile(path.with_suffix(".hdr"), "<f2").reshape(h, w, 4)[..., :3].astype(float)
        th = np.fromfile(control.with_suffix(".hdr"), "<f2").reshape(h, w, 4)[..., :3].astype(float)
        row["hdr_delta_vs_traced"] = stats(rh - th)
        image, control_image = tone(rh), tone(th)
        Image.fromarray(image).save(a.run / f"{label}.png")
        Image.fromarray(tone(rh, 4)).save(a.run / f"{label}-plus4.png")
        Image.fromarray(np.clip(np.abs(image.astype(float) - control_image) * 16, 0, 255).astype("u1")).save(a.run / f"{label}-difference-x16.png")
        if not (a.run / f"p{pose}-Traced.png").exists():
            Image.fromarray(control_image).save(a.run / f"p{pose}-Traced.png")
        film = a.film / f"quarry-p{pose if pose < 2 else 0}-film.png" if a.film else None
        if film is not None and film.exists():
            f = np.asarray(Image.open(film).convert("RGB"), float)
            row["film_codes"] = dict(lod=stats(image.astype(float) - f), traced=stats(control_image.astype(float) - f), reference=str(film))
        # Crops for inspection: most coverage changes and most shadow change.
        crops = []
        for name, weight in (("coverage", changed.astype(float)), ("shadow", np.abs(rv - tv) * common)):
            box = ndimage.uniform_filter(weight, 96)
            y, x = np.unravel_index(int(np.argmax(box)), box.shape)
            y0, x0 = int(np.clip(y - 48, 0, h - 96)), int(np.clip(x - 48, 0, w - 96))
            pair = np.concatenate([control_image[y0:y0 + 96, x0:x0 + 96], image[y0:y0 + 96, x0:x0 + 96]], 1)
            Image.fromarray(pair).resize((pair.shape[1] * 4, pair.shape[0] * 4), Image.NEAREST).save(a.run / f"{label}-crop-{name}.png")
            crops.append(dict(kind=name, x=x0, y=y0, traced_left_lod_right=f"{label}-crop-{name}.png"))
        row["crops"] = crops
        rows = [t for t in timings if t["label"] == label]
        if rows:
            row["timing_ms"] = {k: float(np.median([t[k] for t in rows if t.get(k) is not None])) for k in ("primary_ms", "shadow_ms", "span_ms")}
        summary["configs"][label] = row
    (a.run / "summary.json").write_text(json.dumps(summary, indent=2, allow_nan=False))
    for label, row in summary["configs"].items():
        s = row.get("shadow_vs_reference", {})
        lv, tv_ = s.get("lod_virtual", {}), s.get("traced_virtual", {})
        print(label, f"cov_changes={row['coverage_changes']} beyond1px={row['coverage_changes_beyond_1px']} lod_share={row['lod_triangle_share']:.3f}",
              f"gap_mm_p99/max={row['receiver_gap_mm']['all'].get('p99_abs', 0):.3f}/{row['receiver_gap_mm']['all'].get('max_abs', 0):.3f}",
              f"partial={lv.get('partial', {}).get('rms', float('nan')):.4f}/{tv_.get('partial', {}).get('rms', float('nan')):.4f}",
              f"flips={lv.get('solid_flips')}/{tv_.get('solid_flips')} leaks={lv.get('umbra_leaks')}/{tv_.get('umbra_leaks')}",
              f"litdark={lv.get('lit_darkened')}/{tv_.get('lit_darkened')} lost_edges={lv.get('lost_umbra_edges')}/{tv_.get('lost_umbra_edges')}",
              f"hdr_rms={row['hdr_delta_vs_traced']['rms']:.5f}",
              f"film={row.get('film_codes', {}).get('lod', {}).get('rms', float('nan')):.4f}/{row.get('film_codes', {}).get('traced', {}).get('rms', float('nan')):.4f}")


if __name__ == "__main__":
    main()
