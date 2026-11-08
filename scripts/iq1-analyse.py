"""IQ1 (docs/iq1-quarry-image.md): CPU analysis of `gpu_iq1_quarry` dumps against U0b's
4,096-spp full-radiance quarry references and their registered populations.

  py -3 -B scripts/iq1-analyse.py <dump dir> <out json> [image ...]
  py -3 -B scripts/iq1-analyse.py qwalk <gpu_tr1_sequence data dir> <out json>

<dump dir> holds `<path>-<variant>-pose<k>.f32` (raw exposed RGB) and
`<path>-pose<k>-hits.u32x4` from one or more paths. Each image name is `<path>-<variant>`
(radiance variants: standard, light05, nomap, flat, jitterN, or any other radiance
dump); the default scores every standard and jitter image found.

Per image and pose: F0 codes (U0b's Reinhard+sRGB transform) at -14 and -10 stops, RMS per
population as U0b's floor tables; the high-frequency error (RMS of the difference of
3x3 Laplacians of the luminance codes, inside each population's 3x3-interior pixels);
the Laplacian energy ratio against the reference, with the reference's own noise
(split halves) removed; and pinholes (see `pinholes`). CPU only, one process.
"""
import importlib.util
import json
import sys
from pathlib import Path

import numpy as np

REPO = Path(__file__).resolve().parent.parent
U0B = Path("E:/renderlab-work/m7-u0b/reference/quarry-final")
H, W = 1080, 1920
EXPOSURE = -14.0
spec = importlib.util.spec_from_file_location("u0b_score", REPO / "scripts/m7-u0b-score.py")
sc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sc)


def load(path):
    a = np.fromfile(path, "<f4")
    assert a.size == H * W * 3, (path, a.size)
    return a.reshape(H, W, 3)


def radiance(path):
    return load(path) * 2.0 ** -EXPOSURE


def hits(path):
    return np.fromfile(path, "<u4").reshape(H, W, 4)


def luminance_codes(x, stops=EXPOSURE):
    return sc.codes(x, stops).mean(-1)


def neighbours(x):
    p = np.pad(x, 1, mode="edge")
    return np.stack([p[1 + dy:1 + dy + H, 1 + dx:1 + dx + W] for dy in (-1, 0, 1) for dx in (-1, 0, 1) if dy or dx])


def laplacian(x):
    p = np.pad(x, 1, mode="edge")
    return 4 * x - p[:-2, 1:-1] - p[2:, 1:-1] - p[1:-1, :-2] - p[1:-1, 2:]


def interior(mask):
    return mask & (neighbours(mask.astype(np.uint8)).min(0) > 0)


def pinholes(cand, ref, covered):
    """Single dark pixels the reference does not have: every neighbour covered, the pixel
    below 0.4x the median of its 8 neighbours (luminance codes at -14, median above 15),
    below half the reference there, while the reference is not itself locally dark
    (at least 0.6x its own neighbours' median)."""
    l, r = luminance_codes(cand), luminance_codes(ref)
    ml, mr = np.median(neighbours(l), 0), np.median(neighbours(r), 0)
    return interior(covered) & (ml > 15) & (l < 0.4 * ml) & (r >= 0.6 * mr) & (l < 0.5 * r)


def scores(cand, ref, half, pops):
    out = {}
    for k in (0, 4):
        error = sc.codes(cand, EXPOSURE + k) - sc.codes(ref, EXPOSURE + k)
        e2 = (error ** 2).mean(-1)
        out[f"rms{k}"] = {n: float(np.sqrt(e2[m].mean())) for n, m in pops.items()}
    lc, lr = luminance_codes(cand), luminance_codes(ref)
    lap_c, lap_r = laplacian(lc), laplacian(lr)
    # The reference's own Laplacian noise, from its split halves (L0, L1 = ref +- half).
    lap_noise = (laplacian(luminance_codes(ref + half)) - laplacian(luminance_codes(ref - half))) / 2
    hf = {}
    for n, m in pops.items():
        inner = interior(m)
        if inner.sum() < 100:
            continue
        noise2 = float((lap_noise[inner] ** 2).mean())
        energy_r = max(float((lap_r[inner] ** 2).mean()) - noise2, 1e-9)
        hf[n] = dict(n=int(inner.sum()),
                     hf_error=float(np.sqrt(max(float(((lap_c - lap_r)[inner] ** 2).mean()) - noise2, 0.0))),
                     energy_ratio=float(np.sqrt(float((lap_c[inner] ** 2).mean()) / energy_r)))
    out["hf"] = hf
    return out


