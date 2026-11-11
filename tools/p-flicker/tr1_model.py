"""P-FLICKER 1-D model of TR1's moving interior (CPU, seconds): a textured line moving by a
fractional pixel offset each frame, reconstructed as temporal_resolve.wgsl does for a pixel
whose 16 history taps are accepted (Catmull-Rom fetch clamped to the 2 inner taps' range,
window `sharp`, one jittered sample per frame), scored with P-YARD's flicker residual
(output step minus the box-filtered truth's step) and the static error. Ranks variants only;
the quarry decides.

    py -3 -B tools/p-flicker/tr1_model.py
"""
import numpy as np

SS = 32  # supersamples per pixel of the continuous signal
N = 512  # pixels
rng = np.random.default_rng(7)


def texture():
    # Pink-ish noise with detail at and below the pixel scale, like the quarry's rock.
    f = np.fft.rfftfreq(N * SS * 4)
    spec = (rng.normal(size=f.size) + 1j * rng.normal(size=f.size)) / np.maximum(f, 1 / (N * SS)) ** 0.9
    spec[f > 2.0 / SS] = 0  # band-limit at twice the pixel rate (texture filtering)
    s = np.fft.irfft(spec)
    return (s - s.min()) / (s.max() - s.min()) * 200 + 20


SIGNAL = texture()
LATTICE = np.array([0.0] + [((k * 0.618034) % 1) - 0.5 for k in range(1, 255)] + [0.0])


def value(x):
    """Signal at continuous pixel coordinates x (linear interpolation of the supersamples)."""
    u = x * SS
    i = np.floor(u).astype(int)
    t = u - i
    return SIGNAL[i] * (1 - t) + SIGNAL[i + 1] * t


def truth(offset):
    xs = np.arange(N)[:, None] + offset + (np.arange(SS)[None, :] + 0.5) / SS - 0.5
    return value(xs + 64).mean(1)


def cr(t):
    return np.stack([((-0.5 * t + 1) * t - 0.5) * t, (1.5 * t - 2.5) * t * t + 1, ((-1.5 * t + 2) * t + 0.5) * t, (0.5 * t - 0.5) * t * t])


def run(sharp=16, clamp=True, jitter=True, resample="cr", frames=200, speed=0.37, wobble=0.15, ss_history=1):
    offset, h, n, out, ref = 0.0, None, None, [], []
    for k in range(frames):
        d = speed + wobble * np.sin(k * 0.7)
        offset += d
        j = LATTICE[k % 256] if jitter else 0.0
        sample = value(np.arange(N) + offset + j + 64)
        if h is None:
            h, n = truth(offset), np.full(N, 1.0)
        else:
            # Pixel p's surface point was at p + d in the previous frame.
            x = np.arange(N) + d
            b = np.floor(x).astype(int)
            f = x - b
            taps = np.stack([h[np.clip(b + o, 0, N - 1)] for o in (-1, 0, 1, 2)])
            if resample == "cr":
                w = cr(f)
                c = (w * taps).sum(0)
                if clamp:
                    c = np.clip(c, np.minimum(taps[1], taps[2]), np.maximum(taps[1], taps[2]))
                window = sharp
            else:
                c = taps[1] * (1 - f) + taps[2] * f
                window = np.minimum(256, 1 + 1.0 / np.maximum(f * (1 - f), 1e-6))
            n = np.minimum(n + 1, window)
            h = c + (sample - c) / n
        out.append(h.copy())
        ref.append(truth(offset))
    o, r = np.array(out[40:]), np.array(ref[40:])
    step = np.diff(o, axis=0) - np.diff(r, axis=0)
    return dict(flicker=float(np.sqrt((step[:, 8:-8] ** 2).mean())), error=float(np.sqrt(((o - r)[:, 8:-8] ** 2).mean())))


if __name__ == "__main__":
    for name, kw in [("main (sharp 16, CR, clamp)", {}), ("no jitter", dict(jitter=False)),
                     ("sharp 8", dict(sharp=8)), ("sharp 32", dict(sharp=32)), ("sharp 64", dict(sharp=64)),
                     ("CR unclamped", dict(clamp=False)), ("CR unclamped, sharp 32", dict(clamp=False, sharp=32)),
                     ("bilinear + blur budget 1", dict(resample="bilinear"))]:
        print(f"{name:28s}", "flicker %.3f  error %.3f" % tuple(run(**kw).values()))
