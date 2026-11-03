"""Run in background Blender. Inspect one unchanged FBX and optionally export full geometry."""
from __future__ import annotations

import argparse
from array import array
import hashlib
import json
from pathlib import Path
import sys
import time
import struct

import bpy
import numpy as np
from mathutils import Vector


def bounds(points):
    points = list(points)
    return [[min(p[i] for p in points) for i in range(3)],
            [max(p[i] for p in points) for i in range(3)]]


def preserve_static_normals(exporter):
    """The stock Blender 5.1 exporter rounds normals to four decimals. Keep imported values."""
    if exporter.key_blocks or exporter.armature:
        raise ValueError("normal preservation adapter is static-mesh only")
    exporter.normals = np.empty((len(exporter.blender_mesh.loops), 3), dtype=np.float32)
    exporter.blender_mesh.corner_normals.foreach_get("vector", exporter.normals.ravel())
    exporter.morph_normals = []
    if exporter.export_settings["gltf_yup"]:
        exporter.zup2yup(exporter.normals)


def verify_export(path, meshes):
    with path.open("rb") as source:
        source.seek(12)
        length, _ = struct.unpack("<II", source.read(8))
        doc = json.loads(source.read(length))
        length, _ = struct.unpack("<II", source.read(8))
        binary = source.read(length)

    def accessor(index):
        item = doc["accessors"][index]
        view = doc["bufferViews"][item["bufferView"]]
        width = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}[item["type"]]
        dtype = {5126: "<f4", 5125: "<u4", 5123: "<u2", 5121: "u1"}[item["componentType"]]
        assert "byteStride" not in view, "verification requires packed exporter accessors"
        return np.frombuffer(binary, dtype=dtype, count=item["count"] * width,
                             offset=view.get("byteOffset", 0) + item.get("byteOffset", 0)).reshape(-1, width)

    results = []
    for obj in meshes:
        mesh = obj.data
        node = next(n for n in doc["nodes"] if n.get("name") == obj.name and "mesh" in n)
        primitives = doc["meshes"][node["mesh"]]["primitives"]
        assert len(primitives) == 1, "opaque scan verification expects one source material"
        primitive = primitives[0]
        indices = accessor(primitive["indices"]).ravel()
        positions = accessor(primitive["attributes"]["POSITION"])[indices]
        normals = accessor(primitive["attributes"]["NORMAL"])[indices]
        uv = accessor(primitive["attributes"]["TEXCOORD_0"])[indices]
        triangles = np.empty(len(mesh.loop_triangles) * 3, dtype=np.int32)
        mesh.loop_triangles.foreach_get("loops", triangles)
        loop_vertices = np.empty(len(mesh.loops), dtype=np.int32)
        mesh.loops.foreach_get("vertex_index", loop_vertices)
        source_positions = np.empty(len(mesh.vertices) * 3, dtype=np.float32)
        mesh.vertices.foreach_get("co", source_positions)
        source_positions = source_positions.reshape(-1, 3)[loop_vertices[triangles]][:, [0, 2, 1]]
        source_positions[:, 2] *= -1
        source_normals = np.empty(len(mesh.corner_normals) * 3, dtype=np.float32)
        mesh.corner_normals.foreach_get("vector", source_normals)
        source_normals = source_normals.reshape(-1, 3)[triangles][:, [0, 2, 1]]
        source_normals[:, 2] *= -1
        source_uv = np.empty(len(mesh.loops) * 2, dtype=np.float32)
        mesh.uv_layers.active.data.foreach_get("uv", source_uv)
        source_uv = source_uv.reshape(-1, 2)[triangles]
        source_uv[:, 1] = 1 - source_uv[:, 1]
        assert np.array_equal(positions, source_positions), "export changed triangle corner positions or order"
        normal_error = float(np.max(np.abs(normals - source_normals)))
        print("SCAN_NORMAL_CHECK", obj.name, normal_error,
              "source_length", float(np.min(np.linalg.norm(source_normals, axis=1))),
              float(np.max(np.linalg.norm(source_normals, axis=1))), flush=True)
        assert np.array_equal(normals, source_normals), "export changed supplied corner normals"
        assert np.array_equal(uv, source_uv), "export changed UVs beyond glTF V flip"
        results.append({"object": obj.name, "triangles": len(indices) // 3,
                        "triangle_corner_positions_exact": True, "uv0_exact_after_v_flip": True,
                        "max_corner_normal_error": normal_error})
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--export", action="store_true")
    parser.add_argument("--preview", action="store_true")
    parser.add_argument("--texture-size", type=int, default=2048)
    args = parser.parse_args(sys.argv[sys.argv.index("--") + 1:])
    args.output.mkdir(parents=True, exist_ok=True)
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.context.scene.render.threads_mode = "FIXED"
    bpy.context.scene.render.threads = 2
    start = time.perf_counter()
    bpy.ops.import_scene.fbx(filepath=str(args.source.resolve()), use_custom_normals=True)
    imported = time.perf_counter()
    meshes = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    records = []
    world_points = []
    for obj in meshes:
        mesh = obj.data
        mesh.calc_loop_triangles()
        world = [obj.matrix_world @ Vector(c) for c in obj.bound_box]
        world_points.extend(world)
        coordinates = array("f", [0]) * (len(mesh.vertices) * 3)
        mesh.vertices.foreach_get("co", coordinates)
        indices = array("i", [0]) * (len(mesh.loop_triangles) * 3)
        mesh.loop_triangles.foreach_get("vertices", indices)
        normals = array("f", [0]) * (len(mesh.corner_normals) * 3)
        mesh.corner_normals.foreach_get("vector", normals)
        records.append({"name": obj.name, "vertices": len(mesh.vertices),
                        "polygons": len(mesh.polygons), "triangles": len(mesh.loop_triangles),
                        "corners": len(mesh.loops), "has_custom_normals": mesh.has_custom_normals,
                        "normal_count": len(mesh.corner_normals),
                        "uv_layers": [x.name for x in mesh.uv_layers],
                        "color_attributes": [x.name for x in mesh.color_attributes],
                        "material_slots": [x.name if x else None for x in mesh.materials],
                        "material_triangle_counts": {str(i): sum(t.material_index == i for t in mesh.loop_triangles)
                                                     for i in range(max(1, len(mesh.materials)))},
                        "matrix_world": [list(row) for row in obj.matrix_world],
                        "bounds_blender_world": bounds(world),
                        "position_f32_sha256": hashlib.sha256(coordinates.tobytes()).hexdigest(),
                        "triangle_vertex_i32_sha256": hashlib.sha256(indices.tobytes()).hexdigest(),
                        "corner_normal_f32_sha256": hashlib.sha256(normals.tobytes()).hexdigest()})
    report = {"source": str(args.source.resolve()), "blender": bpy.app.version_string,
              "import_seconds": imported - start, "objects": records,
              "bounds_blender_world": bounds(world_points),
              "source_materials": [{"name": m.name, "images": [n.image.filepath for n in m.node_tree.nodes
                                   if n.type == "TEX_IMAGE" and n.image] if m.use_nodes else []}
                                   for m in bpy.data.materials],
              "units": {"system": bpy.context.scene.unit_settings.system,
                        "scale_length": bpy.context.scene.unit_settings.scale_length},
              "geometry_policy": "No modifiers, welding, decimation or geometry normalization. FBX importer axis/unit conversion; glTF Y-up export."}
    report_path = args.output / ("inspection.json" if args.export or args.preview else "inventory.json")
    report_path.write_text(json.dumps(report, indent=2) + "\n")
    if not args.export and not args.preview:
        print("SCAN_RESULT " + json.dumps(report), flush=True)
        return
    maps = list(args.source.parent.glob("*.jpg"))
    base = next((p for p in maps if p.stem.lower().endswith(("_basecolor", "_albedo")) and "billboard" not in p.stem.lower()), None)
    if base is None:
        raise ValueError("no unambiguous base color map")
    source_image = bpy.data.images.load(str(base.resolve()), check_existing=False)
    original_size = list(source_image.size)
    source_image.scale(args.texture_size, args.texture_size)
    source_image.filepath_raw = str((args.output / "basecolor.jpg").resolve())
    source_image.file_format = "JPEG"
    source_image.save()
    material = bpy.data.materials.new("scan_basecolor_only")
    material.use_nodes = True
    material.diffuse_color = (1, 1, 1, 1)
    bsdf = material.node_tree.nodes.get("Principled BSDF")
    bsdf.inputs["Metallic"].default_value = 0
    bsdf.inputs["Roughness"].default_value = 0.85
    tex = material.node_tree.nodes.new("ShaderNodeTexImage")
    tex.image = source_image
    material.node_tree.links.new(tex.outputs["Color"], bsdf.inputs["Base Color"])
    for obj in meshes:
        if len(obj.data.materials) > 1 or len(obj.data.uv_layers) != 1 or len(obj.data.color_attributes):
            raise ValueError("conversion requires one UV set, at most one material, and no source vertex colors")
        for i in range(len(obj.data.materials)):
            obj.data.materials[i] = material
        if len(obj.data.materials) == 0:
            obj.data.materials.append(material)
    if args.export:
        from io_scene_gltf2.blender.exp.primitive_extract import PrimitiveCreator
        if bpy.app.version[:2] != (5, 1):
            raise ValueError("normal preservation adapter verified only with Blender 5.1")
        PrimitiveCreator._PrimitiveCreator__get_normals = preserve_static_normals
        bpy.ops.export_scene.gltf(filepath=str((args.output / "scan.glb").resolve()),
                                  export_format="GLB", export_yup=True, export_normals=True,
                                  export_tangents=False, export_texcoords=True, export_vertex_color="NONE",
                                  export_materials="EXPORT", export_cameras=False,
                                  export_animations=False, export_extras=False,
                                  export_image_format="JPEG", export_jpeg_quality=95)
        report["geometry_verification"] = verify_export(args.output / "scan.glb", meshes)
        report["normal_export_policy"] = "Process-local Blender 5.1 static-normal adapter bypasses exporter rounding. Triangle-corner normals verified exact after Y-up basis conversion. Installed exporter source unchanged."
        report["export"] = {"path": "scan.glb", "source_basecolor": base.name,
                            "source_basecolor_size": original_size,
                            "initial_basecolor_size": [args.texture_size, args.texture_size],
                            "material_policy": "Unlit base color only in renderer. Source roughness, normal, AO, bump, cavity, gloss and specular remain unused. No lighting baked by conversion.",
                            "seconds_since_import": time.perf_counter() - imported}
    if args.preview:
        lo, hi = map(Vector, report["bounds_blender_world"])
        center = (lo + hi) / 2
        radius = max((hi - lo).length / 2, 0.1)
        camera_data = bpy.data.cameras.new("inspection_camera")
        camera = bpy.data.objects.new("inspection_camera", camera_data)
        bpy.context.collection.objects.link(camera)
        camera.location = center + Vector((1.2, -1.8, 1.0)) * radius
        camera.rotation_euler = (center - camera.location).to_track_quat("-Z", "Y").to_euler()
        camera_data.clip_start = radius / 1000
        camera_data.clip_end = radius * 20
        bpy.context.scene.camera = camera
        # Emission gives an unlit content preview, not a prettier reference renderer.
        for link in list(material.node_tree.links):
            material.node_tree.links.remove(link)
        emission = material.node_tree.nodes.new("ShaderNodeEmission")
        material.node_tree.links.new(tex.outputs["Color"], emission.inputs["Color"])
        material.node_tree.links.new(emission.outputs[0], material.node_tree.nodes.get("Material Output").inputs["Surface"])
        scene = bpy.context.scene
        scene.render.engine = "BLENDER_EEVEE"
        scene.eevee.taa_render_samples = 1
        scene.render.resolution_x, scene.render.resolution_y = 640, 480
        scene.render.resolution_percentage = 100
        scene.view_settings.view_transform = "Standard"
        scene.render.filepath = str((args.output / "preview.png").resolve())
        bpy.ops.render.render(write_still=True)
    report_path.write_text(json.dumps(report, indent=2) + "\n")
    print("SCAN_RESULT " + json.dumps(report), flush=True)


if __name__ == "__main__":
    main()
