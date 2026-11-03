"""Prepare the retained M5-A fixture for UE 5.7.4. Files/CPU only; never launches UE."""
import argparse
import copy
import hashlib
import json
import math
from pathlib import Path
import shutil
import struct

REPO = Path(__file__).resolve().parents[1]
DEFAULT_ROOT = Path("D:/Coding/renderlab-work/ue-m5")
DEFAULT_ENGINE = Path("C:/Program Files/Epic Games/UE_5.7")
FIXTURE = REPO / "measurements/m5-a/fixture-retained-1x/scene.json"
FIXTURE_SHA = "db9020f23a880791501daf5850d09381706c33dd1a73a3bfaa83be7245c3ee8c"
SOURCE = REPO / "support/lighting_fixture.rs"
SOURCE_SHA = "8865ad1c2fdeea8885809b4467196904de2baaded6030200bf1ec1548ff0384d"
VARIANTS = ("directional-black", "full")


def sha(data):
    return hashlib.sha256(data).hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n", encoding="utf-8")


def load_fixture(path=FIXTURE):
    data = path.read_bytes()
    if sha(data) != FIXTURE_SHA:
        raise ValueError("fixture bytes differ from retained actual scene SHA256; no implicit replacement")
    if sha(SOURCE.read_bytes()) != SOURCE_SHA:
        raise ValueError("read-only fixture recipe source changed; re-audit conversion")
    return json.loads(data)


def f32(value):
    return struct.unpack("<f", struct.pack("<f", value))[0]


def cross(a, b):
    return [a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0]]


def dot(a, b):
    return sum(x*y for x, y in zip(a, b))


def unit(value):
    length = math.sqrt(dot(value, value))
    if length == 0:
        raise ValueError("zero normal/direction")
    return [v/length for v in value]


def to_ue(value, scale=100):
    x, y, z = value
    return [-scale*z, scale*x, scale*y]


def matrix_parts(columns):
    if len(columns) != 16 or columns[3:12:4] != [0, 0, 0] or columns[15] != 1:
        raise ValueError("expected recorded affine column-major matrix")
    a, b, c = [columns[i:i+3] for i in (0, 4, 8)]
    determinant = dot(a, cross(b, c))
    if determinant == 0:
        raise ValueError("singular normal transform")
    normal_columns = [[v/determinant for v in col] for col in (cross(b, c), cross(c, a), cross(a, b))]
    return [a, b, c], columns[12:15], normal_columns, determinant


def apply_columns(columns, value):
    return [sum(columns[j][i]*value[j] for j in range(3)) for i in range(3)]


def geometry(recipe):
    """Retain seam/poles, normals and topology from lighting_fixture.rs.
    Arithmetic rounds to f32; Python libm trig then rounds to f32. The export
    lacks raw Rust buffers, so platform sinf byte equality remains UNVERIFIED.
    """
    positions, normals, indices = [], [], []
    if recipe["shape"] == "uv_sphere":
        latitude, longitude = recipe["latitude_segments"], recipe["longitude_segments"]
        for y in range(latitude+1):
            theta = f32(f32(f32(math.pi)*f32(y))/f32(latitude))
            for x in range(longitude+1):
                phi = f32(f32(f32(math.tau)*f32(x))/f32(longitude))
                n = [f32(f32(math.sin(theta))*f32(math.cos(phi))), f32(math.cos(theta)),
                     f32(f32(math.sin(theta))*f32(math.sin(phi)))]
                normals.append(n)
                positions.append([f32(v*recipe["radius"]) for v in n])
        for y in range(latitude):
            for x in range(longitude):
                a = y*(longitude+1)+x
                b = a+longitude+1
                indices.extend([a, a+1, b, a+1, b+1, b])
    elif recipe["shape"] == "xy_quad":
        positions = [[x*recipe["width"], y*recipe["height"], 0]
                     for x, y in ((-.5, -.5), (.5, -.5), (.5, .5), (-.5, .5))]
        normals = [recipe["authored_normal"][:] for _ in positions]
        indices = [0, 2, 1, 0, 3, 2] if recipe["reverse_indices"] else [0, 1, 2, 0, 2, 3]
    else:
        raise ValueError("unsupported fixture geometry")
    return {"positions": positions, "normals": normals, "indices": indices}


