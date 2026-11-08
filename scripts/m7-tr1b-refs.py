"""M7-TR1b: the retained, compact reference set in E:/renderlab-work/m7-refs (docs/milestone-7.md,
retention correction 2026-11-08; docs/m7-tr1b.md).

  disocclusion   CPU. Q-disocclusion: TR0's fixed external geometry ages (m7-tr0-s2g.py's
                 nearest-centre / same-instance / 2% depth protocol) from U0b's accepted
                 references, then each reference compacted.
  s2g            GPU batches. S2G's 288 physical states with U0b's qualified D8 tracer, each
                 scene built exactly as TR0's `reference` built it, under m7-tr1's own leases
                 (scripts/m7-tr1-wait.py); every retained array is checked against TR0's
                 recorded per-array SHA-256 of its qualified corpus, then compacted.
  qsun           GPU, one state per lease. Q-sun's distinct sun states with quarry-final's
                 U0b tracer and replay policy (each state its own lighting file), compacted;
                 then the static camera's fixed ages.
  tsr            Unreal. TSR Epic on a quarry track (TR0's `run`, U0's original package),
                 kept as lossless native frames plus TR0's exposure factors.
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
import os
import re
import shutil
import subprocess
import time
import uuid
import zipfile
from pathlib import Path

import numpy as np

REPO = Path(__file__).resolve().parents[1]
REFS = Path("E:/renderlab-work/m7-refs")
TR0 = Path("E:/renderlab-work/m7-tr0")
QUARRY = Path("E:/renderlab-work/m7-u0b/reference/quarry-final")
FORMAT = "m7-refs-compact-v1"
D8 = "d8bd9a8e1e1c7b5d39ee04d70c01ed4ee87bb654463db2b40daec1a96f35f139"
S2G_SCENE = Path("E:/renderlab-work/m7-u0b/scenes/S2G/scene.json")
TR0_S2G = TR0 / "s2g/physical/d8bd9a8e"
S2G_TRACKS = ("s2g-camera-0.25px", "s2g-camera-0.5px", "s2g-camera-1px", "s2g-sun-0.25deg")
BIN = Path("E:/renderlab-work/m7-tr1/bin")
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


def available_gib():
    """Available physical memory (Windows), the measure the lease guard uses."""
    import ctypes

    class Status(ctypes.Structure):
        _fields_ = [("length", ctypes.c_ulong), ("load", ctypes.c_ulong), ("total", ctypes.c_ulonglong),
                    ("available", ctypes.c_ulonglong), ("page_total", ctypes.c_ulonglong), ("page_available", ctypes.c_ulonglong),
                    ("virtual_total", ctypes.c_ulonglong), ("virtual_available", ctypes.c_ulonglong), ("extended", ctypes.c_ulonglong)]
    status = Status()
    status.length = ctypes.sizeof(Status)
    ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status))
    return status.available / 2 ** 30


def member_hashes(path):
    with zipfile.ZipFile(path) as z:
        return {i.filename: hashlib.sha256(z.read(i)).hexdigest() for i in z.infolist()}


def s2g(args):
    """Batches of S2G physical states: trace (own lease), U0b compaction, identity, f16 copy."""
    binary = BIN / "u0b-d8bd9a8e.exe"
    if not binary.exists():
        shutil.copyfile(TR0 / "binaries" / (D8 + ".exe"), binary)
    if sha(binary) != D8:
        raise ValueError("pin the U0b-qualified D8 binary")
    first = json.loads(next(TR0_S2G.glob("s2g-*.json")).read_text())
    if sha(S2G_SCENE) != first["source_scene_sha256"]:
        raise ValueError("S2G source differs from TR0's qualified corpus")
    scene = json.loads(S2G_SCENE.read_text())
    registration = json.loads((TR0 / "s2g/plan.json").read_text())
    frames = {t: json.loads((TR0 / "inputs" / (t + ".json")).read_text())["frames"] for t in S2G_TRACKS}
    instances = {name: [i + 1 for i, mesh in enumerate(scene["meshes"]) if mesh["name"] in names]
                 for name, names in scene["populations"].items()}
    out = REFS / "s2g"
    out.mkdir(parents=True, exist_ok=True)
    compactor = load("m7-u0b-score")
    clear_interrupted(REFS / "work")

    def trace(batch, label):
        """One lease for `batch`; returns the states whose arrays are not byte-identical to
        TR0's qualified archive (not written). U0b qualified D8 across independent
        acceleration-structure builds only to within a few ULP of colour (bounded-as), so
        byte identity holds for most states, not all."""
        work = REFS / "work" / label
        data = work / "data"
        data.mkdir(parents=True)
        cases, entries = [], []
        for c in batch:
            p = frames[c["track"]][c["frame"]]
            # TR0's `reference` construction, byte for byte (its generated-scene SHA-256 is checked).
            copy_scene = dict(scene, poses=[dict(name=c["track"].removeprefix("s2g-") + f"-{c['frame']:03}",
                                                 eye=p["eye"], target=p["target"])])
            copy_scene["sun"] = dict(scene["sun"], to_light=p["to_light"])
            destination = work / "inputs" / str(len(cases)) / "s2g" / "scene.json"
            destination.parent.mkdir(parents=True)
            destination.write_text(json.dumps(copy_scene, separators=(",", ":")), encoding="utf-8")
            name = f"{c['track']}-{c['frame']:03}"
            qualified = TR0_S2G / (name + ".json")
            expected = json.loads(qualified.read_text()) if qualified.exists() else None
            # The bench records the hash of the scene file it loaded.
            if expected and expected["scene_sha256"] != sha(destination):
                raise ValueError("generated scene differs from TR0's: " + name)
            cases.append(dict(case=c, name=name, pose=p, input=destination, expected=expected))
            entries.append(f"{destination.parent}@1024")
        plan = dict(steps=[dict(name="trace", binary="u0b-d8bd9a8e", test_name="reference::tests::gpu_reference_benches",
                                env={"RENDERLAB_M7_BENCHES": ";".join(entries), "RENDERLAB_M7_REFERENCE_OUT": str(data),
                                     "RENDERLAB_M7_CAPTURE_GEOMETRY": "1", "RAYON_NUM_THREADS": "2"})])
        (work / "plan.json").write_text(json.dumps(plan, indent=1))
        for attempt in range(6):
            subprocess.run(["py", "-3", "-B", str(REPO / "scripts/m7-tr1-wait.py"), f"{label}-{attempt}",
                            str(work / "plan.json"), "--minutes", "10", "--wait-minutes", "120",
                            "--note", "U0b D8 S2G reference batch, functional, no timing claim"], cwd=REPO)
            if (data / "replay.json").exists():
                break
        replay = json.loads((data / "replay.json").read_text())
        # The bench replays its first case from an independent rebuild. D8 differs there by a
        # few ULP on some runs (bounded-AS); the bench then stops, so later cases were not
        # rendered and stay pending.
        replay_identical = replay["bit_identical"]
        rendered = [k for k in cases if (data / (k["name"] + ".json")).exists()]
        removed, mismatched = [], []
        for k in rendered:
            name, p, c = k["name"], k["pose"], k["case"]
            meta_path = data / (name + ".json")
            meta = json.loads(meta_path.read_text())
            if meta["scene_sha256"] != sha(k["input"]) or meta["invalid_pixels"]:
                raise ValueError("scene identity or finite-pixel check failed: " + name)
            pose = {k2: p[k2] for k2 in ("eye", "target")}
            frame = dict(name=name, open=pose, close=pose, to_light=p["to_light"], fov_y=p["fov_y_radians"],
                         width=1920, height=1080, sky_only=False)
            meta.update(frame=frame, bench="S2G", camera_background=[0., 0., 0.], reference_binary_sha256=D8,
                        source_scene_sha256=sha(S2G_SCENE), population_instances=instances)
            save(meta_path, meta)
            compactor.compact(argparse.Namespace(root=data, name=name))
            archive = data / (name + ".npz")
            members = member_hashes(archive)
            differing = []
            if k["expected"]:
                differing = [m for m, h in k["expected"]["retained_array_sha256"].items() if members.get(m) != h]
            # Accepted when byte-identical to TR0's qualified archive, or, traced alone, when its
            # own independent-rebuild replay is bit-identical (TR0's per-state criterion).
            if differing and not (len(cases) == 1 and replay_identical):
                mismatched.append(c)
            else:
                with np.load(archive) as z:
                    arrays = {m: z[m] for m in z.files}
                meta = json.loads(meta_path.read_text())
                if not differing and k["expected"]:
                    identity = (f"all {len(k['expected']['retained_array_sha256'])} retained arrays byte-identical to "
                                f"TR0's qualified D8 corpus ({TR0_S2G / (name + '.json')})")
                else:
                    identity = ("traced alone, its own independent-rebuild replay bit-identical (TR0's per-state "
                                "criterion); differs from TR0's archive in " + ",".join(differing or ["(no TR0 record)"])
                                + " at the bounded-AS level")
                meta.update(accepted_for_scoring=True, corpus_key=c["key"], aliases=c["aliases"], batch=label,
                            batch_replay_bit_identical=replay_identical, identity=identity, tr0_differing_arrays=differing)
                write_compact(out / (name + ".npz"), arrays, meta, float(scene["exposure_stops"]), archive)
                print(name, "compact", (out / (name + ".npz")).stat().st_size, "differs" if differing else "identical", flush=True)
            for f in sorted(data.glob(name + "*")):
                if f.suffix in (".npz", ".f32", ".bin", ".png", ".pending"):
                    removed.append(dict(path=str(f), bytes=f.stat().st_size, sha256=sha(f)))
                    f.unlink()
        for k in cases:
            removed.append(dict(path=str(k["input"]), bytes=k["input"].stat().st_size, sha256=sha(k["input"])))
            k["input"].unlink()
        save(work / "removed.json", removed)
        return mismatched

    done = 0
    while done < args.limit:
        pending = [c for c in registration["physical"] if not (out / f"{c['track']}-{c['frame']:03}.npz").exists()]
        if not pending:
            break
        # About 0.35 GiB of raw output per state until it is compacted; the runner's preflight
        # needs 5 GiB free on the output disk.
        if shutil.disk_usage(REFS).free < args.batch * 0.35 * 1024 ** 3 + 5.5 * 1024 ** 3:
            raise SystemExit("deferred: disk headroom below the batch's raw output plus 5.5 GiB")
        batch = pending[:min(args.batch, args.limit - done)]
        for c in trace(batch, f"s2g-{uuid.uuid4().hex[:8]}"):
            for attempt in range(3):
                if not trace([c], f"s2g1-{uuid.uuid4().hex[:8]}"):
                    break
            else:
                raise ValueError(f"{c['track']}-{c['frame']:03}: neither TR0's bytes nor a bit-identical replay in 3 tries")
        done += len(batch)
        print("S2G batch done", done, "remaining", len(pending) - len(batch), flush=True)


def clear_interrupted(root):
    """Raw tracer output of a batch that stopped before compaction (no removed.json): this
    tool's own intermediates, removed with their hashes recorded."""
    for work in sorted(root.glob("s2g*")):
        if (work / "removed.json").exists() or not work.is_dir():
            continue
        removed = [dict(path=str(f), bytes=f.stat().st_size, sha256=sha(f)) for f in sorted(work.rglob("*"))
                   if f.is_file() and f.suffix in (".npz", ".f32", ".bin", ".png", ".pending", ".json")
                   and f.name != "plan.json"]
        for record in removed:
            Path(record["path"]).unlink()
        save(work / "removed.json", dict(removed=removed, reason="interrupted batch; its states are traced again"))
        print("cleared interrupted batch", work.name, len(removed), "files", flush=True)


