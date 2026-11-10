"""P-YARD2 Unreal runs over the shared manifest, through U0/U0b's packaged UE 5.7.4 drivers.

Run only inside this unit's n8-run lease (host.py lease); the U0 driver's own lease is
replaced by a no-op because n8-run already holds the shared GPU slot.

    capture SCENE CONFIGS...   native Tonemap-input HDR of every frame from ue_dump_first
    timing SCENE CONFIGS...    the same frames without dumps; CSV GPUTime per frame
    decode SCENE               CPU only, after the lease: HDR dumps to npz (raw dumps deleted)

Settings are U0's (scripts/m7-u0-ue.py BASE_COMMANDS: D3D12 SM6, -RenderOffscreen, manual
-14 EV with pre-exposure override 0, VSM SMRT 8x8 or HRT, TSR Epic 100 % / 200 % history),
each proven by DumpCVars readback. Low and Medium set their GI/reflection groups with
Lumen disallowed by the installed BaseScalability.ini ("Lumen not active").
"""
import argparse
import contextlib
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
WORK = Path("D:/Coding/renderlab-work/p-yard2/ue")
U0 = Path("E:/renderlab-work/m7-u0/ue-package")
U0B = Path("E:/renderlab-work/m7-u0b/ue-package")
# Unreal's exec takes paths relative to the package, so the tiny macro files must share
# its drive; they are removed at the unit's end.
MACROS = Path("E:/renderlab-work/m7-u0b/m/py2")
MANIFEST = HERE / "manifest.json"
CONFIGS = ["ue-hw-e", "ue-sw-e", "ue-low", "ue-med", "ue-sw-h", "ue-hw-h", "ue-sw-c", "ue-hw-c"]


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), REPO / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def driver(capture):
    b = load("m7-u0b")
    u = b.ue(allow_game=False)
    u.OUT = WORK

    @contextlib.contextmanager
    def held(label, minutes):  # n8-run owns the slot and its guard
        yield dict(utc=u.utc().isoformat(), expected_minutes=minutes, label=label)
    u.lease = held
    if capture:  # U0b's capture-only synchronisation; never used for timing
        u.UE_FLAGS += ["-UseFixedTimeStep", "-FPS=60"]
    # Low/Medium: the preset's GI/reflection groups, Lumen disallowed by them. The matched
    # project's SystemSettings pin r.DistanceFieldAO 0 and r.SSR.Quality 0; restore each
    # group's own values at console priority so the rows are the stock presets.
    for name, level, extra in (("ue-low", 0, ["r.DistanceFieldAO 0", "r.ReflectionMethod 0"]),
                               ("ue-med", 1, ["r.DistanceFieldAO 1", "r.SSR.Quality 2", "r.ReflectionMethod 2"])):
        c = u.spec()
        c["commands"] = [f"sg.GlobalIlluminationQuality {level}", f"sg.ReflectionQuality {level}"] + \
            [x for x in c["commands"] if not x.startswith("r.ReflectionMethod")] + extra
        c["required"] = [x for x in c["required"] if x[0] != "r.ReflectionMethod"] + [
            ("sg.GlobalIlluminationQuality", str(level)), ("sg.ReflectionQuality", str(level)),
            ("r.Lumen.DiffuseIndirect.Allow", "0"), ("r.Lumen.Reflections.Allow", "0"),
            *[(x.split()[0], x.split()[1]) for x in extra]]
        u.CONFIGS[name] = c
    for c in u.CONFIGS.values():
        if capture:
            c["commands"] += ["r.OneFrameThreadLag 0"]
            c["required"] += [("r.OneFrameThreadLag", "0")]
    return b, u


def events(u, b, scene, name):
    """Per-frame camera/mover macros wherever the frame differs from the level's start."""
    macros = MACROS / name
    macros.mkdir(parents=True, exist_ok=True)
    frames = scene["frames"]
    out = []
    for i, f in enumerate(frames):
        lines = []
        if scene["kind"] == "quarry":
            if i >= 160:
                lines += ["getall PlayerCameraManager CameraCachePrivate"] + b.pose_commands(f)
        else:
            # The pose is applied twice before any capture and read back once (frame 20).
            if i in (0, 8):
                lines += b.pose_commands(f)
            if i == 20:
                lines.append("getall PlayerCameraManager CameraCachePrivate")
            for mover, offset in f.get("movers", {}).items():
                if i and offset == frames[i - 1]["movers"][mover]:
                    continue
                actor = MOVER_ACTORS[(name, mover)]
                x, y, z = offset
                lines.append(f'ke {actor} K2_SetActorLocation "(X={-100 * z:.6f},Y={100 * x:.6f},Z={100 * y:.6f})" false () false')
        if f.get("cut"):
            lines.append("r.ResetViewState")
        if lines:
            out.append((i, u.macro(macros, f"{i}", lines)))
    if scene["kind"] == "quarry":
        out.append((len(frames), "getall PlayerCameraManager CameraCachePrivate"))
    return out


# U0b's import inventories (E:/renderlab-work/m7-u0b/import/S4.json, S5.json); both movable.
MOVER_ACTORS = {("s4-door", "door"): "StaticMeshActor_4", ("s5-cube", "cube"): "StaticMeshActor_0"}