def mesh_bytes(mesh):
    """Identity: interleaved LE f32 position/normal, followed by LE u32 indices."""
    return (b"".join(struct.pack("<6f", *p, *n) for p, n in zip(mesh["positions"], mesh["normals"]))
            + struct.pack(f"<{len(mesh['indices'])}I", *mesh["indices"]))


def convert_object(obj):
    local = geometry(obj["geometry"])
    if len(local["positions"]) != obj["vertices"] or len(local["indices"]) != 3*obj["triangles"]:
        raise ValueError("recipe counts disagree with actual export")
    columns, translation, normal_columns, determinant = matrix_parts(obj["transform_column_major"])
    world = [[v+t for v, t in zip(apply_columns(columns, p), translation)] for p in local["positions"]]
    converted = {
        "positions": [[f32(v) for v in to_ue(p)] for p in world],
        "normals": [[f32(v) for v in to_ue(unit(apply_columns(normal_columns, n)), 1)] for n in local["normals"]],
        "indices": local["indices"][:],
    }
    # Axis map: RH CCW -> UE LH clockwise; no extra index swap for axis reflection.
    # Bake the renderer's source-front rule by reversing indices for mirrored matrices.
    if determinant < 0:
        for i in range(0, len(converted["indices"]), 3):
            converted["indices"][i+1], converted["indices"][i+2] = converted["indices"][i+2], converted["indices"][i+1]
    bounds = {key: [op(p[i] for p in converted["positions"]) for i in range(3)]
              for key, op in (("min_cm", min), ("max_cm", max))}
    return converted, {"source_local_buffer_sha256": sha(mesh_bytes(local)),
                       "ue_world_buffer_sha256": sha(mesh_bytes(converted)),
                       "source_determinant": determinant, "mirrored_index_swap": determinant < 0, "bounds": bounds}


def light_record(light):
    rgb = light["lux"] if light["type"] == "directional" else light["candela"]
    peak = max(rgb)
    record = {"source": copy.deepcopy(light), "intensity": peak,
              "intensity_units": "lux" if light["type"] == "directional" else "candela",
              "linear_color": [v/peak for v in rgb],
              "color_storage": "UE 8-bit LightColor; measure actual linear color readback/quantization"}
    if "travel" in light:
        record["travel_ue"] = to_ue(unit(light["travel"]), 1)
    if "position" in light:
        record.update(position_cm=to_ue(light["position"]), attenuation_radius_cm=100*light["range"],
                      source_radius_cm=0, source_length_cm=0, inverse_squared_falloff=True)
    if light["type"] == "spot":
        record["inner_cone_half_angle_degrees"] = math.degrees(light["inner"])
        record["outer_cone_half_angle_degrees"] = math.degrees(light["outer"])
    return record


def projected_pixels(scene):
    camera = scene["camera"]
    eye, target = camera["eye"], camera["target"]
    forward = unit([t-e for t, e in zip(target, eye)])
    right = unit(cross(forward, camera["up"]))
    up = cross(right, forward)
    focal = scene["height"]/(2*math.tan(camera["fov_y_radians"]/2))
    probes = []
    for obj in scene["objects"]:
        p = obj["transform_column_major"][12:15]
        delta = [v-e for v, e in zip(p, eye)]
        depth = dot(delta, forward)
        probes.append({"name": obj["name"], "source_m": p, "ue_cm": to_ue(p),
                       "pixel_xy": [scene["width"]/2+focal*dot(delta, right)/depth,
                                    scene["height"]/2-focal*dot(delta, up)/depth]})
    return probes


def display_rgb(rgb, exposure_stops):
    result = []
    for v in rgb:
        if not math.isfinite(v):
            raise ValueError("nonfinite radiance")
        x = max(0, v)*2**exposure_stops
        y = x/(1+x)
        result.append(12.92*y if y <= .0031308 else 1.055*y**(1/2.4)-.055)
    return result


def ps_quote(value):
    return "'" + str(value).replace("'", "''") + "'"


