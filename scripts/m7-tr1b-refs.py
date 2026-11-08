"""M7-TR1b: the retained, compact reference set in E:/renderlab-work/m7-refs (docs/milestone-7.md,
retention correction 2026-11-08; docs/m7-tr1b.md).

  disocclusion   CPU. Q-disocclusion: TR0's fixed external geometry ages (m7-tr0-s2g.py's
                 nearest-centre / same-instance / 2% depth protocol) from U0b's accepted
                 references, then each reference compacted.
  compact-check  CPU. One archive: compact size and the mean-code/noise change it causes.

Compact format (m7-refs-compact-v1): L0, L1, D0, D1 as float16 already exposed at the
track's registered exposure (the scorer's 2^stops factor applied before rounding); V0, V1,
visibility float16; hit0 the centre placement (uint32); every pop_* mask unchanged. World,
normals, depth and counts are dropped (ages are generated first). TR0's arithmetic needs
nothing else. Half precision moves scored mean codes by about 0.005 RMS (docs/m7-tr1b.md).
"""
import argparse
import hashlib
import importlib.util
import io
import json
from pathlib import Path

import numpy as np

REPO = Path(__file__).resolve().parents[1]
REFS = Path("E:/renderlab-work/m7-refs")
TR0 = Path("E:/renderlab-work/m7-tr0")
QUARRY = Path("E:/renderlab-work/m7-u0b/reference/quarry-final")
FORMAT = "m7-refs-compact-v1"
SCRIPT_SHA = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), REPO / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=1, allow_nan=False) + "\n", encoding="utf-8")


def codes(hdr, stops):
    x = np.maximum(0, hdr * (2 ** stops)); x = x / (1 + x)
    return 255 * np.where(x <= .0031308, x * 12.92, 1.055 * np.power(x, 1 / 2.4) - .055)


def compact_arrays(arrays, exposure_stops):
    """The scorer's arrays at half precision, exposed; masks and the centre placement exact."""
    scale = np.float32(2 ** exposure_stops)
    out = {}
    for k in ("L0", "L1", "D0", "D1"):
        exposed = arrays[k].astype("f4") * scale
        if np.abs(exposed).max() >= 65504:
            raise ValueError(f"{k} exceeds float16 after exposure")
        out[k] = exposed.astype("f2")
    for k in ("V0", "V1", "visibility"):
        if k in arrays:
            out[k] = arrays[k].astype("f2")
    out["hit0"] = np.ascontiguousarray(arrays["hit"][..., 0])
    out.update({k: v for k, v in arrays.items() if k.startswith("pop_")})
    return out


def compaction_error(arrays, compact, exposure_stops):
    """Mean-code change on all-covered pixels and the split-half noise before/after, L0/L4."""
    scale = np.float32(2 ** exposure_stops)
    mask = arrays["pop_all-covered"] if "pop_all-covered" in arrays else arrays["pop_image"]
    result = {}
    for stops in (0, 4):
        full = [arrays[k].astype("f4") * scale for k in ("L0", "L1")]
        half = [compact[k].astype("f4") for k in ("L0", "L1")]
        d = codes((half[0] + half[1]) * .5, stops) - codes((full[0] + full[1]) * .5, stops)
        noise = lambda h: np.sqrt(((((codes(h[0], stops) - codes(h[1], stops)) * .5) ** 2).mean(-1))[mask].mean())
        result[f"L{stops}"] = dict(mean_code_change_rms=float(np.sqrt((d[mask] ** 2).mean())),
                                   mean_code_change_max=float(np.abs(d).max()),
                                   noise_rms_f32=float(noise(full)), noise_rms_f16=float(noise(half)))
    return result


def write_compact(destination, arrays, meta, exposure_stops, source):
    compact = compact_arrays(arrays, exposure_stops)
    destination.parent.mkdir(parents=True, exist_ok=True)
    np.savez_compressed(destination, **compact)
    record = dict(meta, compact_format=FORMAT, exposure_stops_applied=exposure_stops,
                  archive_sha256=sha(destination), source_archive=str(source), source_archive_sha256=sha(source),
                  source_archive_provenance_sha256=meta.get("archive_sha256"),
                  compaction=compaction_error(arrays, compact, exposure_stops), compactor_sha256=SCRIPT_SHA)
    save(destination.with_suffix(".json"), record)
    return record