def qsun(args):
    """Q-sun physical states with U0b's quarry tracer (997151eb, bounded-as-v1 replay, as
    quarry-final), one lease and one sun per state; then fixed ages for the static camera."""
    binary = BIN / "u0b-997151eb.exe"
    source_binary = Path("E:/renderlab-work/m7-u0b/bin/reference-tests-997151eb.exe")
    if not binary.exists():
        shutil.copyfile(source_binary, binary)
    if not sha(binary).startswith("997151eb"):
        raise ValueError("pin quarry-final's U0b binary")
    base = json.loads(Path("E:/renderlab-work/m7-u0b/quarry-unit-sequence.json").read_text())
    lighting = json.loads(Path(base["lighting"]).read_text())
    inputs = TR0 / "inputs" / (args.track + ".json")
    frames = json.loads(inputs.read_text())["frames"]
    key = lambda p: np.asarray(p["eye"] + p["target"] + p["to_light"], "<f4").tobytes()
    states = {}
    for i, p in enumerate(frames):
        states.setdefault(key(p), (i, p))
    out = REFS / "quarry"
    compactor = REPO / "scripts/m7-u0b-score.py"
    for i, p in [s for _ in range(4) for s in states.values()]:
        name = f"{args.track}-{i:03}"
        if (out / (name + ".npz")).exists():
            continue
        if shutil.disk_usage(REFS).free < 7 * 1024 ** 3:
            raise SystemExit("deferred: less than 7 GiB free")
        # The quarry tracer holds the whole asset in memory: start its lease only with room,
        # or the 2 GiB guard stops it.
        while available_gib() < 9:
            time.sleep(5)
        label = f"qsun-{uuid.uuid4().hex[:8]}"
        work = REFS / "work" / label
        data = work / "data"
        data.mkdir(parents=True)
        light = dict(lighting, lights=[dict(lighting["lights"][0], travel=[-x for x in p["to_light"]])])
        (work / "lighting.json").write_text(json.dumps(light, indent=1))
        pose = {k: p[k] for k in ("eye", "target")}
        manifest = dict(base, lighting=str(work / "lighting.json"),
                        frames=[dict(name=name, open=pose, close=pose, fov_y=p["fov_y_radians"], width=1920, height=1080,
                                     samples=1024, sky_only=False)])
        (work / "manifest.json").write_text(json.dumps(manifest, indent=1))
        plan = dict(steps=[dict(name="trace", binary="u0b-997151eb", test_name="reference::quarry_tests::gpu_reference_quarry",
                                env={"RENDERLAB_M7_QUARRY_MANIFEST": str(work / "manifest.json"),
                                     "RENDERLAB_M7_REFERENCE_OUT": str(data), "RENDERLAB_M7_QUARRY_REPLAY_POLICY": "bounded-as-v1",
                                     "RENDERLAB_M7_REFERENCE_BATCH": "1", "RENDERLAB_M7_DEFER_REFERENCE_COMPACT": "1"})])
        (work / "plan.json").write_text(json.dumps(plan, indent=1))
        for attempt in range(6):
            subprocess.run(["py", "-3", "-B", str(REPO / "scripts/m7-tr1-wait.py"), f"{label}-{attempt}", str(work / "plan.json"),
                            "--minutes", "8", "--wait-minutes", "120",
                            "--note", "U0b quarry Q-sun reference state, functional, no timing claim"], cwd=REPO)
            if (data / (name + ".compact-pending.json")).exists():
                break
        else:
            print(name, "not rendered; left pending for a later pass", flush=True)
            continue
        environment = {k: v for k, v in os.environ.items() if k != "RENDERLAB_M7_DEFER_REFERENCE_COMPACT"}
        subprocess.run(["py", "-3", "-B", str(compactor), "compact", str(data), name], cwd=REPO, env=environment, check=True)
        meta = json.loads((data / (name + ".json")).read_text())
        if meta.get("accepted_for_scoring") is not True or meta.get("camera_background") != [0., 0., 0.]:
            raise ValueError(name + ": not accepted or not black-primary")
        meta["frame"] = dict(meta["frame"], to_light=p["to_light"])
        meta.update(lighting=light, reference_binary_sha256=sha(binary), source_inputs=str(inputs), source_inputs_sha256=sha(inputs))
        with np.load(data / (name + ".npz")) as z:
            arrays = {k: z[k] for k in z.files}
        write_compact(out / (name + ".npz"), arrays, meta, -14.0, data / (name + ".npz"))
        removed = []
        for f in sorted(data.iterdir()):
            if f.suffix in (".npz", ".f32", ".bin", ".png"):
                removed.append(dict(path=str(f), bytes=f.stat().st_size, sha256=sha(f)))
                f.unlink()
        save(work / "removed.json", removed)
        print(name, "compact", (out / (name + ".npz")).stat().st_size, flush=True)
    # The camera never moves: TR0's identical-view rule ages every pixel by one per frame.
    ages_dir = REFS / "ages" / args.track
    ages_dir.mkdir(parents=True, exist_ok=True)
    for i, p in enumerate(frames):
        if np.asarray(p["eye"] + p["target"], "<f4").tobytes() != np.asarray(frames[0]["eye"] + frames[0]["target"], "<f4").tobytes():
            raise ValueError("Q-sun ages assume a static camera")
        np.savez_compressed(ages_dir / f"f{i:03}.npz", age=np.full((1080, 1920), min(i, 255), "u1"))
    save(REFS / "ages" / f"protocol-{args.track}.json", dict(track=args.track, frames=len(frames), source_sequences={args.track: sha(inputs)},
         definition="Static camera: TR0's identical-view rule, every pixel age + 1 per frame from 0, capped 255; Sun changes do not reset geometric age."))


