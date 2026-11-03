"""Generate a private UE calibration project. This program never launches Unreal."""

import argparse
import hashlib
import json
import math
from pathlib import Path
import shutil


DEFAULT_ROOT = Path("D:/Coding/renderlab-work/ue-m5")
DEFAULT_ENGINE = Path("C:/Program Files/Epic Games/UE_5.7")


def default_scene():
    # Bootstrap only: the lighting owner must supply the eventual shared fixture.
    return {
        "schema_version": 1,
        "fixture_id": "lambert-bootstrap-v1",
        "matched_renderlab_fixture": None,
        "coordinates": "RH_Xright_Yup_camera_minusZ_metres",
        "resolution": [640, 360],
        "samples": 1,
        "camera": {"eye_m": [0, 3, 4], "target_m": [0, 0, 0], "vertical_fov_deg": 50},
        "directional": {"travel": [0, -1, 0], "lux": 1000},
        "environment_rgb_cd_m2": [0, 0, 0],
        "diffuse_linear_rgb": [0.18, 0.18, 0.18],
        "specular": 0,
        "roughness": 1,
        "metallic": 0,
        "primitives": [
            {"name": "patch", "kind": "plane", "center_m": [0, 0, 0], "size_m": [4, 0, 4]},
            {"name": "metre_box", "kind": "box", "center_m": [-1.25, 0.5, 0], "size_m": [1, 1, 1]},
        ],
        "projection_probes_m": [[0, 0, 0], [1, 0, 0], [0, 0, 1], [-1.25, 1, 0]],
        "offline_display": {"exposure_stops": -6, "curve": "per_channel_Reinhard_then_sRGB"},
    }


def vector(value, name):
    if (not isinstance(value, list) or len(value) != 3
            or any(type(v) not in (int, float) or not math.isfinite(v) for v in value)):
        raise ValueError(f"{name} must contain three finite numbers")
    return value


def validate_scene(scene):
    if set(scene) != set(default_scene()):
        raise ValueError("scene keys must match schema 1 exactly; unsupported fixture inputs cannot be ignored")
    for name, keys in (("camera", {"eye_m", "target_m", "vertical_fov_deg"}),
                       ("directional", {"travel", "lux"}), ("offline_display", {"exposure_stops", "curve"})):
        if set(scene[name]) != keys:
            raise ValueError(f"unsupported {name} inputs")
    if scene["schema_version"] != 1 or scene["coordinates"] != "RH_Xright_Yup_camera_minusZ_metres":
        raise ValueError("only scene schema 1 and the declared RH metre convention are supported")
    if scene["samples"] != 1:
        raise ValueError("this bootstrap supports 1x only; forward-MSAA needs separate runtime validation")
    if (len(scene["resolution"]) != 2
            or any(type(v) is not int or not 0 < v <= 8192 for v in scene["resolution"])):
        raise ValueError("resolution must contain two integers in 1..8192")
    camera = scene["camera"]
    eye, target = vector(camera["eye_m"], "eye"), vector(camera["target_m"], "target")
    if eye[0] == target[0] and eye[2] == target[2]:
        raise ValueError("camera must have a nonzero horizontal direction (fixed +Y up, no roll)")
    if not 1 < camera["vertical_fov_deg"] < 179:
        raise ValueError("vertical FOV must be between 1 and 179 degrees")
    travel = vector(scene["directional"]["travel"], "travel")
    lux = scene["directional"]["lux"]
    if sum(v * v for v in travel) == 0 or not math.isfinite(lux) or lux <= 0:
        raise ValueError("directional light needs a nonzero travel and positive finite lux")
    if scene["environment_rgb_cd_m2"] != [0, 0, 0]:
        raise ValueError("bootstrap requires a black environment")
    if scene["specular"] != 0 or scene["metallic"] != 0 or scene["roughness"] != 1:
        raise ValueError("bootstrap requires specular=0, metallic=0, roughness=1 for Lambert calibration")
    if any(not 0 <= v <= 1 for v in vector(scene["diffuse_linear_rgb"], "diffuse")):
        raise ValueError("diffuse must be in [0,1]")
    if not scene["primitives"]:
        raise ValueError("at least one primitive is required")
    names = set()
    for primitive in scene["primitives"]:
        if set(primitive) != {"name", "kind", "center_m", "size_m"}:
            raise ValueError("unsupported primitive inputs")
        name = primitive["name"]
        if not name.isidentifier() or name in names:
            raise ValueError("primitive names must be unique identifiers")
        names.add(name)
        vector(primitive["center_m"], "center")
        size = vector(primitive["size_m"], "size")
        if primitive["kind"] not in ("plane", "box"):
            raise ValueError("only axis-aligned +Y planes and boxes are supported; no import")
        if size[0] <= 0 or size[2] <= 0 or (size[1] <= 0 if primitive["kind"] == "box" else size[1] != 0):
            raise ValueError("box dimensions must be positive; plane size is [positive,0,positive]")
    for probe in scene["projection_probes_m"]:
        vector(probe, "projection probe")
    display = scene["offline_display"]
    if display["curve"] != "per_channel_Reinhard_then_sRGB" or not math.isfinite(display["exposure_stops"]):
        raise ValueError("unsupported offline display transform")
    return scene


