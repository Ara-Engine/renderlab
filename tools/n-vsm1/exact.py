"""N-VSM2: the exact answer the Disk lookup approximates, on the CPU, for the S2/S3 benches.

For each crop pixel of one bench frame, rays from the reference's centre receiver point toward
sun-disk samples against the scene's triangles (S2 has 32, S3 a few thousand):
  exact64   Disk's own 64 rook samples in the map's basis: what the lookup would return if the map
            were the geometry. Disk minus exact64 is the map's error alone.
  exactN    N stratified samples: the true point visibility. exact64 minus exactN is the pattern's
            error; exactN minus the film reference is what any centre-point lookup scores (the
            film integrates the pixel footprint).

    from exact import Frame; f = Frame("S2", "shadow-step-r0.04", 16); f.exact(64); f.exact(1024)
"""
import json
import math
from pathlib import Path
import re
import sys

import numpy as np

REPO = Path(__file__).resolve().parents[2]
REFS = Path("D:/Coding/renderlab-work/n-vsm1/bench/refs")
sys.path[:0] = [str(REPO / "support"), str(REPO / "tools/n-vsm1")]


def rook():
    """The lookup's 64 samples in the unit disk (virtual_shadow_gpu.rs DISK_Y_RANK)."""
    import pattern
    source = (REPO / "src/virtual_shadow_gpu.rs").read_bytes().decode("utf-8")
    rank = [int(x) for x in re.search(r"DISK_Y_RANK: \[u8; 64\] = \[(.*?)\];", source, re.S).group(1).split(",") if x.strip()]
    q = np.array(pattern.Q)
    return np.stack([q, q[rank]], 1)


def stratified(n, seed=7):
    """n samples uniform over the unit disk (jittered grid, concentric map)."""
    side = int(round(math.sqrt(n)))
    rng = np.random.default_rng(seed)
    a = (np.stack(np.meshgrid(np.arange(side), np.arange(side)), -1).reshape(-1, 2) + rng.random((side * side, 2))) / side * 2 - 1
    r = np.where(np.abs(a[:, 0]) > np.abs(a[:, 1]), a[:, 0], a[:, 1])
    phi = np.where(np.abs(a[:, 0]) > np.abs(a[:, 1]), np.pi / 4 * a[:, 1] / np.where(a[:, 0] == 0, 1, a[:, 0]),
                   np.pi / 2 - np.pi / 4 * a[:, 0] / np.where(a[:, 1] == 0, 1, a[:, 1]))
    return np.stack([r * np.cos(phi), r * np.sin(phi)], 1)


def basis(toward):
    """virtual_shadow_gpu.rs basis()."""
    l = toward / np.linalg.norm(toward)
    up = np.array([0.0, 1.0, 0.0]) if abs(l[1]) < 0.999 else np.array([1.0, 0.0, 0.0])
    u = np.cross(up, l)
    u /= np.linalg.norm(u)
    return u, np.cross(l, u), l


class Frame:
    def __init__(self, bench, case, index):
        self.scene = json.loads((REFS / bench / "fixture/scene.json").read_text())
        track = next(t for t in self.scene["tracks"] if case.startswith(t["name"] + "-r"))
        self.pose = track["frames"][index]
        self.radius = float(case.rsplit("-r", 1)[1])
        with np.load(REFS / bench / "renderlab" / case / "reference" / f"pose-{index}.npz") as a:
            self.d = {k: a[k] for k in a.files}
        import fresh_level_b_metrics as metrics
        self.pops = metrics.populations(self.d)
        triangles, thin = [], []
        for mesh in self.scene["meshes"]:
            shift = np.array([self.pose.get({"reveal-plate": "plate_x", "shadow-blocker": "blocker_x"}.get(mesh["name"], ""), 0.0), 0.0, 0.0])
            p = np.array(mesh["positions"]) + shift
            idx = np.array(mesh["indices"]).reshape(-1, 3)
            triangles.append(p[idx])
            thin += [mesh["thin"]] * len(idx)
        self.triangles = np.concatenate(triangles)
        self.u, self.v, self.l = basis(np.array(self.pose["to_light"]))

    def exact(self, samples=64, offset=1e-4):
        """Visible share per crop pixel (float32, NaN where the reference has no receiver)."""
        g = rook() if samples == 64 else stratified(samples)
        tan = math.tan(self.radius)
        dirs = self.l + tan * (g[:, :1] * self.u + g[:, 1:] * self.v)
        world = self.d["world"][..., :3].astype("f8")
        cover = self.d["cover"]
        origin = world[cover] + offset * self.l
        blocked = np.zeros((len(origin), len(dirs)), bool)
        a, b, c = self.triangles[:, 0], self.triangles[:, 1], self.triangles[:, 2]
        e1, e2 = b - a, c - a
        for k, d in enumerate(dirs):
            # Moller-Trumbore, every receiver against every triangle, in chunks of triangles.
            h = np.cross(d, e2)
            det = np.einsum("ij,ij->i", e1, h)
            for lo in range(0, len(a), 256):
                s = slice(lo, lo + 256)
                ok = np.abs(det[s]) > 1e-14
                inv = np.where(ok, 1.0 / np.where(ok, det[s], 1.0), 0.0)
                t0 = origin[:, None, :] - a[s][None]
                uu = np.einsum("ntj,tj->nt", t0, h[s]) * inv
                q = np.cross(t0, e1[s][None])
                vv = (q @ d) * inv
                tt = np.einsum("ntj,tj->nt", q, e2[s]) * inv
                blocked[:, k] |= (ok & (uu >= 0) & (vv >= 0) & (uu + vv <= 1) & (tt > 0)).any(1)
        out = np.full(cover.shape, np.nan, "f4")
        out[cover] = 1.0 - blocked.mean(1)
        return out