def run(args, capture):
    manifest = json.loads(MANIFEST.read_text())
    scene = manifest["scenes"][args.scene]
    b, u = driver(capture)
    bench = scene["kind"] == "bench"
    u.PACKAGE_DIR = U0B if bench else U0
    u.PACKAGE = u.PACKAGE_DIR / "Windows/Engine/Binaries/Win64/UnrealGame.exe"
    evs = events(u, b, scene, args.scene)
    n = len(scene["frames"])
    for config in args.configs:
        out = WORK / args.scene / ("capture" if capture else f"timing-{args.visit}") / config
        if (out / "process.json").exists() and json.loads((out / "process.json").read_text())["exit_code"] == 0:
            continue
        out.mkdir(parents=True, exist_ok=True)
        extra = []
        e = list(evs)
        if bench:
            source = json.loads(Path(scene["bench"]).read_text())
            angle = math.degrees(2 * source["sun"]["angular_radius"])
            # U0b: restore the bench sun disk after the HRT preset sets the quarry's.
            extra += [f"set DirectionalLightComponent LightSourceAngle {angle:.9f}",
                      "r.Shadow.CacheWPOPrimitives 1", "r.Shadow.CacheWPOPrimitives 0", *u.READBACK]
        if capture:
            first = scene["ue_dump_first"]
            # Unreal dumps one contiguous block: every frame from the quarry's still capture,
            # or the bench's capture range.
            count = sum(f["capture"] for f in scene["frames"]) if bench else n - first
            extra += ["r.DumpGPU.Upload 0", "r.DumpGPU.Explore 0", "r.DumpGPU.Stream 0", "r.DumpGPU.Texture 2",
                      "r.DumpGPU.Buffer 2", "r.DumpGPU.PassParameters 1", f"r.DumpGPU.FrameCount {count}",
                      "r.DumpGPU.Root Tonemap*", f"r.DumpGPU.Directory {(out / 'hdr').as_posix()}"]
            e.append((first - 1, "DumpGPU"))
        e.sort(key=lambda x: x[0])
        started = time.monotonic()
        u.launch_ue(out, config, "static", extra=extra, events=e, frames=n + 8, level=scene["ue_level"])
        ok = u.readback(out, config, u.scalability_expectations())
        if not bench:
            log = (out / "ue.log").read_text(encoding="utf-8-sig", errors="replace")
            # Readbacks at frames 160..n report the camera each frame 159..n-1 rendered.
            poses = [dict(f, fov_y_radians=f["fov_y"]) for f in scene["frames"][159:]]
            u.save(out / "camera-readback.json", dict(frames=b.check_camera(log, poses), all_ok=True))
        u.save(out / "recipe.json", dict(scene=args.scene, config=config, capture=capture, frames=n,
                                         manifest_sha256=u.sha(MANIFEST), package_executable_sha256=u.sha(u.PACKAGE),
                                         fixed_timestep=capture, seconds=time.monotonic() - started))
        print(json.dumps(dict(scene=args.scene, config=config, capture=capture, readback_ok=ok,
                              seconds=round(time.monotonic() - started, 1))), flush=True)
        if not ok:
            raise RuntimeError("readback failed " + str(out))


def decode(args):
    root = WORK / args.scene / "capture"
    stops = int(json.loads(MANIFEST.read_text())["scenes"][args.scene]["exposure_stops"])
    for out in sorted(root.iterdir()):
        if (out / "native.json").exists() or not (out / "hdr").exists():
            continue
        with (out / "decode.log").open("w") as log:
            code = subprocess.run(["py", "-3", "-B", str(REPO / "scripts/m7-u0b-score.py"), "decode", str(out / "hdr"),
                                   str(out / "native.npz"), "--stops", str(stops)], cwd=REPO, stdout=log, stderr=log,
                                  creationflags=subprocess.CREATE_NO_WINDOW).returncode
        if code and args.scene != "quarry":
            bench_native(out)
        elif code:
            raise RuntimeError("decode failed " + str(out))
        print("decoded", out, flush=True)


def bench_native(out):
    """Bench dumps skip the frame whose HighResShot (CSV 72) takes the Tonemap, so U0b's
    contiguous check fails. Each archive is still exposure-proven by the decoder; index it by
    engine frame instead. The UE log numbers frames CSV + 1 here (the door's CSV-33 move logs
    at [34]); the camera/mover macros prove it per run."""
    log = (out / "ue.log").read_text(encoding="utf-8-sig", errors="replace")
    import re
    calls = re.findall(r"\]\[\s*(\d+)\]Called 'K2_SetActorLocation \"\(X=([-0-9.]+),Y=([-0-9.]+),Z=([-0-9.]+)\)\" false \(\) false' on instance", log)
    first_move = next(int(f) for f, *xyz in calls if any(abs(float(v)) > 0 for v in xyz))
    offset = first_move - 33  # both tracks first move at CSV frame 33
    frames = []
    for path in sorted(out.glob("native-*.npz"), key=lambda p: int(p.stem.split("-")[1])):
        engine = int(path.stem.split("-")[1])
        frames.append(dict(index=engine - offset, engine_frame=engine, archive=str(path),
                           archive_sha256=hashlib.sha256(path.read_bytes()).hexdigest()))
    (out / "native.json").write_text(json.dumps(dict(frames=frames, engine_minus_csv=offset,
        note="indexed by engine frame; the HighResShot frame is absent"), indent=1))
    removed = {}
    for path in (out / "hdr").rglob("*.bin"):
        if path.stat().st_size > 5 * 1024 ** 2:
            removed[path.name] = path.stat().st_size
            path.unlink()
    (out / "native.deleted.json").write_text(json.dumps(removed))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    s = p.add_subparsers(dest="mode", required=True)
    for mode in ("capture", "timing"):
        q = s.add_parser(mode)
        q.add_argument("scene")
        q.add_argument("configs", nargs="*", default=CONFIGS)
        q.add_argument("--visit", type=int, default=0)
    s.add_parser("decode").add_argument("scene")
    a = p.parse_args()
    if a.mode == "decode":
        decode(a)
    else:
        run(a, a.mode == "capture")


if __name__ == "__main__":
    main()
