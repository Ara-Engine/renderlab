"""Light CPU replay of mapped-patch GGX, to diagnose distribution and quantization errors.

Uses the saved GPU film/reference and baseline, not a replacement acceptance oracle.
CPU texture interpolation and arithmetic need not equal the driver's bit for bit.
"""
import importlib.util
import json
import math
from pathlib import Path
import struct

import numpy as np

ROOT = Path("E:/renderlab-work/m7-u1b")
spec = importlib.util.spec_from_file_location("score", Path(__file__).with_name("m7-u1-score.py"))
score = importlib.util.module_from_spec(spec)
spec.loader.exec_module(score)


def textures():
    b = (ROOT / "fixture.rla").read_bytes()
    offset, result = 24, []
    for _ in range(struct.unpack_from("<I", b, 20)[0]):
        role, levels = struct.unpack_from("<II", b, offset); offset += 8
        mips = []
        for _ in range(levels):
            w, h, n = struct.unpack_from("<III", b, offset); offset += 12
            if role == 4:
                value = np.frombuffer(b, dtype="<f2", count=n // 2, offset=offset).reshape(2, h, w, 4).astype(float)
                value = np.concatenate([value[0], value[1]], axis=-1)
            else:
                value = np.frombuffer(b, dtype=np.uint8, count=n, offset=offset).reshape(h, w, 4).astype(float) / 255
            mips.append(value); offset += n
        result.append(mips)
    return result


def sample(mips, uv, lod):
    def level(index):
        a = mips[index]; h, w = a.shape[:2]
        xy = uv * [w, h] - .5; q = np.floor(xy).astype(int); f = xy - q
        return sum(a[(q[:, 1] + y) % h, (q[:, 0] + x) % w] * ((f[:, 0] if x else 1-f[:, 0]) * (f[:, 1] if y else 1-f[:, 1]))[:, None] for x in (0, 1) for y in (0, 1))
    lod = np.clip(lod, 0, len(mips)-1); j = int(lod)
    return level(j) * (1-lod+j) + level(min(j+1, len(mips)-1)) * (lod-j)


def unit(v):
    return v / np.linalg.norm(v, axis=-1, keepdims=True)


def brdf(n, v, light, rough):
    nl = np.maximum(n @ light, 0)
    nv = np.maximum(np.sum(n*v, axis=-1), 1e-7)
    h = unit(v+light); nh = np.maximum(np.sum(n*h, axis=-1), 0)
    a2 = max(rough*rough, 1e-4)**2
    denom = np.sum(np.cross(n, h)**2, axis=-1) + a2*nh*nh
    d = a2 / (math.pi * denom**2)
    visibility = .5 / (nl*np.sqrt(a2+(1-a2)*nv*nv) + nv*np.sqrt(a2+(1-a2)*nl*nl))
    vh = np.clip(np.sum(v*h, axis=-1), 0, 1)
    base = np.array([.7, .5, .25])
    fresnel = base+(1-base)*(1-vh[:, None])**5
    return d[:, None]*visibility[:, None]*fresnel*nl[:, None]*(1000/256)


def main():
    maps = textures(); rows = []
    w, h = 320, 180
    half_y = 4*math.tan(.4); pitch = 2*half_y/h
    lod = math.log2(max(512*pitch/1.05, 512*pitch/.6))
    for track in ("static", "camera", "light"):
        for frame in range(1 if track == "static" else 12):
            reference = score.read(ROOT / "reference" / f"{track}-{frame}.f32", [w, h])
            eye_x = frame*.5*pitch if track == "camera" else 0
            light = unit(np.array([frame*.005 if track == "light" else 0, .05, 1.]))
            for patch, rough in enumerate((.08, .15, .3)):
                mask = score.patch_mask([w, h], patch, track, frame)
                ys, xs = np.nonzero(mask)
                world = np.column_stack(((xs+.5-w/2)*pitch+eye_x, (h/2-ys-.5)*pitch, np.zeros_like(xs)))
                v = unit(np.array([eye_x, 0, 4.])-world)
                uv = (world[:, :2] - [-1.65+patch*1.1, .65]) / [1.05, .6]
                legacy = sample(maps[1], uv, lod)[:, :3]*2-1
                results = {"before-cpu":brdf(unit(legacy), v, light, rough)}
                for index, label in ((2, "point"), (3, "bilinear")):
                    a = sample(maps[index], uv, lod)
                    mean = a[:, :2]; xx = np.maximum(a[:, 2]-mean[:, 0]**2, 0)
                    yy = np.maximum(a[:, 3]-mean[:, 1]**2, 0)
                    xy = np.clip(a[:, 4]-mean[:, 0]*mean[:, 1], -np.sqrt(xx*yy), np.sqrt(xx*yy))
                    l00 = np.sqrt(xx); l10 = xy / np.maximum(l00, 1e-20); l11 = np.sqrt(np.maximum(yy-l10*l10, 0))
                    for order in (3, 5, 9):
                        nodes, weights = np.polynomial.hermite_e.hermegauss(order); weights /= np.sqrt(2*math.pi)
                        value = np.zeros((len(xs), 3))
                        for iy, ny in enumerate(nodes):
                            for ix, nx in enumerate(nodes):
                                slope = mean + np.column_stack((l00*nx, l10*nx+l11*ny))
                                normal = unit(np.column_stack((slope, np.ones(len(xs)))))
                                value += brdf(normal, v, light, rough) * weights[ix]*weights[iy]
                        results[f"{label}-mixture-{order**2}"] = value
                ref = reference[mask]
                for label, image in results.items():
                    error = image-ref
                    rows.append(dict(track=track, frame=frame, roughness=rough, variant=label, rms=float(np.sqrt(np.mean(error**2))), bias=float(error.sum()/ref.sum())))
    geometric=[]
    for track in ("static","camera","light"):
        for frame in range(1 if track=="static" else 12):
            reference=score.read(ROOT/"reference"/f"{track}-{frame}.f32",[w,h]); eye_x=frame*.5*pitch if track=="camera" else 0
            light=unit(np.array([frame*.005 if track=="light" else 0,.05,1.]));eye=np.array([eye_x,0,4.])
            for patch,rough in enumerate((.08,.15,.3),3):
                mask=score.patch_mask([w,h],patch,track,frame);ys,xs=np.nonzero(mask)
                world=np.column_stack(((xs+.5-w/2)*pitch+eye_x,(h/2-ys-.5)*pitch,np.zeros_like(xs)))
                def evaluate(world):
                    uv=(world[:,:2]-[-1.65+(patch%3)*1.1,0])/[1.05,.6]
                    normal=unit(np.column_stack(((uv-.5)*.6,np.ones(len(xs)))))
                    return brdf(normal,unit(eye-world),light,rough)
                images={"before-cpu":evaluate(world)}
                for order in (3,5,7):
                    nodes,weights=np.polynomial.legendre.leggauss(order);nodes/=2;weights/=2
                    image=np.zeros((len(xs),3))
                    for iy,ny in enumerate(nodes):
                        for ix,nx in enumerate(nodes):
                            q=world+np.array([nx*pitch,ny*pitch,0]);image+=evaluate(q)*weights[ix]*weights[iy]
                    images[f"box-{order**2}"]=image
                for label,image in images.items():
                    error=image-reference[mask];geometric.append(dict(track=track,frame=frame,roughness=rough,variant=label,rms=float(np.sqrt(np.mean(error**2))),bias=float(error.sum()/reference[mask].sum())))
    (ROOT/'cpu-geometric-replay.json').write_text(json.dumps(geometric,indent=2))
    for rough in (.08,.15,.3):
        print('geometry',rough,[(v,round(np.sqrt(np.mean([r['rms']**2 for r in geometric if r['roughness']==rough and r['variant']==v])),6),round(max(abs(r['bias']) for r in geometric if r['roughness']==rough and r['variant']==v),6)) for v in sorted({r['variant'] for r in geometric})])
    (ROOT / "cpu-replay.json").write_text(json.dumps(rows, indent=2))
    for rough in (.08, .15, .3):
        print(rough, [(v, round(np.sqrt(np.mean([r['rms']**2 for r in rows if r['roughness']==rough and r['variant']==v])), 6), round(max(abs(r['bias']) for r in rows if r['roughness']==rough and r['variant']==v), 6)) for v in sorted({r['variant'] for r in rows})])


if __name__ == "__main__":
    main()
