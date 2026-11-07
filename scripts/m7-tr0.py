"""TR0 measurement tooling; never edits renderer code or the U0 package.

plan creates immutable input timelines; run blocks on its own bounded, hidden
offscreen GPU lease; summarize and clean are CPU-only. Missing references and
T1b age masks are explicit gaps, never inferred from TSR's final colour.
"""
import argparse
import csv
import copy
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import re
import shutil
import subprocess
import uuid

import numpy as np
from PIL import Image

REPO = Path(__file__).resolve().parents[1]
ROOT = Path("E:/renderlab-work/m7-tr0")
T1 = Path("E:/renderlab-work/t1-gpu")


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), REPO / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


U0 = load("m7-u0-ue")
U0.OUT, U0.OWNER = ROOT, "m7-tr0"
_engine_tool = U0.engine_tool
U0.engine_tool = lambda p: _engine_tool(p) or p["Name"].lower() == "build-script-build.exe"
NATIVE = load("bench-motion-score")


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n", encoding="utf-8")


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def pose_commands(p):
    x, y, z = (-100 * p["eye"][2], 100 * p["eye"][0], 100 * p["eye"][1])
    d = np.array(p["target"]) - p["eye"]
    dx, dy, dz = -d[2], d[0], d[1]
    pitch = math.degrees(math.atan2(dz, math.hypot(dx, dy)))
    yaw = math.degrees(math.atan2(dy, dx))
    lines = [f'ke CameraActor K2_SetActorLocation "(X={x:.9f},Y={y:.9f},Z={z:.9f})" false () false',
             f'ke CameraActor K2_SetActorRotation "(Pitch={pitch:.9f},Yaw={yaw:.9f},Roll=0)" false']
    if "to_light" in p:
        d = -np.array(p["to_light"])
        pitch = math.degrees(math.atan2(d[1], math.hypot(d[0], d[2])))
        yaw = math.degrees(math.atan2(d[0], -d[2]))
        lines.append(f'ke DirectionalLight K2_SetActorRotation "(Pitch={pitch:.9f},Yaw={yaw:.9f},Roll=0)" false')
    return lines


def plan(_):
    ROOT.mkdir(parents=True, exist_ok=True)
    (ROOT / "inputs").mkdir(exist_ok=True)
    registered = json.loads((T1 / "sequences/registered.json").read_text())
    inputs = {}
    for name in ("q-walk", "q-disocclusion", "q-zoom"):
        source = T1 / "sequences" / (name + ".json")
        actual = digest(source)
        if actual != registered[name]["sha256"]:
            raise ValueError("T1b registered sequence changed: " + name)
        data = json.loads(source.read_text())
        destination = ROOT / "inputs" / source.name
        if destination.exists() and digest(destination) != actual:
            raise ValueError("refusing to overwrite a different registration")
        shutil.copyfile(source, destination)
        inputs[name] = dict(path=str(destination), sha256=actual, frames=len(data["frames"]),
                            t1b_scored=name == "q-walk")
    lighting = json.loads(Path(json.loads((T1 / "walk-b15.json").read_text())["lighting"]).read_text())
    # Preserve the quarry's actual directional vector when constructing light-event tracks.
    inputs["lighting"] = dict(path=json.loads((T1 / "walk-b15.json").read_text())["lighting"],
                              contents=lighting)
    base = json.loads((T1 / "cameras.json").read_text())["frames"][0]
    light = -np.array(lighting["lights"][0]["travel"])
    light /= np.linalg.norm(light)
    for name, count in (("q-sun-slow", 128), ("q-sun-fast", 64), ("q-sun-jump", 33)):
        frames = []
        for f in range(count):
            degrees = .25 * f / 60 if name == "q-sun-slow" else 10 * min(f, 31) / 60 if name == "q-sun-fast" else 5
            a = math.radians(degrees)
            p = copy.deepcopy(base)
            p.update(frame=f, phase="hold" if name == "q-sun-fast" and f >= 32 else "sun",
                     to_light=[light[0] * math.cos(a) + light[2] * math.sin(a), light[1],
                               light[2] * math.cos(a) - light[0] * math.sin(a)])
            frames.append(p)
        destination = ROOT / "inputs" / (name + ".json")
        save(destination, dict(frames=frames, registration="TR0 from T1 section 7; T1c was never run", hz=60, shutter=0))
        inputs[name] = dict(path=str(destination), sha256=digest(destination), frames=count, t1b_scored=False)
    save(ROOT / "inputs/manifest.json", dict(base_commit="708e87c", inputs=inputs,
         reference_gaps=["T1b walk compact reference buffers deleted", "T1b per-pixel ages deleted",
                         "U0 tracer has no quarry textures, RLE2 sky, or quarry RLA upload",
                         "T1b never ran Q-disocclusion, Q-zoom or Q-sun"],
         package=str(U0.PACKAGE), package_exe_sha256=digest(U0.PACKAGE)))
    print(json.dumps({k: {a: b for a, b in v.items() if a != "contents"} for k, v in inputs.items()}, indent=2))


