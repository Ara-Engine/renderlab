"""M7-TR1: score Renderlab reconstruction outputs on TR0's physical target and compare
them, row by row, with TR0's retained TSR Epic rows (E:/renderlab-work/m7-tr0/bar).

  score    one candidate directory of fNNN.rgba16f HDRs (exposed at the registered
           exposure) against U0b's lossless zero-shutter references, with TR0's exact
           arithmetic (scripts/m7-tr0.py `score`) and the same fixed T1b age masks
  compare  the candidate's rows against the bar's TSR (and recorded T1b) rows of the
           matched case: every row is a separate comparison, lower is better, a missing
           candidate row never passes

CPU only. The candidate replaces only Unreal's native capture: references, populations,
masks, transforms, noise subtraction and the age protocol are TR0's. No fitting.
"""
import argparse
import csv
import contextlib
import hashlib
import io
import json
import math
from pathlib import Path

import numpy as np

TR0 = Path("E:/renderlab-work/m7-tr0")
REFERENCE = Path("E:/renderlab-work/m7-u0b/reference/quarry-final")
AGES = TR0 / "runs/t1b-black-b"
SCRIPT_SHA = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
W, H = 1920, 1080


def codes(hdr, stops):
    # scripts/m6-temporal-gpu-score.py's frozen F0 transform, as TR0 loads it.
    x = np.maximum(0, hdr * (2 ** stops)); x = x / (1 + x)
    return 255 * np.where(x <= .0031308, x * 12.92, 1.055 * np.power(x, 1 / 2.4) - .055)