def tsr(args):
    """TSR Epic on a quarry track with TR0's `run` (U0's original package, the one TR0's Q-walk
    captures used), rooted in m7-refs/tsr under m7-tr1's leases. The capture is kept as each
    frame's native TSR output (R11G11B10, exact in float16) plus TR0's exposure factor, which
    returns it to registered display units without fitting; the raw dump is then deleted."""
    tr0 = load("m7-tr0")
    root = REFS / "tsr"
    (root / "inputs").mkdir(parents=True, exist_ok=True)
    for name in (args.track + ".json", "manifest.json"):
        shutil.copyfile(TR0 / "inputs" / name, root / "inputs" / name)
    tr0.ROOT = root
    tr0.U0.OUT, tr0.U0.OWNER = root, "m7-tr1"
    out = root / "runs" / args.label
    if not (out / "profile.csv").exists():
        tr0.run(argparse.Namespace(label=args.label, track=args.track, path=args.path, quality=3, capture=True, smoke=False,
                                   program="full", minutes=args.minutes, wait_seconds=args.wait_seconds, package_dir=None))
    if not (out / "capture-readback.json").exists():
        # TR0's CPU readback: frame coverage, 60 Hz deltas, exposure and identity records.
        tr0.summarize(argparse.Namespace(label=args.label))
    recipe = json.loads((out / "recipe.json").read_text())
    capture = json.loads((out / "capture-readback.json").read_text())
    if tr0.digest(tr0.U0.PACKAGE) != recipe["package_exe_sha256"] or not recipe["package_exe_sha256"].startswith("69d6f88a"):
        raise ValueError("capture must use TR0's original U0 package")
    dump = tr0.NATIVE.s1.native.Dump(out / "dump")
    groups = {}
    for event in dump.passes:
        frame = next(int(m.group(1)) for s in event["ParentEventScopes"] if (m := re.match(r"Frame (\d+)", s)))
        groups.setdefault(frame, []).append(event)
    if len(groups) != len(recipe["frames"]):
        raise ValueError("native frame coverage mismatch")
    frames_dir = out / "frames"
    frames_dir.mkdir(exist_ok=True)
    registered = 2 ** recipe.get("registered_exposure_stops", recipe.get("exposure_stops", -14))
    factors = {}
    for i, (_, events) in enumerate(sorted(groups.items())):
        consumer, name = tr0.colour_input(dump, events, recipe)
        hdr, identity = tr0.NATIVE.resource(dump, name, consumer)
        if identity["sha256"] != capture["identities"][i]["hdr"]["sha256"]:
            raise ValueError("candidate bytes changed since capture readback")
        rgb = hdr[..., :3].astype("f4")
        if not np.array_equal(rgb.astype("f2").astype("f4"), rgb):
            raise ValueError("native TSR output is not exact in float16")
        rgba = np.concatenate([rgb, np.ones(rgb.shape[:2] + (1,), "f4")], -1).astype("<f2")
        (frames_dir / f"f{i:03}.rgba16f").write_bytes(rgba.tobytes())
        factors[str(i)] = registered / capture["identities"][i]["exposure"]["values"][0]
    save(frames_dir / "exposure.json", dict(factors=factors, definition="multiply the native frame by this factor: "
         "registered 2^stops over the frame's measured pre-exposure (TR0's score)", script_sha256=SCRIPT_SHA))
    removed = []
    for f in sorted((out / "dump").rglob("*")):
        if f.is_file():
            removed.append(dict(path=str(f), bytes=f.stat().st_size, sha256=sha(f)))
    shutil.rmtree(out / "dump")
    save(out / "deleted-raw.json", dict(removed=removed, retained="frames/ (lossless native TSR output) and exposure.json"))
    print("TSR frames", len(factors), frames_dir, flush=True)