def to_ue_cm(position):
    x, y, z = position
    return [-100 * z, 100 * x, 100 * y]


def display_rgb(linear_rgb, exposure_stops):
    """Shared offline transform for either engine, after unexposed linear RGB decoding."""
    def channel(value):
        if not math.isfinite(value):
            raise ValueError("nonfinite radiance cannot enter the display comparison")
        exposed = max(0, value) * 2**exposure_stops
        mapped = exposed / (1 + exposed)
        return 12.92*mapped if mapped <= .0031308 else 1.055*mapped**(1/2.4) - .055
    return [channel(v) for v in vector(linear_rgb, "linear RGB")]


def projected_pixels(scene):
    """Independent pinhole expectations, top-left origin, continuous pixel coordinates."""
    eye, target = scene["camera"]["eye_m"], scene["camera"]["target_m"]
    def unit(v):
        length = math.sqrt(sum(x*x for x in v))
        return [x / length for x in v]
    def cross(a, b):
        return [a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0]]
    forward = unit([t-e for e, t in zip(eye, target)])
    right = unit(cross(forward, [0, 1, 0]))
    up = cross(right, forward)
    width, height = scene["resolution"]
    focal = height / (2 * math.tan(math.radians(scene["camera"]["vertical_fov_deg"]) / 2))
    probes = []
    for point in scene["projection_probes_m"]:
        delta = [p-e for p, e in zip(point, eye)]
        depth = sum(a*b for a, b in zip(delta, forward))
        if depth <= 0:
            raise ValueError("projection probes must be in front of the camera")
        pixel = [width/2 + focal*sum(a*b for a, b in zip(delta, right))/depth,
                 height/2 - focal*sum(a*b for a, b in zip(delta, up))/depth]
        probes.append({"source_m": point, "ue_cm": to_ue_cm(point), "expected_pixel_xy": pixel})
    return probes


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n", encoding="utf-8")


def ps_quote(value):
    return "'" + str(value).replace("'", "''") + "'"