def macro(out, name, lines):
    path = out / "macros" / (name + ".txt")
    path.parent.mkdir(exist_ok=True)
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return "exec " + Path(os.path.relpath(path, U0.PACKAGE.parent.parent)).as_posix()


def settings(quality, path):
    commands = [c for c in U0.BASE_COMMANDS + U0.CONFIGS["ue-tsr"]["commands"]
                if not c.startswith(("r.TSR.History.ScreenPercentage", "sg.AntiAliasingQuality"))]
    commands += [f"sg.AntiAliasingQuality {quality}", "r.ScreenPercentage 100", "r.DynamicRes.OperationMode 0",
                 "ShowFlag.MotionBlur 0", "r.MotionBlurQuality 0", "r.EyeAdaptation.PreExposureOverride 0",
                 "r.DumpGPU.Upload 0", "r.DumpGPU.Explore 0", "r.DumpGPU.Stream 0",
                 "r.DumpGPU.Texture 2", "r.DumpGPU.Buffer 2", "r.DumpGPU.PassParameters 1"]
    if path == "hrt":
        commands += U0.HRT_SHADOWS
    return commands


def readback(out, quality, path):
    rows = U0.cvar_rows(out / "cvars.csv")
    expected = {"sg.AntiAliasingQuality": str(quality), "r.AntiAliasingMethod": "4", "r.ScreenPercentage": "100",
                "r.EyeAdaptation.PreExposureOverride": "0", "r.RayTracing.Shadows": str(int(path == "hrt")),
                "r.DynamicRes.OperationMode": "0", "r.MotionBlurQuality": "0"}
    groups = U0.scalability_expectations()
    expected.update(groups.get(f"AntiAliasingQuality@{quality}", {}))
    checks = [dict(name=k, expected=v, actual=rows.get(k), ok=k in rows and U0.same(rows[k]["value"], v))
              for k, v in expected.items()]
    log = (out / "ue.log").read_text(encoding="utf-8-sig", errors="replace")
    cameras = [line for line in log.splitlines() if "CameraCachePrivate =" in line]
    recipe = json.loads((out / "recipe.json").read_text())
    camera_checks = []
    if recipe["capture"]:
        if len(cameras) != len(recipe["frames"]):
            raise ValueError("missing per-frame camera readback")
        for p, line in zip(recipe["frames"], cameras, strict=True):
            m = re.search(r"Location=\(X=([^,]+),Y=([^,]+),Z=([^\)]+)\),Rotation=\(Pitch=([^,]+),Yaw=([^,]+),Roll=([^\)]+)\),FOV=([^,]+)", line)
            if m is None:
                raise ValueError("camera readback syntax")
            x, y, z, pitch, yaw, roll, fov = map(float, m.groups())
            eye = np.array([y, z, -x]) / 100
            a, b = np.radians([pitch, yaw])
            forward = np.array([math.cos(a) * math.sin(b), math.sin(a), -math.cos(a) * math.cos(b)])
            desired = np.array(p["target"]) - p["eye"]
            desired /= np.linalg.norm(desired)
            fov_expected = math.degrees(2 * math.atan(math.tan(p["fov_y_radians"] / 2) * 1920 / 1080))
            good = np.max(np.abs(eye - p["eye"])) < 1e-6 and np.max(np.abs(forward - desired)) < 1e-6 and abs(fov - fov_expected) < 1e-5
            camera_checks.append(dict(frame=p["frame"], ok=bool(good), eye=eye.tolist(), forward=forward.tolist(), horizontal_fov=fov))
    result = dict(ok=all(c["ok"] for c in checks + camera_checks), checks=checks, camera_checks=camera_checks,
                  camera_readbacks=cameras,
                  d3d12_sm6="PCD3D_SM6" in log and "LogD3D12RHI" in log,
                  commandline_offscreen="-RenderOffscreen" in log)
    save(out / "readback.json", result)
    if not result["ok"] or not result["d3d12_sm6"]:
        raise ValueError("runtime readback failed: " + str(out))