def qwalk(data, out):
    """`gpu_tr1_sequence` frames (<data>/<variant>/fNNN.rgba16f, exposed at -14) against
    U0b's Q-walk reference of the same frame (q-walk-NNN.npz)."""
    result = {}
    for variant in sorted(p for p in data.iterdir() if p.is_dir()):
        rows = {}
        for f in sorted(variant.glob("f*.rgba16f")):
            frame = int(f.stem[1:])
            archive = U0B / f"q-walk-{frame:03}.npz"
            if not archive.exists():
                continue
            hdr = np.fromfile(f, "<f2").reshape(H, W, 4)[..., :3].astype("f4") * 2.0 ** -EXPOSURE
            with np.load(archive) as r:
                ref = (r["L0"] + r["L1"]) / 2
                half = (r["L0"] - r["L1"]) / 2
                pops = {k[4:]: r[k] for k in r.files if k.startswith("pop_") and r[k].any()}
            row = scores(hdr, ref, half, pops)
            row["pinholes"] = int(pinholes(hdr, ref, pops["all-covered"]).sum())
            rows[frame] = row
            print(variant.name, frame, "pinholes", row["pinholes"], "all-covered rms",
                  round(row["rms0"]["all-covered"], 3), "hf", round(row["hf"]["all-covered"]["hf_error"], 3))
        result[variant.name] = rows
    out.write_text(json.dumps(result, indent=1))


def main():
    if sys.argv[1] == "qwalk":
        return qwalk(Path(sys.argv[2]), Path(sys.argv[3]))
    dump, out = Path(sys.argv[1]), Path(sys.argv[2])
    names = sys.argv[3:] or sorted({p.name.rsplit("-pose", 1)[0] for p in dump.glob("*-pose*.f32")
                                     if "-standard-" in p.name or "-jitter" in p.name})
    result = dict(scope="IQ1 quarry analysis; F0 codes at -14/-10 against U0b's 4,096-spp archives", poses={})
    for pose in (0, 1):
        archive = U0B / f"quarry-p{pose}.npz"
        if not archive.exists():
            continue
        with np.load(archive) as r:
            ref = (r["L0"] + r["L1"]) / 2
            half = (r["L0"] - r["L1"]) / 2
            pops = {k[4:]: r[k] for k in r.files if k.startswith("pop_") and r[k].any()}
        rows = {}
        for name in names:
            f = dump / f"{name}-pose{pose}.f32"
            if not f.exists():
                continue
            cand = radiance(f)
            path = name.split("-")[0]
            hit_file = dump / f"{path}-pose{pose}-hits.u32x4"
            covered = hits(hit_file)[..., 0] != 0 if hit_file.exists() else cand.sum(-1) > 0
            row = scores(cand, ref, half, pops)
            mask = pinholes(cand, ref, covered)
            row["pinholes"] = int(mask.sum())
            row["pinholes_by_population"] = {n: int((mask & m).sum()) for n, m in pops.items()}
            rows[name] = row
            print(pose, name, "pinholes", row["pinholes"], "all-covered rms", round(row["rms0"]["all-covered"], 3),
                  "hf", round(row["hf"]["all-covered"]["hf_error"], 3), "energy", round(row["hf"]["all-covered"]["energy_ratio"], 3))
        result["poses"][f"p{pose}"] = rows
    out.write_text(json.dumps(result, indent=1))


if __name__ == "__main__":
    main()
