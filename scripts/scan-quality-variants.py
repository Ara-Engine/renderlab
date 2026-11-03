"""Plan or create private texture variants without importing or simplifying scan geometry."""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct

import numpy as np
from PIL import Image


U32_MAX = (1 << 32) - 1
MIB = 1 << 20


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def copy_span(source, output, start, length):
    source.seek(start)
    identity = hashlib.sha256()
    while length:
        block = source.read(min(length, MIB))
        if not block:
            raise ValueError("truncated input span")
        if output is not None:
            output.write(block)
        identity.update(block)
        length -= len(block)
    return identity.hexdigest()


def layout(path):
    """Read only schema-1 record boundaries. The renderer remains the semantic validator."""
    size = path.stat().st_size
    with path.open("rb") as source:
        def words(count=1):
            data = source.read(4 * count)
            if len(data) != 4 * count:
                raise ValueError("truncated RLA record")
            return struct.unpack("<" + "I" * count, data)

        def skip(length):
            if source.tell() + length > size:
                raise ValueError("RLA span exceeds input")
            source.seek(length, 1)

        if source.read(8) != b"RLA\0\1\0\0\0":
            raise ValueError("expected schema-1 RLA")
        meshes, placements, materials, textures = words(4)
        mesh_records = []
        for _ in range(meshes):
            primitives = []
            for _ in range(words()[0]):
                material, vertices, indices = words(3)
                primitives.append((material, vertices, indices))
                skip(vertices * 48 + indices * 4)
            mesh_records.append(primitives)
        skip(placements * 68)
        material_start = source.tell()
        bindings = []
        for _ in range(materials):
            skip(28)
            refs = []
            for _ in range(2):
                present = words()[0]
                if present not in (0, 1):
                    raise ValueError("invalid material reference")
                refs.append(words(6) if present else None)
            bindings.append(refs)
        texture_start = source.tell()
        spans = []
        for _ in range(textures):
            start = source.tell()
            role, levels = words(2)
            if role != 0 or not 1 <= levels <= 32:
                raise ValueError("quality variants require opaque base-color textures")
            dimensions = []
            for _ in range(levels):
                w, h, length = words(3)
                if not w or not h or length != w * h * 4:
                    raise ValueError("invalid texture span")
                if dimensions and (w, h) != tuple(max(1, v // 2) for v in dimensions[-1]):
                    raise ValueError("invalid mip progression")
                dimensions.append((w, h))
                skip(length)
            if dimensions[-1] != (1, 1) or dimensions.count((1, 1)) != 1:
                raise ValueError("incomplete or repeated final mip")
            spans.append({"start": start, "length": source.tell() - start,
                          "size": dimensions[0][0], "dimensions": dimensions})
        if source.tell() != size:
            raise ValueError("trailing RLA bytes")
    return {"meshes": mesh_records, "bindings": bindings, "textures": spans,
            "material_start": material_start, "texture_start": texture_start}


def mip_bytes(size):
    return sum(4 * (size >> level) ** 2 for level in range(size.bit_length()))


def half_srgb(pixels):
    """Power-of-two opaque area mips, linear RGB, rounded to RGBA8 after each level."""
    h, w, _ = pixels.shape
    if h != w or w < 2 or w & (w - 1) or not np.all(pixels[:, :, 3] == 255):
        raise ValueError("expected square power-of-two opaque RGBA8")
    x = np.arange(256, dtype=np.float64) / 255
    linear = np.where(x <= 0.04045, x / 12.92, ((x + 0.055) / 1.055) ** 2.4)
    result = np.full((h // 2, w // 2, 4), 255, dtype=np.uint8)
    for y in range(0, h // 2, 32):
        rows = pixels[2*y:2*min(y+32, h//2), :, :3]
        value = linear[rows[0::2, 0::2]] + linear[rows[0::2, 1::2]]
        value += linear[rows[1::2, 0::2]]
        value += linear[rows[1::2, 1::2]]
        value *= 0.25
        encoded = np.where(value <= 0.0031308, value * 12.92,
                           1.055 * value ** (1 / 2.4) - 0.055)
        result[y:y+len(rows)//2, :, :3] = np.floor(encoded * 255 + 0.5).clip(0, 255).astype(np.uint8)
    return result


def write_texture(output, path, size):
    with Image.open(path) as image:
        w, h = image.size
        if w != h or w < size or w & (w - 1):
            raise ValueError("expected a square power-of-two source at least as large as the variant")
        pixels = np.array(image.convert("RGBA"))
    while pixels.shape[0] > size:
        pixels = half_srgb(pixels)
    output.write(struct.pack("<II", 0, size.bit_length()))
    while True:
        h, w, _ = pixels.shape
        output.write(struct.pack("<III", w, h, pixels.nbytes))
        output.write(memoryview(pixels))
        if w == 1:
            return
        pixels = half_srgb(pixels)


def quality_cameras(canonical):
    frames = []
    for index in (0, 4):
        frames.append(copy.deepcopy(canonical["frames"][index]))
    a, b = (canonical["frames"][i] for i in (8, 11))
    for step in range(9):
        pose = copy.deepcopy(a)
        for key in ("eye", "target"):
            pose[key] = [x + (y - x) * step / 8 for x, y in zip(a[key], b[key])]
        pose["phase"] = "grazing_motion"
        frames.append(pose)
    frames.append(copy.deepcopy(canonical["frames"][12]))
    for index, pose in enumerate(frames):
        pose["frame"] = index
    reference = copy.deepcopy(frames)
    for pose in reference:
        pose["width"] *= 2
        pose["height"] *= 2
        if max(pose["width"], pose["height"]) > 8192:
            raise ValueError("supersampled camera exceeds replay extent")
    return {"frames": frames}, {"frames": reference}


def virtual_metadata(cache, manifest):
    raw = manifest.read_bytes()
    if len(raw) != 80 or raw[:8] != b"RVM\0\1\0\0\0":
        raise ValueError("expected schema-1 virtual manifest")
    path = cache / "virtual-v1/packages" / raw[40:72].hex()
    with path.open("rb") as source:
        header = source.read(24)
    if len(header) != 24 or header[:8] != b"RLV\0\1\0\0\0" or path.stat().st_size != struct.unpack_from("<Q", raw, 72)[0]:
        raise ValueError("invalid virtual package header or length")
    resource_size = struct.unpack_from("<I", header, 20)[0]
    pages = struct.unpack_from("<I", header, 16)[0]
    if 24 + resource_size + pages * 44 > path.stat().st_size:
        raise ValueError("invalid virtual resource or page span")
    return raw, path, header, resource_size, pages


def create_variant(plan, asset, records, replacements, virtual):
    out = Path(plan["output"])
    full = out / "full.rla"
    with asset.open("rb") as source, full.open("xb") as output:
        geometry_hash = copy_span(source, output, 0, records["texture_start"])
        for index, span in enumerate(records["textures"]):
            if index in replacements:
                item = replacements[index]
                if digest(Path(item["path"])) != item["sha256"]:
                    raise ValueError("source texture digest differs from intake")
                write_texture(output, Path(item["path"]), item["size"])
            else:
                copy_span(source, output, span["start"], span["length"])
    if full.stat().st_size != plan["full_bytes"]:
        raise ValueError("variant size differs from plan")
    raw, package, header, resource_size, pages = virtual
    with asset.open("rb") as source, package.open("rb") as old:
        old.seek(24)
        expected_header = b"RLA\0\1\0\0\0" + struct.pack("<IIII", 0, 0, len(records["bindings"]), len(records["textures"]))
        if old.read(24) != expected_header:
            raise ValueError("canonical virtual resources differ from full RLA")
        suffix_size = asset.stat().st_size - records["material_start"]
        if resource_size != 24 + suffix_size:
            raise ValueError("canonical resource sizes differ")
        if copy_span(source, None, records["material_start"], suffix_size) != copy_span(old, None, 48, suffix_size):
            raise ValueError("canonical virtual materials/textures differ from full RLA")
    destination = out / "cache/virtual-v1"
    (destination / "packages").mkdir(parents=True)
    (destination / "pages").mkdir()
    new_package = destination / "packages/pending"
    with package.open("rb") as old, full.open("rb") as source, new_package.open("xb") as output:
        output.write(header[:20] + struct.pack("<I", plan["resource_bytes"]))
        output.write(expected_header)
        copy_span(source, output, records["material_start"], full.stat().st_size - records["material_start"])
        tail_hash = copy_span(old, output, 24 + resource_size, package.stat().st_size - 24 - resource_size)
    package_hash = digest(new_package)
    new_package.rename(destination / "packages" / package_hash)
    with package.open("rb") as source:
        source.seek(-pages * 44, 2)
        for _ in range(pages):
            record = source.read(44)
            name = record[12:44].hex()
            original = Path(plan["canonical_cache"]) / "virtual-v1/pages" / name
            if original.stat().st_size != 65536:
                raise ValueError("canonical page extent differs")
            target = destination / "pages" / name
            if not target.exists():
                os.link(original, target)
    identity = hashlib.sha256(json.dumps(plan, sort_keys=True).encode()).digest()
    manifest = b"RVM\0\1\0\0\0" + identity + bytes.fromhex(package_hash) + struct.pack("<Q", plan["package_bytes"])
    (out / "virtual.manifest").write_bytes(manifest)
    plan["proof"] = {"unchanged_rla_prefix_sha256": geometry_hash,
                     "unchanged_rlv_geometry_placements_page_table_sha256": tail_hash,
                     "full_sha256": digest(full), "package_sha256": package_hash,
                     "pages": pages, "page_reuse": "hard links, not rewritten or recooked",
                     "key_namespace": "private scan-quality variant, not a cooker cache key"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--size", type=int, choices=(2048, 4096, 8192), help="omit to prepare only camera/AA commands")
    parser.add_argument("--ids", nargs="+", help="promote only these scans; others retain canonical texels")
    parser.add_argument("--create", action="store_true", help="write a fresh directory; otherwise print the plan only")
    parser.add_argument("--replay", type=Path, default=Path("target/release/examples/virtual-stream.exe"))
    args = parser.parse_args()
    root, out = args.root.resolve(), args.output.resolve()
    if out.drive.lower() != "d:" or out.exists() or out == root or root in out.parents:
        raise ValueError("choose a fresh D-drive directory outside the canonical scan root")
    if args.ids and args.size is None:
        raise ValueError("--ids requires --size")
    intake = json.loads((root / "scan-manifest.json").read_text())
    composition = json.loads((root / "quarry.json").read_text())
    rlm = (root / "quarry.manifest").read_bytes()
    if len(rlm) < 84 or rlm[:8] != b"RLM\0\1\0\0\0":
        raise ValueError("expected schema-1 prepared manifest")
    asset = root / "cache/assets" / rlm[40:72].hex()
    records = layout(asset)
    entries = composition["entries"]
    if len(entries) != len(records["meshes"]):
        raise ValueError("composition and prepared mesh counts differ")
    scans = {s["id"]: s for s in intake["scans"]}
    selected = set(args.ids or scans) if args.size else set()
    if selected - scans.keys():
        raise ValueError("unknown scan ID")
    replacements = {}
    for entry, primitives in zip(entries, records["meshes"]):
        if len(primitives) != 1 or primitives[0][1:] != (entry["vertices"], entry["triangles"] * 3):
            raise ValueError("expected one unchanged primitive per registered scan")
        scan_id = entry["scan_id"]
        binding, mr = records["bindings"][primitives[0][0]]
        if binding is None or mr is not None:
            raise ValueError("expected a single base-color binding")
        index = binding[0]
        if scan_id in selected:
            scan = scans[scan_id]
            name = scan["inspection"]["export"]["source_basecolor"]
            maps = [m for m in scan["texture_maps"] if Path(m["entry"]).name == name]
            if len(maps) != 1 or index in replacements:
                raise ValueError("ambiguous texture mapping")
            source_path = root / "source" / scan_id / name
            with Image.open(source_path) as image:
                if image.size != (8192, 8192):
                    raise ValueError("source dimensions differ from the recorded 8K texture")
            replacements[index] = {"scan_id": scan_id, "path": str(source_path),
                                   "sha256": maps[0]["sha256"], "size": args.size}
    sizes = [replacements[i]["size"] if i in replacements else t["size"] for i, t in enumerate(records["textures"])]
    if any(t["dimensions"][0] != (t["size"], t["size"]) or t["size"] & (t["size"] - 1) for t in records["textures"]):
        raise ValueError("quality variant requires square power-of-two baseline textures")
    texture_bytes = sum(map(mip_bytes, sizes))
    encoded_textures = sum(mip_bytes(s) + 8 + s.bit_length() * 12 for s in sizes)
    resource_bytes = 24 + records["texture_start"] - records["material_start"] + encoded_textures
    if resource_bytes > U32_MAX:
        raise ValueError(f"RLV resource span {resource_bytes} exceeds u32. Promote a subset at 8K instead.")
    virtual = virtual_metadata(root / "cache", root / "quarry-virtual.manifest")
    _, package, _, old_resources, _ = virtual
    full_bytes = records["texture_start"] + encoded_textures
    package_bytes = package.stat().st_size - old_resources + resource_bytes
    limits = json.loads((root / "limits.json").read_text())
    for section, key, minimum in [
        ("prepared", "max_dimension", max(sizes)),
        ("prepared", "max_file_bytes", full_bytes + 64 * MIB),
        ("prepared", "max_decoded_bytes", full_bytes + 128 * MIB),
        ("runtime", "texture_bytes", texture_bytes + 16 * MIB),
        ("runtime", "bootstrap_bytes", package_bytes + 128 * MIB),
        ("renderer", "max_texture_bytes", max(map(mip_bytes, sizes)) + MIB),
    ]:
        fallback = 4096 if (section, key) == ("prepared", "max_dimension") else 0
        current = limits.setdefault(section, {}).get(key, fallback)
        if not isinstance(current, int):
            raise ValueError("use finite baseline policy for quality variants")
        limits[section][key] = max(current, minimum)
    cameras, reference = quality_cameras(json.loads((root / "cameras.json").read_text()))
    plan = {"output": str(out), "canonical_asset": str(asset), "canonical_cache": str(root / "cache"),
            "canonical_payload_sha256": rlm[40:72].hex(), "texture_bytes": texture_bytes,
            "canonical_package_sha256": virtual[0][40:72].hex(),
            "texture_mib": texture_bytes / MIB, "full_bytes": full_bytes,
            "package_bytes": package_bytes, "resource_bytes": resource_bytes,
            "replacements": replacements, "filter": "opaque linear-light 2x2 box, RGBA8 rounding per level",
            "canonical_manifest_sha256": digest(root / "quarry.manifest"),
            "canonical_virtual_manifest_sha256": digest(root / "quarry-virtual.manifest"),
            "canonical_cameras_sha256": digest(root / "cameras.json"),
            "canonical_limits_sha256": digest(root / "limits.json"),
            "variant_script_sha256": digest(Path(__file__)), "limits": limits,
            "motion_pairs": [[i, i + 1] for i in range(2, 10)],
            "acceptance": {"background_rgb8": [36, 44, 56], "edge_mae_max": 0.04,
                           "edge_p95_max": 0.15, "smooth_mae_max": 0.02,
                           "coverage_mismatch_fraction_max": 0.0002,
                           "temporal_edge_mae_max": 0.04, "temporal_texture_mae_max": 0.025,
                           "edges_improvement_min": 0.20, "texture_improvement_min": 0.10},
            "scope": "quality only, unlit opaque base color; no speed or whole-engine claim"}
    plan["commands"] = {}
    for label, samples, camera_file in [("full-1", "1", "cameras.json"), ("full-4", "4", "cameras.json"), ("reference-4", "4", "reference-cameras.json")]:
        plan["commands"][label] = [str(args.replay.resolve()), "--mode", "full", "--asset", str(out / "full.rla") if args.size else str(asset),
                                   "--cameras", str(out / camera_file), "--limits", str(out / "limits.json"),
                                   "--samples", samples, "--frames", "12", "--capture", ",".join(map(str, range(12))),
                                   "--validate", "--output", str(out / label)]
    if args.create:
        out.parent.mkdir(parents=True, exist_ok=True)
        required = full_bytes + package_bytes if args.size else 0
        if shutil.disk_usage(out.parent).free < required + 2 * (1 << 30):
            raise ValueError("insufficient disk headroom")
        if digest(asset) != rlm[40:72].hex() or digest(package) != virtual[0][40:72].hex():
            raise ValueError("canonical payload digest differs from manifest")
        out.mkdir()
        if args.size:
            create_variant(plan, asset, records, replacements, virtual)
        for name, data in [("cameras.json", cameras), ("reference-cameras.json", reference), ("limits.json", limits), ("plan.json", plan)]:
            (out / name).write_text(json.dumps(data, indent=2) + "\n")
    print(json.dumps(plan, indent=2))


if __name__ == "__main__":
    main()