def score(args):
    from scipy.ndimage import distance_transform_edt
    recipe = json.loads((TR0 / "runs" / args.case / "recipe.json").read_text())
    if recipe["track"] != "q-walk":
        raise ValueError("TR1 scoring registers Q-walk only until other tracks have references")
    prefix = "L"
    candidate_dir = Path(args.candidate)

    def camera_key(p):
        return np.asarray(p["eye"] + p["target"], dtype="<f4").tobytes()
    references = {}
    for path in list(REFERENCE.glob("q-walk-*.json")) + list(REFERENCE.glob("quarry-p*.json")):
        meta = json.loads(path.read_text())
        if meta.get("accepted_for_scoring") is False:
            raise ValueError("reference archive is explicitly unaccepted: " + str(path))
        frame = meta.get("frame", {})
        if not frame or frame.get("open") != frame.get("close") or frame.get("sky_only"):
            continue
        archive = path.with_suffix(".npz")
        if archive.exists():
            if meta.get("camera_background") != [0., 0., 0.]:
                raise ValueError("reference must apply black to primary misses before integration")
            references[camera_key(frame["open"])] = (archive, meta)
    arrival = {}
    for f, p in enumerate(recipe["frames"]):
        arrival.setdefault(camera_key(p), f)
    rows, residuals, missing, transient = [], [], [], {}
    previous = None
    cached_archive, cached_arrays, cached_sha = None, None, None
    candidates = {}
    for i, p in enumerate(recipe["frames"]):
        path = candidate_dir / f"f{i:03}.rgba16f"
        if not path.exists():
            previous = None
            continue
        source = references.get(camera_key(p))
        if source is None:
            missing.append(i)
            previous = None
            continue
        archive, meta = source
        frame = meta["frame"]
        if (frame["width"], frame["height"]) != (W, H) or \
                np.asarray(frame["fov_y"], dtype="<f4").tobytes() != np.asarray(p["fov_y_radians"], dtype="<f4").tobytes():
            raise ValueError("reference projection mismatch")
        raw = path.read_bytes()
        candidates[i] = hashlib.sha256(raw).hexdigest()
        # Renderlab's HDR target is already exposed at the registered -14 stops.
        hdr = np.frombuffer(raw, dtype="<f2").reshape(H, W, 4)[..., :3].astype("f4")
        age = None
        age_path = AGES / "data" / f"age-f{i:03}.u8"
        if age_path.exists():
            age = np.fromfile(age_path, dtype="u1").reshape(H, W)
        if archive != cached_archive:
            payload = archive.read_bytes()
            archive_sha = hashlib.sha256(payload).hexdigest()
            if meta.get("archive_sha256") and archive_sha != meta["archive_sha256"]:
                raise ValueError("reference archive differs from its provenance")
            with np.load(io.BytesIO(payload)) as loaded:
                cached_arrays = {k: loaded[k] for k in loaded.files
                                 if k in ("L0", "L1", "D0", "D1", "visibility", "hit") or k.startswith("pop_")}
            del payload
            cached_archive, cached_sha = archive, archive_sha
        ref = cached_arrays
        scale = 2 ** -14
        mean = (ref[prefix + "0"] + ref[prefix + "1"]) * .5 * scale
        halves = [ref[k] * scale for k in (prefix + "0", prefix + "1")]
        result = dict(frame=i, reference=str(archive), reference_sha256=cached_sha, samples=meta.get("samples"),
                      hdr_sha256=candidates[i], measures={}, transient_pools={})
        distance = distance_transform_edt(~ref["pop_edge"]) if "pop_edge" in ref else None
        result["background_distance_bands"] = {}
        result["background_error_extent"] = {}
        changed = None
        back = i - 8
        if back >= 0 and camera_key(recipe["frames"][back]) == camera_key(p):
            back = arrival[camera_key(p)] - 8
        back_source = references.get(camera_key(recipe["frames"][back])) if back >= 0 else None
        if back_source is not None and camera_key(recipe["frames"][back]) != camera_key(p):
            with np.load(back_source[0]) as before:
                if all(k in ref and k in before for k in ("visibility", "D0", "D1")):
                    now_d = (ref["D0"] + ref["D1"]) * .5 * scale
                    before_d = (before["D0"] + before["D1"]) * .5 * scale
                    changed = (np.abs(ref["visibility"] - before["visibility"]) > .05) | \
                        (np.abs(codes(now_d, 4) - codes(before_d, 4)).mean(-1) > 8)
                    changed &= ref["pop_all-covered"]
                    result["changed"] = dict(back_frame=back, pixels=int(changed.sum()), measures={})
        for stops in (0, 4):
            candidate, reference = codes(hdr, stops), codes(mean, stops)
            error = candidate - reference
            noise = (codes(halves[0], stops) - codes(halves[1], stops)) * .5
            e2, v2 = np.mean(error ** 2, -1), np.mean(noise ** 2, -1)
            signed_error = error.mean(-1)
            pops = {}
            for key in ref:
                if key.startswith("pop_"):
                    mask = ref[key]
                    if mask.any():
                        pops[key[4:]] = dict(n=int(mask.sum()), rms=float(np.sqrt(e2[mask].mean())),
                              rms_noise_subtracted=float(np.sqrt(max(0, float((e2 - v2)[mask].mean())))),
                              signed_mean=float(error[mask].mean()), maximum=float(np.max(np.abs(error[mask]))),
                              reference_noise_rms=float(np.sqrt(v2[mask].mean())))
                        signed = signed_error[mask]
                        pops[key[4:]]["se"] = float(signed.std(ddof=1) / math.sqrt(signed.size)) if signed.size > 1 else 0
                        if age is not None:
                            converged = mask & (age >= 32)
                            if converged.any():
                                pops[key[4:]]["shared_age32"] = dict(n=int(converged.sum()),
                                    rms_noise_subtracted=float(np.sqrt(max(0, float((e2 - v2)[converged].mean())))),
                                    signed_mean=float(error[converged].mean()))
                            fresh = mask & (age < 8)
                            for event in ("start", "revealed"):
                                selected = fresh & ((age == i) if event == "start" else (age < i))
                                if selected.any():
                                    bins = age[selected]
                                    counts = np.bincount(bins, minlength=8)
                                    fields = {"e2": e2, "reference_v2": v2}
                                    sums = {field: np.bincount(bins, weights=a[selected], minlength=8) for field, a in fields.items()}
                                    for value in np.flatnonzero(counts):
                                        name = f"{event}/{value}/{key[4:]}/{prefix}{stops}"
                                        for target in (transient, result["transient_pools"]):
                                            pool = target.setdefault(name, dict(n=0))
                                            pool["n"] += int(counts[value])
                                            for field, values in sums.items():
                                                pool[field] = pool.get(field, 0.) + float(values[value])
            if distance is not None and "pop_background-centre" in ref:
                bands = {}
                for low, high in ((0, 1.5), (1.5, 3), (3, 6), (6, float("inf"))):
                    mask = ref["pop_background-centre"] & (distance >= low) & (distance < high)
                    if age is not None:
                        mask &= age >= 32
                    if mask.any():
                        bands[f"{low:g}-{high:g}"] = dict(n=int(mask.sum()), rms=float(np.sqrt(e2[mask].mean())))
                result["background_distance_bands"][f"{prefix}{stops}"] = bands
                base = ref["pop_background-centre"] & (distance >= 1.5)
                if age is not None:
                    base &= age >= 32
                magnitude = np.abs(error).mean(-1)
                extents = {}
                for threshold in (1, 4, 8):
                    selected = base & (magnitude > threshold)
                    count = int(selected.sum())
                    extents[str(threshold)] = dict(pixels=count,
                        p95_distance_px=float(np.quantile(distance[selected], .95)) if count else 0,
                        maximum_distance_px=float(distance[selected].max()) if count else 0)
                result["background_error_extent"][f"{prefix}{stops}"] = dict(values=extents, age32_filter_available=age is not None)
            result["measures"][f"{prefix}{stops}"] = pops
            if changed is not None and changed.any():
                result["changed"]["measures"][f"{prefix}{stops}"] = dict(
                    rms=float(np.sqrt(e2[changed].mean())), signed_mean=float(error[changed].mean()),
                    rms_noise_subtracted=float(np.sqrt(max(0, float((e2 - v2)[changed].mean())))),
                    se=float(signed_error[changed].std(ddof=1) / math.sqrt(int(changed.sum()))) if changed.sum() > 1 else 0)
            if previous is not None and previous[0] == i - 1:
                d_reference = reference - codes(previous[2], stops)
                residual = candidate - codes(previous[1], stops) - d_reference
                step = candidate - codes(previous[1], stops)
                covered = (ref["pop_all-covered"] if "pop_all-covered" in ref else ref["pop_image"]) & previous[3]
                if "hit" in ref and previous[4] is not None:
                    covered &= ref["hit"][..., 0] == previous[4]
                if age is not None:
                    covered &= age >= 8
                residuals.append(dict(frames=[i - 1, i], channel=f"{prefix}{stops}", pixels=int(covered.sum()),
                     phase=p.get("phase"), static=camera_key(p) == camera_key(recipe["frames"][i - 1]),
                     rms=float(np.sqrt((residual[covered] ** 2).mean())) if covered.any() else None,
                     step_rms=float(np.sqrt((step[covered] ** 2).mean())) if covered.any() else None,
                     age_filter_available=age is not None))
        previous = (i, hdr, mean, ref["pop_all-covered"] if "pop_all-covered" in ref else ref["pop_image"],
                    ref["hit"][..., 0].copy() if "hit" in ref else None)
        rows.append(result)
        if len(rows) % 16 == 0:
            print("frames scored", len(rows), "through", i, flush=True)
    for pool in transient.values():
        pool["rms"] = math.sqrt(pool["e2"] / pool["n"])
        pool["rms_noise_subtracted"] = math.sqrt(max(0, (pool["e2"] - pool["reference_v2"]) / pool["n"]))
    out = Path(args.out)
    out.write_text(json.dumps(dict(rows=rows, missing_frames=missing, motion_residuals=residuals,
        transient_pools=transient, case=args.case, candidate=str(candidate_dir), candidate_sha256=candidates,
        age_protocol="historical_t1b", script_sha256=SCRIPT_SHA,
        compared_with="U0b physical, disk-integrated multi-bounce reference, zero shutter, no fitting"),
        indent=1, allow_nan=False) + "\n")
    print("scored", len(rows), "frames; missing references", len(missing))