def run(args):
    frames = json.loads((ROOT / "inputs" / (args.track + ".json")).read_text())["frames"]
    if args.smoke:
        frames = frames[:9]
    out = ROOT / "runs" / args.label
    out.mkdir(parents=True, exist_ok=False)
    start = 96
    events = [(40, f"DumpCVars -csv={(out / 'cvars.csv').as_posix()}")]
    # Each repeat starts with a reset, so repeats exercise the same cold input sequence.
    # Installed UE's FCommandLine is 16 KiB. Two full-track A/B pairs fit;
    # four independently guarded invocations provide eight pairs without truncation.
    repeats = 1 if args.capture else 4
    for repeat in range(repeats):
        offset = start + repeat * (len(frames) + 16)
        if not args.capture:
            events.append((offset - 12, f"r.AntiAliasingMethod {0 if repeat % 2 == 0 else 4}"))
        events.append((offset - 1, "r.ResetViewState"))
        if args.capture:
            events.append((offset - 1, macro(out, "arm", ["r.DumpGPU.Root Tonemap*", f"r.DumpGPU.FrameCount {len(frames)}",
                          f"r.DumpGPU.Directory {(out / 'dump').as_posix()}", "DumpGPU"])))
        for i, p in enumerate(frames):
            lines = (["getall PlayerCameraManager CameraCachePrivate"] if args.capture and i > 0 else []) + pose_commands(p)
            if args.capture:
                # Ordinary Shot is the native main view. HighResShot can replace/reset temporal state.
                if i in {0, 1, 2, 4, 8, 16, 32, 48, 60, 64, 65, 66, 68, 72, 80, 88, 92, 95, 96, 108, 120, 128, 160}:
                    lines.append(f"Shot filename={(out / f'f{i:03d}.png').as_posix()}")
            events.append((offset + i, macro(out, f"r{repeat}-f{i:03d}", lines)))
        if args.capture:
            events.append((offset + len(frames), "getall PlayerCameraManager CameraCachePrivate"))
    count = start + repeats * (len(frames) + 16) + (96 if args.capture else 0)
    config = settings(args.quality, args.path)
    expected_fov = math.degrees(2 * math.atan(math.tan(frames[0]["fov_y_radians"] / 2) * 1920 / 1080))
    initial = copy.deepcopy(frames[0])
    if args.track == "q-sun-jump":
        lighting = json.loads((ROOT / "inputs/manifest.json").read_text())["inputs"]["lighting"]["contents"]
        light = -np.array(lighting["lights"][0]["travel"])
        initial["to_light"] = (light / np.linalg.norm(light)).tolist()
    config += pose_commands(initial) + [f"set CameraComponent FieldOfView {expected_fov:.9f}"]
    startup_macro = macro(out, "startup", config + [f"csvprofile frames={count}"])
    # Exec resolves beneath Engine/Binaries and rejects absolute paths. A tiny,
    # exclusively created scratch name also keeps the CSV timeline below 16 KiB.
    while True:
        short_dir = U0.PACKAGE.parent.parent / ("t" + uuid.uuid4().hex[:2])
        try:
            short_dir.mkdir()
            break
        except FileExistsError:
            continue
    replacements = {}
    for index, source in enumerate(sorted((out / "macros").glob("*.txt"))):
        destination = short_dir / f"{index:x}"
        shutil.copyfile(source, destination)
        replacements["exec " + Path(os.path.relpath(source, U0.PACKAGE.parent.parent)).as_posix()] = "exec " + short_dir.name + "/" + destination.name
    events = [(f, replacements.get(cmd, cmd)) for f, cmd in events]
    startup_macro = replacements[startup_macro]
    command = [U0.PACKAGE, "../../../M5Calibration/M5Calibration.uproject", *U0.UE_FLAGS,
               f"-abslog={out / 'ue.log'}", *U0.COMMON_INI, "-dx12", "-sm6", "-RenderOffscreen",
               "/Game/M6Quarry/View0", "-game", "-ResX=1920", "-ResY=1080", "-ForceRes", "-NoVSync",
               "-UseFixedTimeStep", "-FPS=60", "-csvGpuStats", "-csvCompression=0", "-ExitAfterCsvProfiling",
               "-csvExecCmds=" + ",".join(f"{f}:{c}" for f, c in sorted(events)),
               "-ExecCmds=" + startup_macro]
    # UE's own CmdLineFile expansion avoids Windows' 32K process-commandline limit.
    command_file = out / "commandline.txt"
    def ue_argument(value):
        value = str(value)
        if '"' in value:
            raise ValueError("nested commandline quotes belong in an exec macro")
        if " " in value:
            key, sep, tail = value.partition("=")
            return key + sep + '"' + tail + '"' if sep else '"' + value + '"'
        return value
    command_file.write_text(" ".join(ue_argument(c) for c in command[2:]), encoding="utf-8")
    if len(command_file.read_text()) > 16000:
        for path in short_dir.iterdir():
            path.unlink()
        short_dir.rmdir()
        raise ValueError("UE commandline exceeds its 16 KiB expansion limit")
    actual_command = [str(c) for c in command[:2]] + ["-CmdLineFile=" + command_file.as_posix()]
    save(out / "recipe.json", dict(track=args.track, path=args.path, quality=args.quality, capture=args.capture,
          smoke=args.smoke, start=start, frames=frames, repeats=repeats, gap=16, csv_frames=count,
          sequence_sha256=digest(ROOT / "inputs" / (args.track + ".json")), script_sha256=digest(__file__),
          package_exe_sha256=digest(U0.PACKAGE), shutter=0,
          note="Timing: 2 no-AA/TSR paired full-track replays in one process; four invocations give eight pairs. Even repeats no AA, odd repeats TSR. Cold reset per replay. Capture and timing are separate processes. No reference score implied."))
    csv_root = U0.PACKAGE_DIR / "Windows/M5Calibration/Saved/Profiling/CSV"
    before = set(csv_root.glob("*.csv"))
    try:
        with U0.lease(args.label, args.minutes):
            U0.guarded(actual_command, out, timeout=args.minutes * 60 - 20, cwd=U0.PACKAGE.parent)
    finally:
        # Only the exclusively-created directory and its own copied macros.
        if short_dir.parent.resolve() != U0.PACKAGE.parent.parent.resolve():
            raise ValueError("short macro cleanup outside package Binaries")
        for path in short_dir.iterdir():
            path.unlink()
        short_dir.rmdir()
    new = set(csv_root.glob("*.csv")) - before
    if len(new) != 1:
        raise ValueError("expected exactly one CSV: " + str(new))
    source, = new
    shutil.copyfile(source, out / "profile.csv")
    source.unlink()
    readback(out, args.quality, args.path)
    print("captured", out, flush=True)


