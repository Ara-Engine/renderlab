"""P-FLAT-TR1 motion metrics (docs/p-flat-tr1.md): history lag under a flickering light and
trailing moving-caster shadows, from every-frame courtyard runs (`game_scene --every 1`)
against a no-history control of the same flight (TR1 off), which shows each frame's own
lighting. Masks come from the analytic scene (scripts/p-flat-score.py), never from colours.

flicker RUN... --control DIR: the red wall and the ground within 2.5 m of the brazier, lit by
  the flickering fire. Per frame, the crop's mean linear luminance. Each run's frame-to-frame
  change is regressed on the control's: slope 1 follows the flicker; below 1 lags it. Also the
  mean level bias against the control.
trail RUN... --control DIR --hour H: ground pixels the Fox's sun-shadow hull covered in one of the
  previous 6 frames but not this one (dilated by its own hull), against clear ground within 4 m
  of the Fox. A trailing shadow shows as the vacated band reading darker than the control,
  relative to the clear band.

    py -3 -B scripts/p-flat-motion.py OUT.json flicker RUN... --control DIR
    py -3 -B scripts/p-flat-motion.py OUT.json trail RUN... --control DIR --hour 12
"""
import importlib.util
import json
import math
import sys
from pathlib import Path

import numpy as np

spec = importlib.util.spec_from_file_location("score", Path(__file__).with_name("p-flat-score.py"))
score = importlib.util.module_from_spec(spec)
spec.loader.exec_module(score)
BRAZIER = np.array([-3.8, 0.0, 2.0])
LATITUDE = 0.7


def linear(rgb):
    c = rgb / 255.0
    lin = np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)
    return lin @ np.array([0.2126, 0.7152, 0.0722])


def geometry(run, entry):
    face, label, points = score.labels(entry["eye"], entry["target"], entry["t"], run["fox"], run["size"],
                                       run["head"], points=True)
    return score.eroded(face), label, points


def toward_sun(hour):
    angle = (hour - 12.0) * math.pi / 12.0
    v = np.array([-math.sin(angle), math.cos(LATITUDE) * math.cos(angle), math.sin(LATITUDE) * math.cos(angle)])
    return v / np.linalg.norm(v)


def fox_hull(t):
    """The scorer's conservative Fox hull at time t: centre, linear map of the unit cube."""
    walk = 0.35 * t
    ground = np.array([3 * math.sin(walk), 0.0, -1 + 3 * math.cos(walk)])
    heading = math.atan2(math.cos(walk), -math.sin(walk))
    return ground + (0, 0.55, 0), score.rot_y(heading) @ np.diag((1.8, 1.3, 1.8))


def in_shadow(points, sun, t):
    """Points whose ray toward the sun meets the Fox hull at time t."""
    c, m = fox_hull(t)
    inv = np.linalg.inv(m)
    o = (points - c) @ inv.T
    d = inv @ sun
    with np.errstate(divide="ignore", invalid="ignore"):
        t0, t1 = (-0.5 - o) / d, (0.5 - o) / d
    near = np.nanmax(np.minimum(t0, t1), axis=-1)
    far = np.nanmin(np.maximum(t0, t1), axis=-1)
    return (near <= far) & (far > 0)


def frames(run_dir):
    run = score.frames_of(Path(run_dir), False)
    return run, {e["frame"]: e for e in run["frames"]}


def flicker(runs, control):
    crun, cframes = frames(control)
    out = {}
    series = {}
    for path in [control, *runs]:
        run, fr = frames(path)
        values = []
        for k in sorted(fr):
            e = fr[k]
            inside, label, points = geometry(run, cframes[k])
            near_fire = np.linalg.norm((points - BRAZIER)[..., [0, 2]], axis=-1)
            wall = inside & (label == 3) & (points[..., 2] > 0.6) & (points[..., 2] < 3.0)
            ground = inside & (label == 1) & (near_fire < 2.5)
            mask = wall | ground
            values.append(float(linear(score.load(e["png"]))[mask].mean()))
        series[Path(path).name] = values
    c = np.array(series[Path(control).name])
    dc = np.diff(c)[7:]
    for name, values in series.items():
        v = np.array(values)
        dv = np.diff(v)[7:]
        slope = float(np.dot(dv, dc) / np.dot(dc, dc))
        out[name] = dict(slope=slope, residual_rms=float(np.sqrt(np.mean((dv - slope * dc) ** 2)) / c.mean()),
                         level_bias=float(np.mean((v[8:] - c[8:]) / c[8:])), control_change_rms=float(np.sqrt(np.mean(dc ** 2)) / c.mean()),
                         series=values)
    return out


def trail(runs, control, hour):
    sun = toward_sun(hour)
    crun, cframes = frames(control)
    images = {Path(p).name: frames(p)[1] for p in [control, *runs]}
    out = {Path(p).name: dict(trail=[], clear=[]) for p in runs}
    for k in sorted(cframes):
        if k < 8:
            continue
        e = cframes[k]
        inside, label, points = geometry(crun, e)
        ground = inside & (label == 1)
        now = in_shadow(points, sun, e["t"])
        recent = np.zeros_like(now)
        for j in range(1, 7):
            recent |= in_shadow(points, sun, cframes[k - j]["t"])
        fox_xz = fox_hull(e["t"])[0][[0, 2]]
        near = np.linalg.norm(points[..., [0, 2]] - fox_xz, axis=-1) < 4.0
        band = ground & recent & ~now
        clear = ground & near & ~recent & ~now
        if band.sum() < 50 or clear.sum() < 50:
            continue
        ref = linear(score.load(images[Path(control).name][k]["png"]))
        for p in runs:
            img = linear(score.load(images[Path(p).name][k]["png"]))
            rel = (img - ref) / np.maximum(ref, 1e-4)
            out[Path(p).name]["trail"].append(float(rel[band].mean()))
            out[Path(p).name]["clear"].append(float(rel[clear].mean()))
    for v in out.values():
        v["frames"] = len(v["trail"])
        v["trail_minus_clear"] = float(np.mean(np.array(v["trail"]) - np.array(v["clear"]))) if v["trail"] else None
    return out


def main():
    args = sys.argv[1:]
    out, mode = Path(args[0]), args[1]
    options, runs, rest = {}, [], iter(args[2:])
    for a in rest:
        if a.startswith("--"):
            options[a] = next(rest)
        else:
            runs.append(a)
    control = options["--control"]
    hour = float(options.get("--hour", 12.0))
    result = flicker(runs, control) if mode == "flicker" else trail(runs, control, hour)
    out.write_text(json.dumps(result, indent=1), encoding="utf-8")
    for name, v in result.items():
        print(name, {k: (round(x, 4) if isinstance(x, float) else x) for k, x in v.items() if k not in ("series", "trail", "clear")})


if __name__ == "__main__":
    main()