def generate(root, engine, fixture_path=FIXTURE, variant="directional-black"):
    if variant not in VARIANTS:
        raise ValueError("unknown explicit fixture variant")
    scene = load_fixture(fixture_path)
    root, engine = root.resolve(), engine.resolve()
    if not root.is_relative_to(DEFAULT_ROOT.resolve()):
        raise ValueError(f"generated files must stay under {DEFAULT_ROOT}")
    build = json.loads((engine / "Engine/Build/Build.version").read_text(encoding="utf-8-sig"))
    if [build[k] for k in ("MajorVersion", "MinorVersion", "PatchVersion", "Changelist")] != [5, 7, 4, 51494982]:
        raise ValueError("expected verified UE 5.7.4 CL 51494982")
    parent = root
    while not parent.exists():
        parent = parent.parent
    free = shutil.disk_usage(parent).free
    if free < 2*1024**3:
        raise ValueError("less than 2 GiB free; defer preparation")
    for directory in ("Config", "Content/Python", "Meshes", "Artifacts", "DerivedDataCache"):
        (root / directory).mkdir(parents=True, exist_ok=True)
    (root / "scene.json").write_bytes(fixture_path.read_bytes())
    active = copy.deepcopy(scene["lighting"])
    if variant == "directional-black":
        active["lights"] = [l for l in active["lights"] if l["type"] == "directional"]
        active["environment"] = [0, 0, 0]
    write_json(root / "renderlab-lighting.json", active)
    objects = []
    for i, obj in enumerate(scene["objects"]):
        mesh, evidence = convert_object(obj)
        relative = f"Meshes/{i:02d}.json"
        write_json(root / relative, mesh)
        objects.append({"source": copy.deepcopy(obj), "mesh": relative,
                        "mesh_file_sha256": sha((root / relative).read_bytes()), **evidence,
                        "material": {"base_color_linear": obj["base_color_linear"],
                                     "metallic": obj["metallic"], "roughness": obj["perceptual_roughness"],
                                     "specular": .5, "dielectric_F0": .04, "two_sided": obj["double_sided"]}})
    manifest = {
        "schema_version": 2, "fixture_mode": "retained-m5-a", "variant": variant,
        "source_scene_sha256": FIXTURE_SHA, "recipe_source_sha256": SOURCE_SHA,
        "source_origin": str(fixture_path.resolve()), "ue_build": build,
        "resolution": [scene["width"], scene["height"]], "samples": 1,
        "camera": copy.deepcopy(scene["camera"]), "objects": objects,
        "source_lighting": scene["lighting"], "active_lighting": active,
        "lights": [light_record(l) for l in active["lights"]], "shading_source": scene["shading"],
        "conversion": "UE_cm=100*(-Z,X,Y); bake column-major affine/inverse-transpose normals; swap mirrored indices only; UE LH clockwise",
        "geometry_precision": "same topology/recipes and f32 arithmetic; Python libm trig rounded to f32; raw Rust buffer byte equality UNVERIFIED (not exported)",
        "runtime_geometry_check": "imported DynamicMesh full indices/positions/normals, static source oriented triangle multiset; built render buffers still require validation",
        "known_mismatches": [
            "Default-lit diffuse/Smith/roughness floor and energy settings require actual queries/calibration; F0=.04 alone is insufficient",
            "UE 8-bit light color quantization needs actual readback; exact source RGB values retained",
            "Source range=saturate(1-(d/r)^4)^2, inverse-square regularization=1cm; UE falloff not certified equal",
            "Source spot=squared inner/outer cosine interpolation; UE formula unverified; half-angle conversion only",
            "Raw Rust vertex dump absent: trig and CPU-baked/GPU-transformed rounding not byte-certified",
            "UE infinite far differs from source far=50m; fixture fully within range; near/projection require image verification",
        ],
        "environment_status": "black explicitly on BOTH engines via emitted preset" if variant == "directional-black" else "PENDING constant [4,5,8] cd/m2 cubemap/DFG; capture blocked, no fake fill",
        "runtime_status": "UNRUN", "quality_parity": "UNPROVEN", "performance_parity": "UNMEASURED",
    }
    write_json(root / "manifest.json", manifest)
    manifest_hash = sha((root / "manifest.json").read_bytes())
    helper = Path(__file__).with_name("ue-m5-editor.py")
    write_json(root / "expected.json", {
        "source_scene_sha256": FIXTURE_SHA, "manifest_sha256": manifest_hash,
        "generator_sha256": sha(Path(__file__).read_bytes()), "editor_script_sha256": sha(helper.read_bytes()),
        "projection_probes": projected_pixels(scene), "runtime_status": "UNRUN",
        "renderlab_lighting_sha256": sha((root / "renderlab-lighting.json").read_bytes()),
        "total_vertices": sum(o["vertices"] for o in scene["objects"]),
        "total_triangles": sum(o["triangles"] for o in scene["objects"]),
    })
    write_json(root / "M5Calibration.uproject", {
        "FileVersion": 3, "EngineAssociation": "5.7", "Description": "Private retained M5-A fixture",
        "Plugins": [{"Name": n, "Enabled": True} for n in
                    ("PythonScriptPlugin", "EditorScriptingUtilities", "GeometryScripting")],
    })
    (root / "Config/DefaultEngine.ini").write_text(f"""[/Script/EngineSettings.GameMapsSettings]
EditorStartupMap=/Engine/Maps/Entry
GameDefaultMap=/Game/M5/Fixture

[/Script/Engine.RendererSettings]
r.ForwardShading=False
r.AllowStaticLighting=False
r.DynamicGlobalIlluminationMethod=0
r.ReflectionMethod=0
r.RayTracing=False
r.Substrate=False
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

; Private filesystem DDC, installed read-only cache, no shared/Zen/cloud service.
[RenderlabLocalDDC]
Root=(Type=Hierarchical, Inner=EnginePak, Inner=Local)
EnginePak=(Type=ReadPak, Filename="{engine.as_posix()}/Engine/DerivedDataCache/Compressed.ddp", Compressed=true)
Local=(Type=FileSystem, UnusedFileAge=34, Path="{root.as_posix()}/DerivedDataCache")
""", encoding="utf-8")
    (root / "Config/DefaultEditor.ini").write_text("""[/Script/PythonScriptPlugin.PythonScriptPluginSettings]
bDeveloperMode=True
bRemoteExecution=False
""", encoding="utf-8")
    shutil.copyfile(helper, root / "Content/Python/ue_m5_editor.py")
    for mode in ("prepare-assets", "capture"):
        script = root / f"Content/Python/{mode}.py"
        script.write_text("import runpy\nfrom pathlib import Path\n"
                          f"runpy.run_path(str(Path(__file__).with_name('ue_m5_editor.py')), init_globals={{'MODE': {mode!r}}})\n", encoding="utf-8")
        flags = ["-run=pythonscript", f"-script={script.as_posix()}", "-unattended", "-nop4", "-nosound",
                 "-ddc=RenderlabLocalDDC", f"-abslog={(root / 'Artifacts' / (mode+'.log')).as_posix()}"]
        flags += ["-NullRHI"] if mode == "prepare-assets" else ["-AllowCommandletRendering", "-vulkan"]
        arguments = ", ".join(ps_quote(v) for v in [root / "M5Calibration.uproject", *flags])
        launcher = f"""# UNRUN. Execute only in root's assigned GPU/heavy-build slot.
$ErrorActionPreference = 'Stop'
$editor = {ps_quote(engine / 'Engine/Binaries/Win64/UnrealEditor-Cmd.exe')}
$arguments = @({arguments})
$watch = [Diagnostics.Stopwatch]::StartNew()
& $editor @arguments
$exitCode = $LASTEXITCODE
$watch.Stop()
@{{ mode = '{mode}'; variant = '{variant}'; elapsed_ms = $watch.Elapsed.TotalMilliseconds; exit_code = $exitCode;
   boundary = 'whole process: startup, loading, compilation, script, readback/export if capture, shutdown';
   manifest_sha256 = '{manifest_hash}' }} | ConvertTo-Json | Set-Content -Encoding UTF8 {ps_quote(root / 'Artifacts' / (mode+'-process.json'))}
if ($exitCode -ne 0) {{ throw "Unreal commandlet failed: $exitCode" }}
"""
        (root / f"{mode}.ps1").write_text(launcher, encoding="utf-8")
    return {"root": str(root), "variant": variant, "source_scene_sha256": FIXTURE_SHA,
            "manifest_sha256": manifest_hash, "free_gib": round(free/1024**3, 2), "runtime_status": "UNRUN"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=DEFAULT_ROOT)
    parser.add_argument("--engine", type=Path, default=DEFAULT_ENGINE)
    parser.add_argument("--fixture", type=Path, default=FIXTURE, help="actual retained export; SHA256 pinned")
    parser.add_argument("--variant", choices=VARIANTS, default="directional-black")
    args = parser.parse_args()
    print(json.dumps(generate(args.root, args.engine, args.fixture, args.variant), indent=2))


if __name__ == "__main__":
    main()