def summarize(args):
    out = ROOT / "runs" / args.label
    recipe = json.loads((out / "recipe.json").read_text())
    rows = list(csv.reader((out / "profile.csv").open(encoding="utf-8-sig")))
    header = rows[0]
    data = rows[1:]
    n = len(recipe["frames"])
    indices = [recipe["start"] + r * (n + recipe["gap"]) + i
               for r in range(recipe["repeats"]) for i in range(8, n)]
    stats = {}
    for key in ("GPUTime", "GPU/TemporalSuperResolution", "FrameTime", "RenderThreadTime", "GameThreadTime"):
        if key not in header:
            continue
        values = [float(data[i][header.index(key)]) for i in indices if i < len(data) and data[i][header.index(key)]]
        if values:
            stats[key] = dict(n=len(values), p50=float(np.median(values)), p95=float(np.quantile(values, .95)),
                              minimum=min(values), maximum=max(values))
    save(out / "timing.json", dict(stats=stats, capture=recipe["capture"],
         valid_timing=not recipe["capture"] and not recipe["smoke"],
         scope="Native CSV GPUTime critical-path frame and TSR GPU stat; first 8 frames of every cold repeat excluded"))
    if not recipe["capture"]:
        by_method = {}
        for method, parity in (("no_aa", 0), ("tsr", 1)):
            chosen = [recipe["start"] + r * (n + recipe["gap"]) + i
                      for r in range(parity, recipe["repeats"], 2) for i in range(8, n)]
            by_method[method] = {}
            for key in ("GPUTime", "GPU/TemporalSuperResolution"):
                if key not in header:
                    continue
                values = [float(data[i][header.index(key)]) for i in chosen if data[i][header.index(key)]]
                by_method[method][key] = dict(n=len(values), p50=float(np.median(values)), p95=float(np.quantile(values, .95)))
        pairs = []
        frame_costs = []
        col = header.index("GPUTime")
        for r in range(0, recipe["repeats"], 2):
            for i in range(8, n):
                a = recipe["start"] + r * (n + recipe["gap"]) + i
                b = a + n + recipe["gap"]
                pairs.append(float(data[b][col]) - float(data[a][col]))
                frame_costs.append(dict(pair=r // 2, frame=i, phase=recipe["frames"][i].get("phase"),
                     no_aa_gpu_ms=float(data[a][col]), tsr_gpu_ms=float(data[b][col]),
                     tsr_pass_ms=float(data[b][header.index("GPU/TemporalSuperResolution")]) if "GPU/TemporalSuperResolution" in header else None))
        save(out / "timing.json", dict(by_method=by_method, paired_delta_p50_ms=float(np.median(pairs)),
             paired_delta_p95_ms=float(np.quantile(pairs, .95)), pairs=len(pairs), valid_timing=not recipe["smoke"],
             per_frame=frame_costs,
             scope="2 adjacent no-AA/TSR full-track pairs per process, same frame indices, first 8 frames excluded per track. Four processes give eight pairs. Native CSV GPUTime plus TSR stat. Desktop contention recorded in preflight."))
        print(json.dumps(by_method, indent=2))
        return
    dump = NATIVE.s1.native.Dump(out / "dump")
    grouped = {}
    for event in dump.passes:
        frame = next(int(m.group(1)) for scope in event["ParentEventScopes"] if (m := re.match(r"Frame (\d+)", scope)))
        grouped.setdefault(frame, []).append(event)
    if len(grouped) != n:
        raise ValueError(f"native frame coverage {len(grouped)} != {n}")
    identities = []
    previous_hold = None
    hold_steps = []
    hold_indices = [i for i, p in enumerate(recipe["frames"]) if p.get("phase") == "hold"]
    hold_curve = []
    target_codes = None
    def codes_for(value):
        linear = np.maximum(0, value[..., :3])
        linear /= 1 + linear
        return 255 * np.where(linear <= .0031308, linear * 12.92, 1.055 * linear ** (1 / 2.4) - .055)
    if hold_indices:
        target_events = sorted(grouped.items())[hold_indices[-1]][1]
        p, name = next((p, name) for p in target_events for name in p["InputResources"]
                       if dump.resources[name.split(".")[0]]["Name"] in ("TSR.Output", "TSR.History.Color"))
        target_codes = codes_for(NATIVE.resource(dump, name, p)[0])
    for i, (frame, events) in enumerate(sorted(grouped.items())):
        candidates = [(p, name) for p in events for name in p["InputResources"]
                      if dump.resources[name.split(".")[0]]["Name"] in ("TSR.Output", "TSR.History.Color")]
        if len(candidates) != 1:
            raise ValueError(f"expected one Tonemap TSR.Output input at frame {i}: {len(candidates)}")
        consumer, name = candidates[0]
        values, identity = NATIVE.resource(dump, name, consumer)
        if values.shape[:2] != (1080, 1920) or not np.isfinite(values).all():
            raise ValueError("invalid native post-TSR HDR")
        identities.append(dict(frame=i, engine_frame=frame, hdr=identity,
                               min=float(values.min()), max=float(values.max()), mean=float(values[..., :3].mean())))
        if recipe["frames"][i].get("phase") == "hold":
            codes = codes_for(values)
            if previous_hold is not None:
                hold_steps.append(dict(frame=i, rms=float(np.sqrt(np.mean((codes - previous_hold) ** 2)))))
            previous_hold = codes
            diff = np.abs(codes - target_codes)
            hold_curve.append(dict(frame=i, mean=float(diff.mean()), p99=float(np.quantile(diff, .99)),
                                   rms=float(np.sqrt(np.mean(diff ** 2)))))
        # Hashes prove capture coverage after raw retention cleanup; PNGs are inspection only.
        if i in (0, 8, 48, 64, 72, 95, 96, 120, 128, 160):
            display = NATIVE.s1.film.tone(values[..., :3])
            display.save(out / f'hdr-inspect-f{i:03d}.png')
    save(out / "capture-readback.json", dict(frames=n, identities=identities,
         scope="Native pre-tonemap post-TSR colour, still pre-exposed; not F0 radiance or film visibility",
         missing=["U0 quarry reference", "T1b ages and populations", "reference-noise subtraction",
                  "per-frame pre-exposure proof", "F1-F5 and reference-subtracted motion F6 scores"]))
    if hold_curve:
        first = next((row["frame"] - hold_indices[0] for k, row in enumerate(hold_curve)
                      if all(r["mean"] < .5 and r["p99"] < 4 for r in hold_curve[k:])), None)
        save(out / "hold-diagnostic.json", dict(steps=hold_steps, curve=hold_curve, settle_offset=first,
             hold_step_rms_p50=float(np.median([r["rms"] for r in hold_steps])) if hold_steps else None,
             scope="Full-image F0-transform self residual in pre-exposed HDR. Target is last hold, not an independent film reference. No T1b population masks. Diagnostic only; not F2/F5/F6 acceptance."))
    print("native HDR frames", n, flush=True)


