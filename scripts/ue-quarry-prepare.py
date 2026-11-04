"""Stage the registered 4K quarry for the existing private UE project; CPU/files only.

No Unreal import or launch takes place here. Geometry comes from the measured RLA,
not the older 2K GLB. Importer/render-data verification remains a separate step.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct

import numpy as np
from PIL import Image

PRIVATE = Path("D:/Coding/renderlab-work/ue-m5")
ASSET = Path("D:/Coding/projects/desktop/renderlab/measurements/quarry-quality/detail/full.rla")
ASSET_SHA = "1ce53c381b467345addc1dce5a25fad45435bd90cb12ad2a0d14bb7a593257f3"
CAMERAS = Path("D:/Coding/renderlab-work/m6-hardware-msaa/quarry-cameras.json")
CAMERAS_SHA = "8a9c055339a625121360dbe93b512fcbb75afd583594a403ffe2d7d2c3606dd1"
LIGHTING = Path("D:/Coding/projects/desktop/renderlab/.refs/m5-m6-combined/quarry-hdr-lighting.json")
LIGHTING_SHA = "064d75ab6d0aa626aa498de33867a01ecefa37b36efe1383d23a3d5d2cf2ef25"
ENGINE = Path("C:/Program Files/Epic Games/UE_5.7")


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n", encoding="utf-8")


def read_layout(path):
    """Read schema-1 boundaries without loading the 1.55 GB asset into memory."""
    total = path.stat().st_size
    with path.open("rb") as stream:
        def take(size):
            data = stream.read(size)
            if len(data) != size:
                raise ValueError("truncated RLA")
            return data

        def words(count=1):
            return struct.unpack("<" + "I" * count, take(4 * count))

        def span(size):
            start = stream.tell()
            if start + size > total:
                raise ValueError("RLA span outside file")
            stream.seek(size, 1)
            return start

        if take(8) != b"RLA\0\1\0\0\0":
            raise ValueError("expected schema-1 RLA; normal-map schemas need an explicit adapter")
        nm, np_, nmat, nt = words(4)
        meshes = []
        for _ in range(nm):
            primitives = []
            for _ in range(words()[0]):
                material, nv, ni = words(3)
                if material >= nmat or nv == 0 or ni == 0 or ni % 3:
                    raise ValueError("invalid primitive")
                primitives.append(dict(material=material, vertices=nv, indices=ni,
                                       vertex_offset=span(nv * 48), index_offset=span(ni * 4)))
            meshes.append(primitives)
        placements = []
        for _ in range(np_):
            mesh = words()[0]
            matrix = list(struct.unpack("<16f", take(64)))
            if mesh >= nm or not np.isfinite(matrix).all() or [matrix[i] for i in (3, 7, 11, 15)] != [0, 0, 0, 1]:
                raise ValueError("invalid placement")
            placements.append(dict(mesh=mesh, transform_column_major=matrix))
        materials = []
        for _ in range(nmat):
            factors = list(struct.unpack("<6f", take(24)))
            sided = words()[0]
            refs = []
            for _ in range(2):
                present = words()[0]
                if present not in (0, 1):
                    raise ValueError("invalid texture reference")
                refs.append(list(words(6)) if present else None)
            if sided not in (0, 1) or refs[1] is not None or not np.isfinite(factors).all():
                raise ValueError("expected registered opaque base-color-only materials")
            materials.append(dict(factors=factors, double_sided=bool(sided), references=refs))
        textures = []
        for _ in range(nt):
            role, levels = words(2)
            if role != 0 or not 1 <= levels <= 32:
                raise ValueError("expected base-color texture")
            mips = []
            for _ in range(levels):
                width, height, size = words(3)
                if not width or not height or size != width * height * 4:
                    raise ValueError("invalid RGBA8 mip")
                if mips and [width, height] != [max(1, mips[-1][k] // 2) for k in ("width", "height")]:
                    raise ValueError("invalid mip chain")
                mips.append(dict(width=width, height=height, offset=span(size), size=size))
            if (width, height) != (1, 1):
                raise ValueError("incomplete mip chain")
            textures.append(dict(role=role, mips=mips))
        if stream.tell() != total:
            raise ValueError("trailing RLA bytes")
    return dict(meshes=meshes, placements=placements, materials=materials, textures=textures)


def bake(vertices, indices, column_major):
    """Bake the source affine, then rotate glTF so UE imports (-Z,X,Y).

    Installed UE 5.7 glTF conversion maps (x,y,z) to (x,z,y); the additional
    proper rotation (-z,y,x) therefore yields the retained calibration basis.
    This is a mathematical conversion, not proof of imported render buffers.
    """
    matrix = np.asarray(column_major, dtype="f8").reshape(4, 4).T
    linear = matrix[:3, :3]
    inverse = np.linalg.inv(linear)
    world = np.einsum("ij,kj->ki", linear, vertices[:, :3]) + matrix[:3, 3]
    normals = np.einsum("ij,kj->ki", inverse.T, vertices[:, 3:6])
    length = np.linalg.norm(normals, axis=1)
    if not np.isfinite(world).all() or not np.isfinite(normals).all() or np.any(length <= 0):
        raise ValueError("invalid transformed geometry")
    normals /= length[:, None]
    result = vertices.copy()
    result[:, :3] = world[:, [2, 1, 0]] * [-1, 1, 1]
    result[:, 3:6] = normals[:, [2, 1, 0]] * [-1, 1, 1]
    result_indices = indices.copy()
    if np.linalg.det(linear) < 0:
        result_indices.reshape(-1, 3)[:, [1, 2]] = result_indices.reshape(-1, 3)[:, [2, 1]]
    return result, result_indices


def gltf_sampler(reference):
    _, u, v, mag, min_, mip = reference
    wrap = {0: 33071, 1: 33648, 2: 10497}
    return {"wrapS": wrap[u], "wrapT": wrap[v], "magFilter": [9728, 9729][mag],
            "minFilter": [[9728, 9729], [9984, 9985], [9986, 9987]][mip][min_]}


def stage(root, asset=ASSET, cameras=CAMERAS, lighting=LIGHTING):
    root = root.resolve()
    if not root.is_relative_to(PRIVATE.resolve()) or root == PRIVATE.resolve():
        raise ValueError("stage must be a new child of the existing private UE project")
    # Fail before creating output if any registered identity differs.
    identities = {}
    for name, path, wanted in (("asset", asset, ASSET_SHA), ("cameras", cameras, CAMERAS_SHA),
                               ("lighting", lighting, LIGHTING_SHA)):
        actual = digest(path)
        if actual != wanted:
            raise ValueError(f"registered {name} SHA differs: {actual}")
        identities[name] = dict(path=str(path.resolve()), sha256=actual)
    project = PRIVATE / "M5Calibration.uproject"
    if not project.is_file():
        raise ValueError("existing private calibration project missing")
    build_path = ENGINE / "Engine/Build/Build.version"
    build = json.loads(build_path.read_text(encoding="utf-8-sig"))
    if [build[k] for k in ("MajorVersion", "MinorVersion", "PatchVersion", "Changelist")] != [5, 7, 4, 51494982]:
        raise ValueError("unexpected installed Unreal build")
    source = read_layout(asset)
    triangles = sum(p["indices"] // 3 for placement in source["placements"]
                    for p in source["meshes"][placement["mesh"]])
    if len(source["placements"]) != 12 or triangles != 13034903:
        raise ValueError("registered quarry placement/triangle count differs")
    root.mkdir(parents=True, exist_ok=False)
    camera_data = json.loads(cameras.read_text())
    frames = camera_data["frames"]
    if len(frames) != 2 or any((f["width"], f["height"]) != (1920, 1080) for f in frames):
        raise ValueError("registered two-pose camera contract differs")
    # A return pose is explicit and byte-equal in eye/target/up to pose 0.
    poses = [frames[0], frames[1], {**frames[0], "frame": 2, "phase": "return"}]
    write_json(root / "cameras.json", {"frames": poses})
    active = json.loads(lighting.read_text())
    direct = {**active, "environment": [0, 0, 0]}
    write_json(root / "renderlab-directional-black.json", direct)
    gltf = dict(asset={"version": "2.0", "generator": "Renderlab private exact-RLA quarry staging"},
                scene=0, scenes=[{"nodes": []}], nodes=[], meshes=[], materials=[],
                buffers=[{"uri": "quarry.bin", "byteLength": 0}], bufferViews=[], accessors=[],
                images=[], textures=[], samplers=[])
    records = []
    with asset.open("rb") as input_, (root / "quarry.bin").open("wb") as output:
        for pi, placement in enumerate(source["placements"]):
            primitives = []
            for primitive in source["meshes"][placement["mesh"]]:
                input_.seek(primitive["vertex_offset"])
                vertices = np.frombuffer(input_.read(primitive["vertices"] * 48), dtype="<f4").reshape(-1, 12)
                input_.seek(primitive["index_offset"])
                indices = np.frombuffer(input_.read(primitive["indices"] * 4), dtype="<u4")
                if not np.isfinite(vertices).all() or indices.max() >= len(vertices):
                    raise ValueError("invalid source buffers")
                converted, index_data = bake(vertices, indices, placement["transform_column_major"])
                vertex_view = len(gltf["bufferViews"])
                gltf["bufferViews"].append(dict(buffer=0, byteOffset=output.tell(), byteLength=converted.nbytes,
                                              byteStride=48, target=34962))
                output.write(converted.tobytes())
                index_view = len(gltf["bufferViews"])
                gltf["bufferViews"].append(dict(buffer=0, byteOffset=output.tell(), byteLength=index_data.nbytes, target=34963))
                output.write(index_data.tobytes())
                attributes = {}
                for semantic, offset, kind in (("POSITION", 0, "VEC3"), ("NORMAL", 12, "VEC3"),
                                                ("TEXCOORD_0", 24, "VEC2"), ("COLOR_0", 32, "VEC4")):
                    attributes[semantic] = len(gltf["accessors"])
                    accessor = dict(bufferView=vertex_view, byteOffset=offset, componentType=5126,
                                    count=len(vertices), type=kind)
                    if semantic == "POSITION":
                        accessor.update(min=converted[:, :3].min(0).tolist(), max=converted[:, :3].max(0).tolist())
                    gltf["accessors"].append(accessor)
                index_accessor = len(gltf["accessors"])
                gltf["accessors"].append(dict(bufferView=index_view, componentType=5125, count=len(indices), type="SCALAR"))
                primitives.append(dict(attributes=attributes, indices=index_accessor, material=primitive["material"]))
                records.append(dict(placement=pi, material=primitive["material"], vertices=len(vertices),
                                    triangles=len(indices) // 3, gltf_vertex_sha256=hashlib.sha256(converted.tobytes()).hexdigest(),
                                    indices_sha256=hashlib.sha256(index_data.tobytes()).hexdigest(),
                                    expected_ue_bounds_cm=[(100 * converted[:, [0, 2, 1]].min(0)).tolist(),
                                                           (100 * converted[:, [0, 2, 1]].max(0)).tolist()]))
            gltf["scenes"][0]["nodes"].append(pi)
            gltf["nodes"].append(dict(mesh=pi, name=f"Quarry_{pi:02d}"))
            gltf["meshes"].append(dict(name=f"Quarry_{pi:02d}", primitives=primitives))
        gltf["buffers"][0]["byteLength"] = output.tell()
        texture_records = []
        for ti, texture in enumerate(source["textures"]):
            hashes = []
            for mi, mip in enumerate(texture["mips"]):
                input_.seek(mip["offset"])
                pixels = input_.read(mip["size"])
                hashes.append(hashlib.sha256(pixels).hexdigest())
                if mi == 0:
                    image = Image.frombytes("RGBA", (mip["width"], mip["height"]), pixels)
                    if image.getextrema()[3] != (255, 255):
                        raise ValueError("registered texture unexpectedly has transparency")
                    name = f"base-color-{ti:02d}.png"
                    image.save(root / name, compress_level=1)
                    image.close()
                    gltf["images"].append(dict(uri=name))
            texture_records.append(dict(mips=texture["mips"], rgba8_sha256_by_mip=hashes))
    for mi, material in enumerate(source["materials"]):
        factors = material["factors"]
        pbr = dict(baseColorFactor=factors[:4], metallicFactor=factors[4], roughnessFactor=factors[5])
        ref = material["references"][0]
        if ref:
            if ref[0] >= len(source["textures"]):
                raise ValueError("invalid source texture ID")
            pbr["baseColorTexture"] = dict(index=len(gltf["textures"]))
            gltf["textures"].append(dict(source=ref[0], sampler=len(gltf["samplers"])))
            gltf["samplers"].append(gltf_sampler(ref))
        gltf["materials"].append(dict(name=f"QuarryMaterial_{mi:02d}", pbrMetallicRoughness=pbr,
                                      doubleSided=material["double_sided"], alphaMode="OPAQUE"))
    write_json(root / "quarry.gltf", gltf)
    manifest = dict(schema_version=1, status="CPU_STAGED_NOT_IMPORTED", inputs=identities, ue_build=build,
                    existing_project=str(project), existing_project_sha256=digest(project),
                    generator_sha256=digest(Path(__file__)), placements=source["placements"], geometry=records,
                    materials=source["materials"], textures=texture_records, triangles=triangles,
                    registered_lighting=active, directional_black_lighting=direct, poses=poses,
                    source_angle_degrees=math.degrees(2 * .04), requested_source_angle_degrees=4.584,
                    expected_ue_travel=(np.array([.3, -.4, -.8]) / math.sqrt(.89)).tolist(),
                    runtime_status="UNRUN: a staged glTF is not an Unreal import or fidelity certificate",
                    known_mismatches=[
                        "CPU f64 affine bake rounded to f32 versus Renderlab GPU f32 affine; render-buffer/camera audit pending",
                        "UE native importer axis/unit/winding/authored-normal/vertex-color retention needs runtime audit",
                        "Exact 4K mip0 PNG retained; UE-generated lower mips/compression/filtering differ until measured",
                        "Nanite approximation and hardware RT fallback geometry require effective-setting and silhouette audit",
                        "Default Lit sharp-specular disagreement from retained calibration remains unresolved",
                        "8-bit UE light-color quantization; read back reconstructed RGB lux",
                        "4.584 degree diameter rounds exact 4.58366236; cone versus uniform slope disk differs O(alpha^2)",
                        "UE infinite far versus registered 100m far; verify clipping and source 0.5cm near",
                        "Black environment is a matched direct-shadow subset, not the RLE2 complete-frame timing workload",
                        "SceneCapture persistent-view exposure/temporal behavior and profiler main-view correspondence unverified",
                        "1x/no-AA direct ratio baseline differs from any separately measured TSR or hard-4x configuration",
                    ])
    manifest["staged_files_sha256"] = {p.name: digest(p) for p in sorted(root.iterdir()) if p.is_file()}
    write_json(root / "manifest.json", manifest)
    return dict(root=str(root), triangles=triangles, status=manifest["status"], manifest_sha256=digest(root / "manifest.json"))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=PRIVATE / "Quarry")
    args = parser.parse_args()
    print(json.dumps(stage(args.root), indent=2))
