"""Motion-AA CPU simulation (docs/m6-motion-aa.md): resampled pixel history against point-sample reuse.

The world is R1's pose-0 quarry frame as a point cloud: in every pixel, the 16 saved jittered samples
(full HDR, sun-only HDR and Exact visibility; load_pose0) plus the Exact centre sample, each at its
exact sub-pixel position. Under a uniform image translation the
cliff layer stays fixed in pose-0 coordinates while the camera's pixel grid moves by v_c per frame and
the (uniform) background moves by v_b. Every fresh sample is one of the cloud's true samples inside
the current footprint, so no value is ever interpolated. At frames where t * v_c is integral the grid
coincides with pose 0's, and every pixel is scored against R1's independent 240-stratum complement
reference; the background is a constant colour, so its motion does not change the reference.

Methods:
  t1b      - T1b's per-pixel history: Catmull-Rom over 4x4 when all taps pass the forward (surface)
             test, else renormalised bilinear over valid inner taps, else a centre restart; window
             W = 1 + 1 / sum f (1 - f).
  t1b-bit  - t1b plus a mixed-history bit that background samples may not accept.
  pK-*     - K world-anchored sample records per pixel. Records move as points; a pixel keeps the
             K most recent records whose points lie inside its footprint, plus nothing else. Background
             records hidden by the cliff are dropped (nearest-cloud-point coverage stands in for an
             exact camera-ray test). The fresh sample follows T1's lattice ('lattice') or fills the
             largest gap ('gap'); 'cells' averages per 2x2 sub-cell before averaging the cells.

Not modelled: parallax inside the cliff (folds), view-dependent shading, light or object motion.

py -3 -B scripts/m6-motion-aa-sim.py --out E:/renderlab-work/motion-aa
Math-library threads should be set to two.
"""
import argparse
import importlib.util
import json
import time
from pathlib import Path

import numpy as np
from PIL import Image
from scipy import ndimage

spec = importlib.util.spec_from_file_location("replay", Path(__file__).with_name("m6-temporal-replay.py"))
rp = importlib.util.module_from_spec(spec); spec.loader.exec_module(rp)

H, W = rp.H, rp.W
SAVED = [0, 23, 46, 53, 76, 83, 106, 113, 136, 159, 166, 189, 196, 219, 226, 249]
M32 = 0xFFFFFFFF
POPS = rp.REPORT_POPS + ("edge", "bg-d0-1.5", "bg-d1.5-3", "bg-d3-6", "bg-d6+")


def fresh_hash(x):
    x &= M32; x = ((x ^ (x >> 16)) * 0x7feb352d) & M32; x = ((x ^ (x >> 15)) * 0x846ca68b) & M32
    return x ^ (x >> 16)