def clean(args):
    root = (ROOT / "runs" / args.label).resolve()
    if ROOT.resolve() not in root.parents or not root.is_dir():
        raise ValueError("cleanup outside owned run")
    if not (root / "capture-readback.json").exists():
        raise ValueError("summarize before cleanup")
    deleted = []
    for path in root.rglob("*"):
        if path.is_file() and path.stat().st_size > 5_000_000 and path.suffix.lower() in (".bin", ".npz", ".npy", ".exr"):
            deleted.append(dict(path=str(path), bytes=path.stat().st_size, sha256=digest(path)))
            path.unlink()
    save(root / "deleted-raw.json", deleted)
    print("deleted", sum(r["bytes"] for r in deleted), "bytes")


def baseline(_):
    """Extract the immutable recorded T1b scores without recomputing or changing their bar."""
    source = T1 / "scores-walk/score.json"
    spread_path = T1 / "scores-t1k/score.json"
    data = json.loads(source.read_text())
    spread = json.loads(spread_path.read_text())["poses"]["0"]["bars"]
    rows = []
    for frame, result in data["frames"].items():
        if not result.get("converged"):
            continue
        for channel, populations in result["converged"]["t1"].items():
            for population, score in populations.items():
                exact = result["converged"]["exact"][channel][population]["rms"]
                bar = exact * (1 + spread.get(channel, {}).get(population, {}).get("spread_2sd", 0))
                rows.append(dict(frame=int(frame), phase=result["phase"], channel=channel, population=population,
                                 n=score["n"], t1b_rms=score["rms"], exact_rms=exact, bar=bar,
                                 t1b_ratio=score["rms"] / bar if bar else None,
                                 tsr_epic=None, tsr_high=None,
                                 tsr_status="pending matched reference, population and age-mask reconstruction"))
    out = ROOT / "t1b-recorded.json"
    save(out, dict(source=str(source), source_sha256=digest(source), spread_sha256=digest(spread_path),
                   commit="b8fa1f3", rows=rows, gates=data["gates"],
                   scope="Carried historical T1b scores; not rerendered, new references do not replace their bar."))
    print("recorded T1b population/channel/frame rows", len(rows))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("plan")
    commands.add_parser("baseline")
    p = commands.add_parser("run")
    p.add_argument("label")
    p.add_argument("--track", choices=("q-walk", "q-disocclusion", "q-zoom", "q-sun-slow", "q-sun-fast", "q-sun-jump"), default="q-walk")
    p.add_argument("--quality", type=int, choices=(2, 3), default=3)
    p.add_argument("--path", choices=("vsm", "hrt"), default="vsm")
    p.add_argument("--capture", action="store_true")
    p.add_argument("--smoke", action="store_true")
    p.add_argument("--minutes", type=int, default=6)
    for command in ("summarize", "clean"):
        commands.add_parser(command).add_argument("label")
    args = parser.parse_args()
    dict(plan=plan, run=run, summarize=summarize, clean=clean, baseline=baseline)[args.command](args)


if __name__ == "__main__":
    main()