def rows_of(score):
    """The candidate's comparison rows, keyed as m7-tr0-bar.py keys TSR's."""
    out = {}

    def metric(frame, channel, population, measure, value, n=None):
        if value is not None:
            out[(str(frame), channel, population, measure)] = (value, n)
    for row in score["rows"]:
        frame = row["frame"]
        for channel, populations in row["measures"].items():
            for population, values in populations.items():
                for name in ("rms", "rms_noise_subtracted", "maximum", "signed_mean"):
                    if name in values:
                        metric(frame, channel, population, "absolute_signed_mean" if name == "signed_mean" else name,
                               abs(values[name]) if name == "signed_mean" else values[name], values["n"])
                shared = values.get("shared_age32")
                if shared:
                    metric(frame, channel, population + "/age32", "rms_noise_subtracted", shared["rms_noise_subtracted"], shared["n"])
                    metric(frame, channel, population + "/age32", "absolute_signed_mean", abs(shared["signed_mean"]), shared["n"])
        for channel, bands in row["background_distance_bands"].items():
            for band, value in bands.items():
                metric(frame, channel, "background-distance/" + band, "rms", value["rms"])
        for channel, extent in row["background_error_extent"].items():
            for threshold, values in extent["values"].items():
                for name, value in values.items():
                    metric(frame, channel, "background-error-over-" + threshold, name, value)
        for channel, values in row.get("changed", {}).get("measures", {}).items():
            for name in ("rms", "rms_noise_subtracted", "signed_mean"):
                metric(frame, channel, "changed/8-frame-input", "absolute_signed_mean" if name == "signed_mean" else name,
                       abs(values[name]) if name == "signed_mean" else values[name])
    for row in score["motion_residuals"]:
        for name in ("rms", "step_rms"):
            metric(row["frames"][1], row["channel"], "covered/same-instance/age8",
                   "reference_subtracted_flicker_rms" if name == "rms" else "frame_step_rms", row[name])
    for name, values in score["transient_pools"].items():
        event, age, population, channel = name.split("/")
        for measure in ("rms", "rms_noise_subtracted"):
            metric("pooled-age-" + age, channel, population + "/" + event, measure, values[measure], values["n"])
    return out