def disocclusion(args):
    """Ages from the full archives (world/hit/depth), then compact copies; sources untouched."""
    s2g = load("m7-tr0-s2g")
    inputs = TR0 / "inputs/q-disocclusion.json"
    frames = json.loads(inputs.read_text())["frames"]
    key = lambda p: np.asarray(p["eye"] + p["target"], "<f4").tobytes()
    sources = {}
    for path in QUARRY.glob("q-disocclusion-*.json"):
        meta = json.loads(path.read_text())
        if meta.get("accepted_for_scoring") is not True or meta.get("camera_background") != [0., 0., 0.]:
            raise ValueError("unaccepted or non-black-primary reference " + str(path))
        frame = meta["frame"]
        if frame["open"] != frame["close"] or frame.get("sky_only"):
            raise ValueError("zero-shutter frame reference required")
        sources[key(frame["open"])] = (path.with_suffix(".npz"), meta)
    out = REFS / "quarry"
    ages_dir = REFS / "ages/q-disocclusion"
    records, previous, age = [], None, None
    for i, pose in enumerate(frames):
        archive, meta = sources[key(pose)]
        if meta.get("archive_sha256") and sha(archive) != meta["archive_sha256"]:
            raise ValueError("reference bytes differ from provenance " + str(archive))
        with np.load(archive) as z:
            arrays = {k: z[k] for k in z.files}
        current = {k: arrays[k] for k in ("world", "hit", "depth")}
        if i == 0:
            age = np.zeros(current["depth"].shape, "u1")
        else:
            age = s2g.advance_age(previous, current, frames[i - 1], pose, age)
        destination = ages_dir / f"f{i:03}.npz"
        destination.parent.mkdir(parents=True, exist_ok=True)
        np.savez_compressed(destination, age=age)
        records.append(dict(track="q-disocclusion", frame=i, reference=archive.name,
                            age_sha256=hashlib.sha256(age.tobytes()).hexdigest(),
                            age0=int((age == 0).sum()), age32=int((age >= 32).sum())))
        name = f"q-disocclusion-{i:03}"
        record = write_compact(out / (name + ".npz"), arrays, meta, -14.0, archive)
        print(name, "compact", (out / (name + ".npz")).stat().st_size, "L0 change",
              round(record["compaction"]["L0"]["mean_code_change_rms"], 5), "age0", records[-1]["age0"], flush=True)
        previous = current
    save(ages_dir.parent / "protocol-q-disocclusion.json", dict(records=records,
         source_sequences={"q-disocclusion": sha(inputs)}, script_sha256=SCRIPT_SHA,
         advance_age_source=str(REPO / "scripts/m7-tr0-s2g.py"), advance_age_sha256=sha(REPO / "scripts/m7-tr0-s2g.py"),
         definition="TR0's fixed external benchmark ages (m7-tr0-s2g.py advance_age): nearest previous centre, same "
                    "instance, 2% forward depth; age0 on rejection, +1 on acceptance capped 255; black infinity "
                    "same-pixel for the translation-only track. Offline scoring only."))


def compact_check(args):
    with np.load(args.archive) as z:
        arrays = {k: z[k] for k in z.files}
    buffer = io.BytesIO()
    compact = compact_arrays(arrays, args.exposure)
    np.savez_compressed(buffer, **compact)
    print(json.dumps(dict(bytes=len(buffer.getvalue()), source_bytes=Path(args.archive).stat().st_size,
                          error=compaction_error(arrays, compact, args.exposure)), indent=1))


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("disocclusion")
    p = sub.add_parser("compact-check")
    p.add_argument("archive")
    p.add_argument("--exposure", type=float, default=-14.0)
    args = parser.parse_args()
    {"disocclusion": disocclusion, "compact-check": compact_check}[args.command](args)


if __name__ == "__main__":
    main()