def r1_offset(s):
    """R1's film_jitter for stratum s: (x, y) inside the pixel (src/film_reference_tests.rs)."""
    h = fresh_hash(s ^ 0x83d2e19b)
    return ((s % 16 + (h >> 8) / 16777216.0) / 16.0, (s // 16 + (fresh_hash(h) >> 8) / 16777216.0) / 16.0)


OFF = np.array([r1_offset(s) for s in SAVED] + [(0.5, 0.5)])  # (17, 2): x, y
NC = len(OFF)


def film_offset(k, s):
    """T1's production sequence position for index k and symmetry bits s (shaders/film_accumulation.wgsl)."""
    k = np.asarray(k) & 0xFF; s = np.asarray(s)
    order = np.where(k < 4, (0x0231 >> (4 * np.minimum(k, 3))) & 0xF, k)
    j = np.zeros_like(order)
    for b in range(8):
        j |= ((order >> b) & 1) << (7 - b)
    px, py = j, (71 * j) & 0xFF
    flip = (s & 1) != 0
    px, py = np.where(flip, (256 - px) & 0xFF, px), np.where(flip, (256 - py) & 0xFF, py)
    sw = (s & 2) != 0
    px, py = np.where(sw, py, px), np.where(sw, px, py)
    x = (((px + 8) & 0xFF) + 0.5) / 256.0; y = (((py + 8) & 0xFF) + 0.5) / 256.0
    return np.where(k == 0, 0.5, x), np.where(k == 0, 0.5, y)


def candidates(phi):
    """At grid offset phi, the 17 cloud points inside each footprint: (ax, ay) pixel steps and positions."""
    ax = (OFF[:, 0] < phi[0]).astype(int); ay = (OFF[:, 1] < phi[1]).astype(int)
    return ax, ay, ax + OFF[:, 0] - phi[0], ay + OFF[:, 1] - phi[1]


class World:
    def __init__(self, data):
        s = data["samples"]; h, w = data["cover"].shape
        self.h, self.w = h, w
        clear = np.median(s["L"][0][~data["cover"]].reshape(-1, 3), 0)
        P = 3
        def pad(a, value):
            width = [(0, 0)] * a.ndim; width[1] = (0, P); width[2] = (0, P)
            return np.pad(a, width, constant_values=value)
        L = np.concatenate([s["L8"], data["centre8"]["L"][None]]).astype(np.float32)
        D = np.concatenate([s["D8"], data["centre8"]["D"][None]]).astype(np.float32)
        V = np.concatenate([s["V8"], data["centre8"]["V"][None]]).astype(np.float32)
        cov = np.concatenate([np.any(np.abs(s["L"] - clear) > 1e-7, -1), data["cover"][None]])
        self.clear = clear.astype(np.float32)
        self.L = np.concatenate([pad(L[..., c], float(clear[c]))[..., None] for c in range(3)], -1)
        self.D = np.concatenate([pad(D[..., c], 0.0)[..., None] for c in range(3)], -1)
        self.V = pad(V, np.nan); self.cov = pad(cov, False)
        self.Dsky = np.median(s["D"][0][~data["cover"]].reshape(-1, 3), 0).astype(np.float32)
        # Nearest-cloud-point lookup over a 256x256 sub-pixel grid (offsets repeat in every pixel).
        g = (np.arange(256) + 0.5) / 256
        gx, gy = np.meshgrid(g, g)
        best = np.full(gx.shape, np.inf); self.lut = np.zeros(gx.shape + (3,), np.int64)
        for dy in (-1, 0, 1):
            for dx in (-1, 0, 1):
                for i in range(NC):
                    d = (gx - dx - OFF[i, 0]) ** 2 + (gy - dy - OFF[i, 1]) ** 2
                    m = d < best; best[m] = d[m]
                    self.lut[m] = (dx, dy, i)

    def gather(self, a, i, ax, ay):
        """Per-pixel values of cloud point i of pose-0 pixel (q + (ax, ay)) for every label q."""
        return a[i, ay:ay + self.h, ax:ax + self.w]

    def covered(self, x, y):
        """Cliff coverage at points (x, y): the nearest cloud point's coverage."""
        fx, fy = np.floor(x).astype(np.int64), np.floor(y).astype(np.int64)
        lx = np.clip(((x - fx) * 256).astype(np.int64), 0, 255); ly = np.clip(((y - fy) * 256).astype(np.int64), 0, 255)
        d = self.lut[ly, lx]
        X = fx + d[:, 0]; Y = fy + d[:, 1]
        ok = (X >= 0) & (Y >= 0) & (X < self.w + 3) & (Y < self.h + 3)
        out = np.zeros(x.shape, bool)
        out[ok] = self.cov[d[ok, 2], Y[ok], X[ok]]
        return out


class Scorer:
    def __init__(self, data, origin):
        self.pops = dict(data["pops"]); c = data["cover"]
        hit = data["ref"]["hit"][..., 0][origin]
        p = np.pad(hit, 1, mode="edge"); e = np.zeros(hit.shape, bool)
        for dy in (-1, 0, 1):
            for dx in (-1, 0, 1):
                e |= p[1 + dy:1 + dy + hit.shape[0], 1 + dx:1 + dx + hit.shape[1]] != hit
        self.pops["edge"] = e
        dist = ndimage.distance_transform_edt(~c)
        bg = ~c
        for name, lo, hi in (("bg-d0-1.5", 0, 1.5), ("bg-d1.5-3", 1.5, 3), ("bg-d3-6", 3, 6), ("bg-d6+", 6, 1e9)):
            self.pops[name] = bg & (dist > lo) & (dist <= hi)
        n = data["ref"]["n"][origin]
        self.single = c & (n >= 256) & np.isfinite(data["comp"]["F"])
        # Verified solids: the 240-stratum complement and all 17 cloud points agree exactly (as
        # T1a's replica check, R1's 256-sample solids include partial footprints).
        F = data["comp"]["F"]; Vc = data["cloudV"]
        agree1 = np.all(np.isfinite(Vc) & (Vc == 1), 0); agree0 = np.all(np.isfinite(Vc) & (Vc == 0), 0)
        self.pops["solid-lit-v"] = self.single & (F == 1) & agree1
        self.pops["solid-occ-v"] = self.single & (F == 0) & agree0
        self.comp = data["comp"]
        self.ref_codes = {(ch, st): rp.codes(self.comp[ch], st) for ch in ("L", "D") for st in (0, 4)}
        self.cover = c

    def score(self, L, D, V):
        out = {}
        for ch, est in (("L", L), ("D", D)):
            for st in (0, 4):
                e = rp.codes(est, st) - self.ref_codes[(ch, st)]
                e2 = np.mean(e ** 2, -1); sg = np.mean(e, -1)
                row = {}
                for name in POPS:
                    m = self.pops[name]; k = int(m.sum())
                    if k:
                        row[name] = dict(n=k, rms=float(np.sqrt(e2[m].mean())), mean=float(sg[m].mean()),
                                         se=float(sg[m].std() / np.sqrt(k)), max=float(np.sqrt(e2[m].max())))
                out[f"{ch}{st}"] = row
                if ch == "D" and st == 4:
                    out["D4_over16"] = int(((np.sqrt(e2) > 16) & self.cover).sum())
        ok = self.single & np.isfinite(V)
        ev = np.where(ok, np.nan_to_num(V) - self.comp["F"], 0.0)
        row = {}
        for name in rp.REPORT_POPS + ("edge", "solid-lit-v", "solid-occ-v"):
            m = self.pops[name] & ok; k = int(m.sum())
            if k and name not in ("background-centre", "image"):
                row[name] = dict(n=k, rms=float(np.sqrt(np.mean(ev[m] ** 2))), mean=float(ev[m].mean()),
                                 se=float(ev[m].std() / np.sqrt(k)), max=float(np.abs(ev[m]).max()),
                                 over_002=int((np.abs(ev[m]) > 0.002).sum()))
        row["missing"] = int((self.single & ~np.isfinite(V)).sum())
        out["F"] = row
        F = self.comp["F"]
        out["missed"] = int((ok & (F <= .9) & (V >= .99)).sum()); out["false"] = int((ok & (F >= .99) & (V <= .9)).sum())
        return out


# ------------------------------------------------------------------ T1b-style resampled history

def cr_weights(t):
    t2 = t * t
    return 0.5 * np.array([t * (t * (2 - t) - 1), t2 * (3 * t - 5) + 2, t * (t * (-3 * t + 4) + 1), t2 * (t - 1)])


class Resampled:
    """Per-pixel mean history; anchor 0 = cliff, 1 = background, 2 = none. mixed: bit 1 cliff, bit 2 background."""

    def __init__(self, world, data, bit, sym):
        h, w = world.h, world.w
        self.world, self.bit, self.sym = world, bit, sym
        c = data["centre8"]
        self.mL = c["L"].astype(np.float64); self.mD = c["D"].astype(np.float64)
        self.mV = np.nan_to_num(c["V"]).astype(np.float64); self.nV = np.isfinite(c["V"]).astype(np.float64)
        self.n = np.ones((h, w)); cov = data["cover"]
        self.anchor = np.where(cov, 0, 1).astype(np.int8); self.mixed = np.where(cov, 1, 2).astype(np.int8)
        yy, xx = np.mgrid[0:h, 0:w]
        # The latest sample's point (pose-0 coordinates), for T1b's reverse test.
        self.apx = xx + 0.5; self.apy = yy + 0.5
        self.k = np.zeros((h, w), np.int64); self.age = np.zeros((h, w), np.int64); self.resets = 0
        self.reverse_accepts = 0

    def estimate(self):
        return self.mL, self.mD, np.where(self.nV > 0, self.mV, np.nan)

    def _taps(self, base):
        P = 4; h, w = self.world.h, self.world.w
        arrs = dict(mL=self.mL, mD=self.mD, mV=self.mV, nV=self.nV, n=self.n, anchor=self.anchor, mixed=self.mixed,
                    apx=self.apx, apy=self.apy)
        fill = dict(mL=0, mD=0, mV=0, nV=0, n=0, anchor=2, mixed=0, apx=-1e9, apy=-1e9)
        padded = {k: np.pad(v, [(P, P), (P, P)] + [(0, 0)] * (v.ndim - 2), constant_values=fill[k]) for k, v in arrs.items()}
        taps = {}
        for dy in (-1, 0, 1, 2):
            for dx in (-1, 0, 1, 2):
                y0 = P + base[1] + dy; x0 = P + base[0] + dx
                taps[(dx, dy)] = {k: v[y0:y0 + h, x0:x0 + w] for k, v in padded.items()}
        return taps

    def _fetch(self, o, layer, now_cov, phi):
        base = np.floor(o).astype(int); f = o - base
        taps = self._taps(base)
        h, w = self.world.h, self.world.w
        yy, xx = np.mgrid[0:h, 0:w]
        valid = {}; self._rev = 0
        for key, t in taps.items():
            v = t["anchor"] == layer
            if layer == 1:
                # Reverse test: a geometry anchor whose point projects within 1.5 px of this pixel's
                # centre, onto a pixel whose current sample hit that geometry (2D: the cliff).
                dx = t["apx"] - (xx + phi[0] + 0.5); dy = t["apy"] - (yy + phi[1] + 0.5)
                near = (t["anchor"] == 0) & (dx * dx + dy * dy <= 2.25)
                lx = np.clip(np.floor(t["apx"] - phi[0]).astype(np.int64), 0, w - 1)
                ly = np.clip(np.floor(t["apy"] - phi[1]).astype(np.int64), 0, h - 1)
                rev = near & now_cov[ly, lx]
                if key[0] in (0, 1) and key[1] in (0, 1):
                    self._rev += int(rev.sum())
                v = v | rev
                if self.bit:
                    v &= t["mixed"] != 3
            valid[key] = v
        wx = (1 - f[0], f[0]); wy = (1 - f[1], f[1])
        wsum = 0; cL = 0; cD = 0; n = 0; vw = 0; cV = 0; nV = 0; cnt = 0; mixed = np.zeros(self.n.shape, np.int8)
        for dy in (0, 1):
            for dx in (0, 1):
                t = taps[(dx, dy)]; v = valid[(dx, dy)]; ww = wx[dx] * wy[dy] * v
                wsum = wsum + ww; cL = cL + ww[..., None] * t["mL"]; cD = cD + ww[..., None] * t["mD"]
                n = n + ww * t["n"]; cnt = cnt + v
                wv = ww * (t["nV"] > 0); vw = vw + wv; cV = cV + wv * t["mV"]; nV = nV + ww * t["nV"]
                mixed |= np.where(v & (ww > 0), t["mixed"], 0).astype(np.int8)
        ok = wsum > 0
        cL = cL / np.maximum(wsum, 1e-300)[..., None]; cD = cD / np.maximum(wsum, 1e-300)[..., None]
        cV = cV / np.maximum(vw, 1e-300)
        all16 = np.ones(self.n.shape, bool)
        for v in valid.values():
            all16 &= v
        cx, cy = cr_weights(f[0]), cr_weights(f[1])
        crL = 0; crD = 0; crV = 0; crVw = 0
        for (dx, dy), t in taps.items():
            ww = cx[dx + 1] * cy[dy + 1]
            crL = crL + ww * t["mL"]; crD = crD + ww * t["mD"]
            crV = crV + ww * t["mV"] * (t["nV"] > 0); crVw = crVw + ww * (t["nV"] > 0)
        use = all16 & ok
        cL = np.where(use[..., None], np.maximum(crL, 0), cL); cD = np.where(use[..., None], np.maximum(crD, 0), cD)
        allV = use & (np.abs(crVw - 1) < 1e-9)
        cV = np.where(allV, np.clip(crV, 0, None), cV)
        blur = f[0] * (1 - f[0]) + f[1] * (1 - f[1])
        window = 256.0 if blur <= 0 else min(max(1 + 1 / blur, 1), 256)
        return dict(ok=ok, L=cL, D=cD, V=cV, vw=vw, n=n, nV=nV, mixed=mixed, window=window, catmull=use)

    def step(self, t, motion, sample):
        h, w = self.world.h, self.world.w
        j = motion.jump(t); phi = motion.phi(t)
        cov = sample["cov"]
        fc = self._fetch(motion.vc - j, 0, cov, phi); fs = self._fetch(motion.vb - j, 1, cov, phi)
        ok = np.where(cov, fc["ok"], fs["ok"])
        # Restarted pixels trace again at the pixel centre with index 0 (T1b's cold-pixel rule).
        centre = motion.centre_sample(t)
        x = {k: np.where((ok if sample[k].ndim == 2 else ok[..., None]), sample[k], centre[k]) for k in ("L", "D", "V", "x", "y")}
        self.reverse_accepts += self._rev
        cov = np.where(ok, cov, centre["cov"])
        # A restarted pixel's centre sample may lie on the other layer; that does not change ok (no history).
        win = np.where(cov, fc["window"], fs["window"])
        F = {k: np.where(cov.reshape(cov.shape + (1,) * (fc[k].ndim - 2)), fc[k], fs[k]) for k in ("L", "D", "V", "vw", "n", "nV", "mixed")}
        n1 = np.where(ok, np.minimum(F["n"] + 1, win), 1.0)
        prior_L = np.where(ok[..., None], F["L"], 0); prior_D = np.where(ok[..., None], F["D"], 0)
        self.mL = prior_L + (x["L"] - prior_L) / n1[..., None]; self.mD = prior_D + (x["D"] - prior_D) / n1[..., None]
        vfin = np.isfinite(x["V"]); hasV = ok & (F["vw"] > 0)
        nV1 = np.where(vfin, np.where(hasV, np.minimum(F["nV"] + 1, win), 1.0), np.where(hasV, F["nV"], 0.0))
        pV = np.where(hasV, F["V"], 0.0)
        self.mV = np.where(vfin, pV + (np.nan_to_num(x["V"]) - pV) / np.maximum(nV1, 1e-300), pV); self.nV = nV1
        self.n = n1
        layer = np.where(cov, 0, 1).astype(np.int8)
        self.anchor = layer
        self.apx = np.where(cov, x["x"], self.apx); self.apy = np.where(cov, x["y"], self.apy)
        self.mixed = np.where(ok, F["mixed"], 0).astype(np.int8) | np.where(cov, 1, 2).astype(np.int8)
        self.resets += int((~ok).sum())
        # The sample just taken used index k + 1; a restarted pixel's centre sample is index 0.
        self.k = np.where(ok, (self.k + 1) & 0xFF, 0); self.age = np.where(ok, self.age + 1, 0)
        self.catmull_share = float(np.mean(np.where(cov, fc["catmull"], fs["catmull"])[ok])) if ok.any() else 0.0

    def next_k(self):
        return (self.k + 1) & 0xFF


# ------------------------------------------------------------------ point-sample reuse

class Points:
    """World-anchored sample records: positions in pose-0 (cliff) coordinates; background records drift."""

    def __init__(self, world, data, K, choice, cells, sym, nn=False):
        self.world, self.K, self.choice, self.cells, self.sym, self.nn = world, K, choice, cells, sym, nn
        # Silhouette band (pose-0 coordinates are the cliff's, so the band is static): only background
        # records here can be hidden by the cliff.
        c = data["cover"]
        self.band = ndimage.binary_dilation(c, iterations=3) & ndimage.binary_dilation(~c, iterations=3)
        self.tested = []
        h, w = world.h, world.w
        yy, xx = np.mgrid[0:h, 0:w]
        c = data["centre8"]
        self.x = (xx + 0.5).ravel().astype(np.float64); self.y = (yy + 0.5).ravel().astype(np.float64)
        self.L = c["L"].reshape(-1, 3).astype(np.float32); self.D = c["D"].reshape(-1, 3).astype(np.float32)
        self.V = c["V"].ravel().astype(np.float32); self.cov = data["cover"].ravel().copy()
        self.born = np.zeros(h * w, np.int32)
        self.cid = (((yy * (w + 3) + xx) * NC) + (NC - 1)).ravel().astype(np.int64)
        self.labels = (yy * w + xx).ravel()
        self.counts = []; self.dropped_hidden = 0

    def _label(self, phi):
        lx = np.floor(self.x - phi[0]).astype(np.int64); ly = np.floor(self.y - phi[1]).astype(np.int64)
        ok = (lx >= 0) & (ly >= 0) & (lx < self.world.w) & (ly < self.world.h)
        return ok, ly * self.world.w + lx

    def _keep(self, idx):
        for k in ("x", "y", "L", "D", "V", "cov", "born", "cid", "labels"):
            setattr(self, k, getattr(self, k)[idx])

    def step(self, t, motion, sample_choice):
        """Move records, trace the fresh sample, validate background records against the frame's
        coverage evidence, then keep the K - 1 most recent records plus the fresh sample."""
        world = self.world; h, w = world.h, world.w; phi = motion.phi(t)
        sky = ~self.cov
        self.x[sky] += motion.delta[0]; self.y[sky] += motion.delta[1]
        ok, lab = self._label(phi)
        self.labels = lab; self._keep(np.nonzero(ok)[0])
        order = np.lexsort((-self.born, self.labels)); self._keep(order)
        # Fresh sample: one cloud point inside each footprint, not already held.
        ax, ay, px, py = candidates(phi)
        held = np.zeros((NC, h * w), bool)
        qy, qx = np.divmod(self.labels, w)
        for i in range(NC):
            cid_i = ((qy + ay[i]) * (w + 3) + qx + ax[i]) * NC + i
            m = self.cid == cid_i
            held[i, self.labels[m]] = True
        if self.choice == "gap" and self.labels.size:
            fx = self.x - qx - phi[0]; fy = self.y - qy - phi[1]
            dmin = np.full((NC, h * w), np.inf)
            first = np.r_[0, np.nonzero(np.diff(self.labels))[0] + 1]; labs = self.labels[first]
            for i in range(NC):
                d = (fx - px[i]) ** 2 + (fy - py[i]) ** 2
                dmin[i, labs] = np.minimum.reduceat(d, first)
            dmin[held] = -1
            # Pixels holding nothing take the candidate nearest the centre.
            empty = np.all(~np.isfinite(dmin) | (dmin < 0), 0) & ~held.any(0)
            choice = np.argmax(np.where(np.isfinite(dmin), dmin, -2), 0)
            centre_i = int(np.argmin((px - .5) ** 2 + (py - .5) ** 2))
            choice[empty] = centre_i
        else:
            tx, ty = film_offset(np.full(h * w, t), self.sym.ravel())
            d = (tx[None] - px[:, None]) ** 2 + (ty[None] - py[:, None]) ** 2
            d[held] = np.inf
            choice = np.argmin(d, 0)
        new = motion.sample_at(choice.reshape(h, w), phi)
        yy, xx = np.mgrid[0:h, 0:w]
        ci = choice.reshape(h, w)
        nx = (xx + ax[ci] + OFF[ci, 0]).ravel(); ny = (yy + ay[ci] + OFF[ci, 1]).ravel()
        cid = (((yy + ay[ci]) * (w + 3) + xx + ax[ci]) * NC + ci).ravel()
        # Validate background records in the silhouette band.
        sky = ~self.cov
        bx = np.clip(np.floor(self.x).astype(np.int64), 0, w - 1); by = np.clip(np.floor(self.y).astype(np.int64), 0, h - 1)
        test = sky & self.band[by, bx]
        self.tested.append(int(test.sum()))
        hidden = np.zeros(self.x.size, bool)
        if self.nn and test.any():
            # Realizable test: the nearest coverage evidence decides. Evidence is every geometry
            # record (front geometry stays visible in a static scene) and this frame's fresh
            # samples; a background record whose nearest evidence is geometry is hidden.
            from scipy.spatial import cKDTree
            fb = self.band[np.clip(ny.astype(np.int64), 0, h - 1), np.clip(nx.astype(np.int64), 0, w - 1)]
            gi = np.nonzero(self.cov & self.band[by, bx])[0]
            ex = np.r_[self.x[gi], nx[fb]]; ey = np.r_[self.y[gi], ny[fb]]
            ecov = np.r_[np.ones(gi.size, bool), new["cov"].ravel()[fb]]
            if ex.size:
                q = np.nonzero(test)[0]
                _, nb = cKDTree(np.c_[ex, ey]).query(np.c_[self.x[q], self.y[q]], k=1)
                hidden[q] = ecov[nb]
        elif test.any():
            hidden = test & world.covered(self.x, self.y)
        self.dropped_hidden += int(hidden.sum())
        self._keep(np.nonzero(~hidden)[0])
        first = np.r_[0, np.nonzero(np.diff(self.labels))[0] + 1] if self.labels.size else np.zeros(0, np.int64)
        start = np.repeat(first, np.diff(np.r_[first, self.labels.size]))
        rank = np.arange(self.labels.size) - start
        self._keep(np.nonzero(rank < self.K - 1)[0])
        self.x = np.r_[self.x, nx]; self.y = np.r_[self.y, ny]
        self.L = np.r_[self.L, new["L"].reshape(-1, 3)]; self.D = np.r_[self.D, new["D"].reshape(-1, 3)]
        self.V = np.r_[self.V, new["V"].ravel()]; self.cov = np.r_[self.cov, new["cov"].ravel()]
        self.born = np.r_[self.born, np.full(h * w, t, np.int32)]; self.cid = np.r_[self.cid, cid]
        self.labels = np.r_[self.labels, (yy * w + xx).ravel()]
        self.phi_now = phi

    def estimate(self):
        h, w = self.world.h, self.world.w; N = h * w
        lab = self.labels
        if self.cells:
            qy, qx = np.divmod(lab, w)
            fx = self.x - qx - self.phi_now[0]; fy = self.y - qy - self.phi_now[1]
            key = lab * 4 + (fx >= .5) + 2 * (fy >= .5); M = N * 4
        else:
            key = lab; M = N
        cnt = np.bincount(key, minlength=M).astype(np.float64)
        def mean_of(a):
            s = np.stack([np.bincount(key, weights=a[:, c], minlength=M) for c in range(3)], -1)
            return s / np.maximum(cnt, 1)[:, None]
        L, D = mean_of(self.L.astype(np.float64)), mean_of(self.D.astype(np.float64))
        fin = np.isfinite(self.V)
        vc = np.bincount(key[fin], minlength=M).astype(np.float64)
        vs = np.bincount(key[fin], weights=self.V[fin].astype(np.float64), minlength=M)
        V = np.divide(vs, vc, out=np.full(M, np.nan), where=vc > 0)
        if self.cells:
            occ = (cnt > 0).reshape(N, 4); nocc = np.maximum(occ.sum(1), 1)
            L = (L.reshape(N, 4, 3) * occ[..., None]).sum(1) / nocc[:, None]
            D = (D.reshape(N, 4, 3) * occ[..., None]).sum(1) / nocc[:, None]
            vo = (vc > 0).reshape(N, 4)
            V = np.where(vo.any(1), np.nansum(np.where(vo, V.reshape(N, 4), 0), 1) / np.maximum(vo.sum(1), 1), np.nan)
            cnt = cnt.reshape(N, 4).sum(1)
        self.counts.append(cnt)
        return L.reshape(h, w, 3), D.reshape(h, w, 3), V.reshape(h, w)


# ------------------------------------------------------------------ motion

class Motion:
    def __init__(self, world, vc, vb):
        self.world = world; self.vc = np.asarray(vc, float); self.vb = np.asarray(vb, float)
        self.delta = self.vc - self.vb

    def phi(self, t):
        return t * self.vc - np.floor(t * self.vc)

    def jump(self, t):
        return np.floor(t * self.vc) - np.floor((t - 1) * self.vc)

    def aligned(self, t):
        return bool(np.all(self.phi(t) == 0))

    def sample_at(self, choice, phi):
        """Values of cloud point choice (17 per footprint at offset phi) for every label."""
        w = self.world; ax, ay, _, _ = candidates(phi)
        out = dict(L=np.empty((w.h, w.w, 3), np.float32), D=np.empty((w.h, w.w, 3), np.float32),
                   V=np.empty((w.h, w.w), np.float32), cov=np.empty((w.h, w.w), bool))
        for i in range(NC):
            m = choice == i
            if not m.any():
                continue
            for k, a in (("L", w.L), ("D", w.D), ("V", w.V), ("cov", w.cov)):
                out[k][m] = w.gather(a, i, ax[i], ay[i])[m]
        yy, xx = np.mgrid[0:w.h, 0:w.w]
        out["x"] = xx + ax[choice] + OFF[choice, 0]; out["y"] = yy + ay[choice] + OFF[choice, 1]
        # A background point shows the uniform background: the same colour wherever it is.
        sky = ~out["cov"]
        out["L"][sky] = w.clear; out["D"][sky] = w.Dsky; out["V"][sky] = np.nan
        return out

    def centre_sample(self, t):
        phi = self.phi(t); _, _, px, py = candidates(phi)
        i = int(np.argmin((px - .5) ** 2 + (py - .5) ** 2))
        return self.sample_at(np.full((self.world.h, self.world.w), i), phi)

    def lattice_sample(self, t, k, sym):
        phi = self.phi(t); _, _, px, py = candidates(phi)
        tx, ty = film_offset(k, sym)
        d = (tx[None] - px[:, None, None]) ** 2 + (ty[None] - py[:, None, None]) ** 2
        return self.sample_at(np.argmin(d, 0), phi)


def load_pose0(crop):
    """pose_data's inputs without the 8-ray noise model, whose accept-b7 visibility captures were
    deleted: every sample and the centre keep Exact (256-ray) visibility. The replay measured the
    8-ray noise at under 1% of the N = 8 error (docs/m6-temporal-primaries.md section 2)."""
    f0 = rp.f0; d = rp.R1 / "quarry-viewed-p0"
    ref = f0.decode(d, 0)
    point = np.load(f0.FROZEN / "gate-final/reference/pose-0-reference.npz")["ref"]
    pops = f0.populations(ref, point)
    pops["backlit"] = ref["cover"] & ~ref["front"]; pops["all-covered"] = ref["cover"]
    pops["image"] = np.ones_like(ref["cover"]); pops["multi-surface"] = ref["cover"] & (ref["n"] < 256)
    pops["background-centre"] = ~ref["cover"]
    raw = np.fromfile(d / "pose-0.film.bin", dtype=f0.DTYPE).reshape(H, W)
    direct = np.fromfile(d / "pose-0.direct.bin", dtype=f0.DTYPE).reshape(H, W)["radiance"][..., :3].astype(np.float64) / 256
    S = dict(L=[], D=[], V=[])
    full_sum = np.zeros((H, W, 3)); sun_sum = np.zeros((H, W, 3)); vis_sum = np.zeros((H, W)); acc_n = np.zeros((H, W))
    for i in range(16):
        L = np.fromfile(d / f"samples/{i:02}.rgba16f", dtype="<f2").reshape(H, W, 4)[..., :3].astype(np.float32)
        D = np.fromfile(d / f"samples/{i:02}.sun.rgba16f", dtype="<f2").reshape(H, W, 4)[..., :3].astype(np.float32)
        c = np.fromfile(d / f"samples/{i:02}.visibility.u16", dtype="<u2").reshape(H, W).astype(np.int64)
        full_sum += L; sun_sum += D; vis_sum += np.where(c != 65535, c, 0); acc_n += c != 65535
        S["L"].append(L[crop]); S["D"].append(D[crop]); S["V"].append(np.where(c != 65535, c / 256.0, np.nan)[crop].astype(np.float32))
    comp_L = (ref["hdr"][..., :3] * 256 - full_sum) / 240
    comp_D = (direct * 256 - sun_sum) / 240
    counts = raw["counts"].astype(np.float64)
    rest_n = counts[..., 2] + counts[..., 3] - acc_n
    comp_F = np.divide(counts[..., 0] + counts[..., 1] - vis_sum, 256 * rest_n, out=np.full((H, W), np.nan), where=rest_n > 0)
    exact_v = np.fromfile(d / "pose-0.exact.u32", dtype="<u4").reshape(H, W) / 256
    exact_L = np.fromfile(d / "pose-0.exact.rgba16f", dtype="<f2").reshape(H, W, 4)[..., :3].astype(np.float32)
    exact_D = np.fromfile(d / "pose-0.exact.sun.rgba16f", dtype="<f2").reshape(H, W, 4)[..., :3].astype(np.float32)
    cover = ref["cover"]
    exact = dict(L=exact_L[crop], D=exact_D[crop], V=np.where(cover, exact_v, np.nan)[crop])
    samples = {k: np.stack(v) for k, v in S.items()}
    samples.update(L8=samples["L"], D8=samples["D"], V8=samples["V"])
    return dict(ref=ref, pops={k: v[crop] for k, v in pops.items()}, comp=dict(L=comp_L[crop], D=comp_D[crop], F=comp_F[crop]),
                samples=samples, exact=exact, centre8=exact, cover=cover[crop])


def run(world, data, scorer, bar, method, motion_spec, frames, score_at, log):
    vc, vb = motion_spec
    motion = Motion(world, vc, vb)
    h, w = world.h, world.w
    yy, xx = np.mgrid[0:h, 0:w]
    sym = np.vectorize(lambda v: fresh_hash(v ^ 0x2f6b1e5d) & 3)((xx + 572) + (yy + 266) * W).astype(np.int64)
    kind, *rest = method.split("-")
    if kind == "t1b":
        state = Resampled(world, data, bit="bit" in rest, sym=sym)
    else:
        state = Points(world, data, K=int(kind[1:]), choice=rest[0], cells="cells" in rest, sym=sym, nn="nn" in rest)
        state.phi_now = np.zeros(2)
    out = {}; start = time.perf_counter(); prev = None
    est = state.estimate()
    out[0] = scorer.score(*est)
    for t in range(1, frames + 1):
        if kind == "t1b":
            sample = motion.lattice_sample(t, state.next_k(), sym)
            state.step(t, motion, sample)
        else:
            state.step(t, motion, None)
        if t in score_at:
            if not motion.aligned(t):
                raise SystemExit(f"frame {t} not aligned")
            est = state.estimate()
            row = scorer.score(*est)
            if kind == "t1b":
                row["resets_total"] = state.resets; row["catmull_share"] = state.catmull_share
                row["reverse_accepts_total"] = state.reverse_accepts
                old = state.age >= 32
                if old.any():
                    sub = Scorer.__new__(Scorer); sub.__dict__ = dict(scorer.__dict__)
                    sub.pops = {k: v & old for k, v in scorer.pops.items()}; sub.single = scorer.single & old; sub.cover = scorer.cover & old
                    row["aged32"] = sub.score(*est)
            else:
                cnt = state.counts[-1].reshape(h, w)[data["cover"]]
                row["held"] = dict(mean=float(cnt.mean()), p1=float(np.percentile(cnt, 1)), p50=float(np.median(cnt)), p99=float(np.percentile(cnt, 99)))
                row["dropped_hidden_total"] = state.dropped_hidden
                row["band_background_records_tested_last"] = state.tested[-1]
            if prev is not None:
                Lp, _, _ = prev
                d = np.mean((rp.codes(est[0], 0) - rp.codes(Lp, 0)) ** 2, -1)
                row["aligned_step_L0_covered"] = float(np.sqrt(d[data["cover"]].mean()))
            prev = est
            out[t] = row
            if t == max(score_at):
                out["final_estimate"] = est
            r = row["L0"]
            log(f"   {method} v={tuple(vc)} t={t}: L0 all-covered {r['all-covered']['rms'] / bar['L0']['all-covered']['rms']:.3f}x "
                f"edge {r['edge']['rms'] / bar['L0']['edge']['rms']:.3f}x bg-d1.5-3 {r['bg-d1.5-3']['rms']:.2f} "
                f"F solid max {max(row['F']['lit']['max'], row['F']['occluded']['max']):.4f} "
                f"verified {max(row['F']['solid-lit-v']['max'], row['F']['solid-occ-v']['max']):.4f} "
                f"D4 occ {r_d4(row, bar):.3f}x F4 {row['missed']}+{row['false']} D4>16 {row['D4_over16']} "
                f"({time.perf_counter() - start:.0f} s)")
    return out


def r_d4(row, bar):
    return row["D4"]["occluded"]["rms"] / bar["D4"]["occluded"]["rms"]


def ratios(row, bar):
    out = {}
    for ch in ("L0", "L4", "D0", "D4"):
        out[ch] = {p: round(row[ch][p]["rms"] / bar[ch][p]["rms"], 4) for p in row[ch] if p in bar[ch] and bar[ch][p]["rms"] > 1e-9}
    out["F"] = {p: round(row["F"][p]["rms"] / bar["F"][p]["rms"], 4) for p in row["F"] if isinstance(row["F"][p], dict) and p in bar["F"] and bar["F"][p]["rms"] > 1e-9}
    return out


def err_png(est, ref, stops, path, box):
    d = np.mean(rp.codes(est, stops) - rp.codes(ref, stops), -1)[box] * 8 / 255
    rgb = np.stack([np.clip(d, 0, 1), np.zeros_like(d), np.clip(-d, 0, 1)], -1)
    Image.fromarray(np.rint(rgb * 255).astype("u1")).resize((d.shape[1] * 3, d.shape[0] * 3), Image.Resampling.NEAREST).save(path)


def vis_png(V, ref, cover, path, box):
    d = np.where(np.isfinite(V), np.nan_to_num(V) - np.nan_to_num(ref), 0)[box] * 8
    rgb = np.stack([np.clip(d, 0, 1), np.zeros_like(d), np.clip(-d, 0, 1)], -1)
    rgb[~cover[box]] = [0.08, 0.1, 0.14]
    Image.fromarray(np.rint(rgb * 255).astype("u1")).resize((d.shape[1] * 3, d.shape[0] * 3), Image.Resampling.NEAREST).save(path)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--out", type=Path, default=Path("E:/renderlab-work/motion-aa"))
    p.add_argument("--only", default="", help="comma-separated motion names to run")
    p.add_argument("--methods", default="", help="comma-separated methods to run (default: the registered set)")
    p.add_argument("--frames", type=int, default=0, help="smoke test: stop after this many frames")
    a = p.parse_args(); a.out.mkdir(parents=True, exist_ok=True)
    t0 = time.perf_counter(); logf = open(a.out / "run.log", "a", encoding="utf-8")
    def log(s):
        print(s, flush=True); logf.write(s + "\n"); logf.flush()
    cover = np.zeros((H, W), bool)
    for pose in (0, 1):
        cover |= np.fromfile(rp.R1 / f"quarry-viewed-p{pose}/pose-{pose}.hit.u32", dtype="<u4").reshape(H, W, 4)[..., 0] != 0
    ys, xs = np.nonzero(cover)
    crop = (slice(max(0, ys.min() - 8), min(H, ys.max() + 9)), slice(max(0, xs.min() - 8), min(W, xs.max() + 9)))
    assert crop[0].start == 266 and crop[1].start == 572, crop
    data = load_pose0(crop)
    log(f"loaded pose 0 crop {crop} in {time.perf_counter() - t0:.0f} s")
    world = World(data)
    data["samples"] = None
    data["cloudV"] = world.V[:, :world.h, :world.w]
    scorer = Scorer(data, crop)
    bar = scorer.score(data["exact"]["L"], data["exact"]["D"], data["exact"]["V"])
    inter = scorer.score(data["centre8"]["L"], data["centre8"]["D"], data["centre8"]["V"])
    # Sanity: the mean of all 17 cloud points per pixel against the replay's N = 16 figures.
    allL = (world.L[:, :world.h, :world.w].mean(0)); allD = world.D[:, :world.h, :world.w].mean(0)
    allV = np.nanmean(np.where(np.isfinite(world.V[:, :world.h, :world.w]), world.V[:, :world.h, :world.w], np.nan), 0)
    static17 = scorer.score(allL, allD, allV)
    result = dict(crop=[crop[0].start, crop[0].stop, crop[1].start, crop[1].stop], offsets=OFF.tolist(),
                  bar=bar, interactive=inter, static17=static17, static17_ratio=ratios(static17, bar),
                  interactive_ratio=ratios(inter, bar), runs={})
    bf = bar["F"]
    log(f"Exact: F4 {bar['missed']}+{bar['false']}, D4>16 {bar['D4_over16']}, L0 edge {bar['L0']['edge']['rms']:.2f}, "
        f"bg-d0-1.5 {bar['L0']['bg-d0-1.5']['rms']:.2f}, bg-d1.5-3 {bar['L0']['bg-d1.5-3']['rms']:.4f}")
    log("solids: " + ", ".join(f"{k} n={bf[k]['n']} Exact max {bf[k]['max']:.4f} over .002 {bf[k]['over_002']}" for k in ("lit", "occluded", "solid-lit-v", "solid-occ-v")))
    log(f"static 17-point mean: L0 all-covered {static17['L0']['all-covered']['rms'] / bar['L0']['all-covered']['rms']:.3f}x bar; "
        f"Interactive model {inter['L0']['all-covered']['rms'] / bar['L0']['all-covered']['rms']:.3f}x")
    motions = {
        "M1": ((0.5, 0.5), (0.0, 0.5), 48, [2, 4, 6, 8, 16, 24, 32, 40, 48]),
        "M2": ((0.25, 0.875), (0.75, 0.875), 48, [8, 16, 24, 32, 40, 48]),
        "M3": ((0.125, 0.0625), (0.0, 0.0625), 64, [16, 32, 48, 64]),
    }
    methods = {
        "M1": ["t1b", "t1b-bit", "p8-lattice", "p8-gap", "p8-gap-nn"],
        "M2": ["t1b", "t1b-bit", "p4-gap", "p8-lattice", "p8-gap", "p8-gap-nn", "p8-gap-cells", "p16-gap"],
        "M3": ["t1b", "p8-gap", "p8-gap-nn"],
    }
    only = set(a.only.split(",")) if a.only else set(motions)
    box = (slice(30, 230), slice(330, 630))
    for name, (vc, vb, frames, score_at) in motions.items():
        if name not in only:
            continue
        if a.frames:
            frames = a.frames; score_at = [t for t in score_at if t <= frames] or [frames]
        chosen = [m for m in methods[name] if not a.methods or m in a.methods.split(",")]
        for method in chosen:
            log(f" run {name} {method}")
            r = run(world, data, scorer, bar, method, (vc, vb), frames, score_at, log)
            est = r.pop("final_estimate")
            err_png(est[0], data["comp"]["L"], 4, a.out / f"{name}-{method}-L4-err-x8.png", box)
            vis_png(est[2], data["comp"]["F"], data["cover"], a.out / f"{name}-{method}-F-err-x8.png", box)
            for t in list(r):
                if isinstance(t, int) and t > 0:
                    r[t]["ratio"] = ratios(r[t], bar)
                    if "aged32" in r[t]:
                        r[t]["aged32_ratio"] = ratios(r[t]["aged32"], bar)
            result["runs"][f"{name}/{method}"] = dict(vc=vc, vb=vb, frames=r)
            (a.out / "motion-aa-sim.json").write_bytes(json.dumps(result, indent=1, default=lambda o: o.item() if hasattr(o, "item") else str(o)).encode())
    err_png(data["exact"]["L"], data["comp"]["L"], 4, a.out / "exact-L4-err-x8.png", box)
    vis_png(data["exact"]["V"], data["comp"]["F"], data["cover"], a.out / "exact-F-err-x8.png", box)
    result["seconds"] = time.perf_counter() - t0
    (a.out / "motion-aa-sim.json").write_bytes(json.dumps(result, indent=1, default=lambda o: o.item() if hasattr(o, "item") else str(o)).encode())
    log(f"done in {result['seconds']:.0f} s")


if __name__ == "__main__":
    main()