def compare(args):
    score = json.loads(Path(args.score).read_text())
    mine = rows_of(score)
    bar = {"tsr": {}, "t1b-black": {}}
    pixels = {}
    with (TR0 / "bar/physical-measures.csv").open(newline="", encoding="utf-8") as stream:
        for r in csv.DictReader(stream):
            if r["label"] == score["case"] and r["method"] in bar:
                key = (r["frame"], r["channel"], r["population"], r["measure"])
                bar[r["method"]][key] = float(r["value"])
                if r["pixels"]:
                    pixels[key] = int(r["pixels"])
    # Same populations: every shared row's pixel count must equal TR0's.
    mismatched = [list(k) for k, n in pixels.items() if k in mine and mine[k][1] is not None and mine[k][1] != n]
    result_populations = dict(checked=sum(k in mine and mine[k][1] is not None for k in pixels), mismatched=len(mismatched),
                              examples=mismatched[:20])
    mine = {k: v[0] for k, v in mine.items()}
    result = dict(case=score["case"], candidate=score["candidate"], score=args.score, script_sha256=SCRIPT_SHA,
                  populations=result_populations)
    print("population pixel counts checked", result_populations["checked"], "mismatched", result_populations["mismatched"])
    for method, rows in bar.items():
        wins, losses, ties, absent = [], [], 0, []
        for key, theirs in rows.items():
            ours = mine.get(key)
            if ours is None:
                absent.append(key)
            elif ours < theirs:
                wins.append((key, ours, theirs))
            elif ours > theirs:
                losses.append((key, ours, theirs))
            else:
                ties += 1
        groups = {}
        for kind, items in (("win", wins), ("loss", losses)):
            for (frame, channel, population, measure), ours, theirs in items:
                group = f"{population.split('/')[0] if not population.startswith(('revealed', 'start')) else population}|{measure}|{channel}"
                g = groups.setdefault(group, dict(win=0, loss=0, worst_ratio=0.0, worst=None))
                g[kind] += 1
                if kind == "loss":
                    ratio = ours / theirs if theirs > 0 else math.inf
                    if ratio > g["worst_ratio"]:
                        g["worst_ratio"], g["worst"] = ratio, [frame, ours, theirs]
        result[method] = dict(rows=len(rows), wins=len(wins), losses=len(losses), ties=ties, absent=len(absent),
                              absent_examples=[list(k) for k in absent[:20]], groups=groups,
                              worst_losses=[[list(k), o, t] for k, o, t in sorted(losses, key=lambda x: -(x[1] / x[2] if x[2] > 0 else 1e9))[:60]])
        print(f"{method}: rows {len(rows)} wins {len(wins)} losses {len(losses)} ties {ties} absent {len(absent)}")
    Path(args.out).write_text(json.dumps(result, indent=1) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("score")
    p.add_argument("candidate")
    p.add_argument("--case", required=True, help="TR0 comparator case (w-e-r-c1 RT, w-e-n-c2 no-RT)")
    p.add_argument("--out", required=True)
    p = sub.add_parser("compare")
    p.add_argument("score")
    p.add_argument("--out", required=True)
    args = parser.parse_args()
    dict(score=score, compare=compare)[args.command](args)


if __name__ == "__main__":
    main()