def generate(root, engine, scene):
    validate_scene(scene)
    probes = projected_pixels(scene)
    root, engine = root.resolve(), engine.resolve()
    # This task owns only this private tree; never write generated content into a checkout.
    private_root = DEFAULT_ROOT.resolve()
    if root != private_root and private_root not in root.parents:
        raise ValueError(f"generated project must stay under {private_root}")
    build = json.loads((engine / "Engine/Build/Build.version").read_text(encoding="utf-8-sig"))
    if [build[k] for k in ("MajorVersion", "MinorVersion", "PatchVersion", "Changelist")] != [5, 7, 4, 51494982]:
        raise ValueError("expected verified UE 5.7.4 CL 51494982")
    existing_parent = root
    while not existing_parent.exists():
        existing_parent = existing_parent.parent
    free = shutil.disk_usage(existing_parent).free
    if free < 2 * 1024**3:
        raise ValueError("less than 2 GiB free: defer project/cache preparation")
    for directory in ("Config", "Content/Python", "Saved", "Artifacts", "DerivedDataCache"):
        (root / directory).mkdir(parents=True, exist_ok=True)
    write_json(root / "M5Calibration.uproject", {
        "FileVersion": 3, "EngineAssociation": "5.7", "Description": "Private M5 calibration bootstrap",
        "Plugins": [{"Name": name, "Enabled": True} for name in ("PythonScriptPlugin", "EditorScriptingUtilities")],
    })
    scene_bytes = json.dumps(scene, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()
    scene_hash = hashlib.sha256(scene_bytes).hexdigest()
    write_json(root / "scene.json", scene)
    write_json(root / "expected.json", {
        "schema_version": 1, "scene_sha256": scene_hash, "ue_build": build,
        "generator_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "editor_script_sha256": hashlib.sha256(Path(__file__).with_name("ue-m5-editor.py").read_bytes()).hexdigest(),
        "conversion": "UE_cm = 100 * (-source_z, source_x, source_y); determinant -1",
        "projection_probes": probes,
        "lambert_patch_cd_m2": [v * scene["directional"]["lux"] * max(0, -scene["directional"]["travel"][1])
                                / math.sqrt(sum(x*x for x in scene["directional"]["travel"])) / math.pi
                                for v in scene["diffuse_linear_rgb"]],
        "verification": "UNRUN; expected values only; matched Renderlab fixture must be supplied",
    })
    (root / "Config/DefaultEngine.ini").write_text(f"""[/Script/EngineSettings.GameMapsSettings]
EditorStartupMap=/Engine/Maps/Entry
GameDefaultMap=/Game/M5/Calibration

[/Script/Engine.RendererSettings]
r.ForwardShading=False
r.AllowStaticLighting=False
r.DynamicGlobalIlluminationMethod=0
r.ReflectionMethod=0
r.RayTracing=False
r.DefaultFeature.AutoExposure=False
r.DefaultFeature.Bloom=False
r.DefaultFeature.MotionBlur=False

[SystemSettings]
r.AntiAliasingMethod=0
r.MSAACount=1
r.ScreenPercentage=100
r.UsePreExposure=0
r.SSR.Quality=0
r.AmbientOcclusionLevels=0
r.DistanceFieldAO=0
r.ContactShadows=0
r.ShadowQuality=0
r.Shadow.Virtual.Enable=0
r.BloomQuality=0
r.EyeAdaptationQuality=0
r.MotionBlurQuality=0
r.VolumetricFog=0
r.SkyAtmosphere=0

; Private filesystem cache, no Zen/shared/cloud service. Reuse installed read-only shader data.
[RenderlabLocalDDC]
Root=(Type=Hierarchical, Inner=EnginePak, Inner=Local)
EnginePak=(Type=ReadPak, Filename=\"{engine.as_posix()}/Engine/DerivedDataCache/Compressed.ddp\", Compressed=true)
Local=(Type=FileSystem, UnusedFileAge=34, Path=\"{root.as_posix()}/DerivedDataCache\")
""", encoding="utf-8")
    (root / "Config/DefaultEditor.ini").write_text("""[/Script/PythonScriptPlugin.PythonScriptPluginSettings]
bDeveloperMode=True
bRemoteExecution=False
""", encoding="utf-8")
    helper = Path(__file__).with_name("ue-m5-editor.py")
    shutil.copyfile(helper, root / "Content/Python/ue_m5_editor.py")
    for mode in ("prepare-assets", "capture"):
        script = root / f"Content/Python/{mode}.py"
        script.write_text("import runpy\nfrom pathlib import Path\n"
                          f"runpy.run_path(str(Path(__file__).with_name('ue_m5_editor.py')), init_globals={{'MODE': {mode!r}}})\n",
                          encoding="utf-8")
        flags = ["-run=pythonscript", f"-script={script.as_posix()}", "-unattended", "-nop4", "-nosound",
                 "-ddc=RenderlabLocalDDC", f"-abslog={(root / 'Artifacts' / (mode + '.log')).as_posix()}"]
        flags += ["-NullRHI"] if mode == "prepare-assets" else ["-AllowCommandletRendering", "-vulkan"]
        arguments = ", ".join(ps_quote(v) for v in [root / "M5Calibration.uproject", *flags])
        launcher = f"""# UNRUN. Requires root's explicit GPU/heavy-build slot even for preparation.
$ErrorActionPreference = 'Stop'
$editor = {ps_quote(engine / 'Engine/Binaries/Win64/UnrealEditor-Cmd.exe')}
$arguments = @({arguments})
$watch = [Diagnostics.Stopwatch]::StartNew()
& $editor @arguments
$exitCode = $LASTEXITCODE
$watch.Stop()
@{{ mode = '{mode}'; elapsed_ms = $watch.Elapsed.TotalMilliseconds; exit_code = $exitCode;
   boundary = 'whole process: startup, loading, compilation, script, readback/export if capture, shutdown';
   scene_sha256 = '{scene_hash}' }} | ConvertTo-Json | Set-Content -Encoding UTF8 {ps_quote(root / 'Artifacts' / (mode + '-process.json'))}
if ($exitCode -ne 0) {{ throw "Unreal commandlet failed: $exitCode" }}
"""
        (root / f"{mode}.ps1").write_text(launcher, encoding="utf-8")
    return {"root": str(root), "scene_sha256": scene_hash, "free_gib": round(free / 1024**3, 2),
            "status": "generated only; no UE process launched"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=DEFAULT_ROOT)
    parser.add_argument("--engine", type=Path, default=DEFAULT_ENGINE)
    parser.add_argument("--scene", type=Path, help="schema-1 calibration JSON; see generated scene.json")
    args = parser.parse_args()
    scene = json.loads(args.scene.read_text(encoding="utf-8-sig")) if args.scene else default_scene()
    print(json.dumps(generate(args.root, args.engine, scene), indent=2))


if __name__ == "__main__":
    main()