def pack_tsr(args):
    """A scored TSR capture's native frames into one compressed archive (byte-identical frames,
    checked), then the loose frames removed; scripts/m7-tr1-score.py reads either form."""
    frames = REFS / "tsr/runs" / args.label / "frames"
    loose = sorted(frames.glob("f*.rgba16f"))
    arrays = {f.stem: np.frombuffer(f.read_bytes(), "<f2").reshape(1080, 1920, 4) for f in loose}
    np.savez_compressed(frames / "frames.npz", **arrays)
    with np.load(frames / "frames.npz") as z:
        if any(z[f.stem].astype("<f2").tobytes() != f.read_bytes() for f in loose):
            raise ValueError("packed frames differ")
    removed = [dict(path=str(f), bytes=f.stat().st_size, sha256=sha(f)) for f in loose]
    for f in loose:
        f.unlink()
    save(frames / "packed.json", dict(archive_sha256=sha(frames / "frames.npz"), frames=removed))
    print(args.label, "packed", len(loose), (frames / "frames.npz").stat().st_size, flush=True)


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
    p = sub.add_parser("s2g")
    p.add_argument("--batch", type=int, default=10)
    p.add_argument("--limit", type=int, default=288)
    p = sub.add_parser("qsun")
    p.add_argument("--track", default="q-sun-fast")
    p = sub.add_parser("tsr")
    p.add_argument("label")
    p.add_argument("--track", default="q-disocclusion")
    p.add_argument("--path", choices=("hrt", "vsm-nort"), required=True)
    p.add_argument("--minutes", type=int, default=10)
    p.add_argument("--wait-seconds", type=int, default=5400)
    p = sub.add_parser("pack-tsr")
    p.add_argument("label")
    p = sub.add_parser("compact-check")
    p.add_argument("archive")
    p.add_argument("--exposure", type=float, default=-14.0)
    args = parser.parse_args()
    {"disocclusion": disocclusion, "s2g": s2g, "qsun": qsun, "tsr": tsr, "pack-tsr": pack_tsr,
     "compact-check": compact_check}[args.command](args)


if __name__ == "__main__":
    main()
