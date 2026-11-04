"""CPU source-ray diagnosis for the saved M6 quarry pose; never starts a renderer.

Uses the immutable RLA1 source and captured camera. Shadow matrices are a Python
replay of shadow_plan.rs, NOT GPU readback. Source intersections do not reproduce
raster subpixel snapping. Keep output in an ignored, new directory.
"""

import argparse
import hashlib
import itertools
import json
import math
import mmap
from pathlib import Path
import struct
import sys
import time

import numpy as np
from PIL import Image


def unit(value):
    return value / np.linalg.norm(value)


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


class Source:
    """Read-only RLA1 views, keeping only a small triangle chunk in scratch RAM."""

    def __init__(self, path):
        self.stream = path.open("rb")
        self.data = mmap.mmap(self.stream.fileno(), 0, access=mmap.ACCESS_READ)
        if self.data[:8] != b"RLA\x00\x01\x00\x00\x00":
            raise ValueError("requires the preserved RLA1 version-1 asset")
        self.at = 8
        mesh_count, placement_count, material_count, self.texture_count = [
            self.u32() for _ in range(4)
        ]
        self.meshes = []
        self.triangles = 0
        for _ in range(mesh_count):
            primitives = []
            for _ in range(self.u32()):
                material, nv, ni = [self.u32() for _ in range(3)]
                vertices = np.ndarray((nv, 12), "<f4", self.data, self.at)
                self.at += nv * 48
                indices = np.ndarray((ni // 3, 3), "<u4", self.data, self.at)
                self.at += ni * 4
                primitives.append((material, vertices, indices))
                self.triangles += ni // 3
            self.meshes.append(primitives)
        self.placements = []
        for _ in range(placement_count):
            mesh = self.u32()
            matrix = np.ndarray((4, 4), "<f4", self.data, self.at).T.copy()
            self.at += 64
            self.placements.append((mesh, matrix))
        self.sided = []
        for _ in range(material_count):
            self.at += 24
            self.sided.append(bool(self.u32()))
            for _ in range(2):
                if self.u32():
                    self.at += 24

    def u32(self):
        value = struct.unpack_from("<I", self.data, self.at)[0]
        self.at += 4
        return value

    def triangle(self, identity):
        mesh, primitive, index = identity
        _, vertices, indices = self.meshes[mesh][primitive]
        matrices = [matrix for placed, matrix in self.placements if placed == mesh]
        if len(matrices) != 1:
            raise ValueError("saved triangle identity requires one placement per mesh")
        matrix = matrices[0]
        return (vertices[indices[index], :3] @ matrix[:3, :3].T + matrix[:3, 3]).astype("f8")

    def bounds(self):
        result = []
        for mesh, model in self.placements:
            for _, vertices, _ in self.meshes[mesh]:
                lo, hi = vertices[:, :3].min(0), vertices[:, :3].max(0)
                corners = np.array(list(itertools.product(*zip(lo, hi))), dtype="f8")
                homogeneous = np.c_[corners, np.ones(8)]
                points = homogeneous @ model[:3].astype("f8").T
                guard = abs(homogeneous) @ abs(model[:3].astype("f8")).T
                guard *= 8 * np.finfo("f4").eps
                lower = np.nextafter((points - guard).min(0).astype("f4"), -np.inf)
                upper = np.nextafter((points + guard).max(0).astype("f4"), np.inf)
                result.append(np.array(list(itertools.product(*zip(lower, upper)))))
        return result

    def intersect(self, origin, direction):
        best, identity, normal = math.inf, None, None
        for mesh, matrix in self.placements:
            for primitive, (material, vertices, indices) in enumerate(self.meshes[mesh]):
                for start in range(0, len(indices), 131072):
                    p = (vertices[indices[start:start + 131072], :3] @ matrix[:3, :3].T
                         + matrix[:3, 3]).astype("f8")
                    e1, e2 = p[:, 1] - p[:, 0], p[:, 2] - p[:, 0]
                    h = np.cross(direction, e2)
                    det = np.einsum("ij,ij->i", e1, h)
                    valid = abs(det) > 1e-12 if self.sided[material] else det > 1e-12
                    inv = np.divide(1., det, out=np.zeros_like(det), where=valid)
                    s = origin - p[:, 0]
                    u = np.einsum("ij,ij->i", s, h) * inv
                    q = np.cross(s, e1)
                    v = (q @ direction) * inv
                    t = np.einsum("ij,ij->i", e2, q) * inv
                    valid &= (u >= 0) & (v >= 0) & (u + v <= 1) & (t > 1e-6) & (t < best)
                    if np.any(valid):
                        t[~valid] = np.inf
                        index = int(np.argmin(t))
                        best = float(t[index])
                        identity = [mesh, primitive, start + index]
                        normal = unit(np.cross(e1[index], e2[index]))
        return best, identity, normal


def roundoff(magnitude):
    return 16 * float(np.finfo("f4").eps) * 2 ** math.ceil(math.log2(max(1., magnitude)))


def replay_views(camera, camera_view, travel, resolution, bounds):
    forward = unit(travel.astype("f8"))
    up = np.eye(3)[np.argmin(abs(forward))]
    right = unit(np.cross(forward, up))
    rotation = np.array([right, np.cross(right, forward), -forward])
    inverse = np.linalg.inv(camera_view)
    ty = math.tan(camera["fov_y_radians"] * .5)
    tx = ty * camera["width"] / camera["height"]
    light_bounds = [corners @ rotation.T for corners in bounds]
    near = camera["near"]
    views = []
    for far in [4., 12., 40.]:
        center = (inverse @ [0, 0, -(near + far) * .5, 1])[:3]
        radius = np.linalg.norm([far * tx, far * ty, (far - near) * .5])
        half = (radius + roundoff(max(abs(center)) + radius)) / (1 - 1 / resolution)
        texel = 2 * half / resolution
        # Rust round() rounds ties away from zero; these centres are not at ties.
        grid = (rotation @ center)[:2] / texel
        xy = np.copysign(np.floor(abs(grid) + .5), grid) * texel
        receivers = np.array([(inverse @ [sx * z * tx, sy * z * ty, -z, 1])[:3]
                              for z in [near, far] for sx in [-1, 1] for sy in [-1, 1]])
        rl = receivers @ rotation.T
        zmin, zmax = rl[:, 2].min(), rl[:, 2].max()
        magnitude = max(abs(center)) + radius
        casters = []
        for index, (world, light) in enumerate(zip(bounds, light_bounds)):
            lo, hi = light.min(0), light.max(0)
            tolerance = roundoff(max(max(abs(lo)), max(abs(hi))))
            if np.any(hi[:2] + tolerance < xy - half) or np.any(lo[:2] - tolerance > xy + half):
                continue
            if hi[2] + tolerance < rl[:, 2].min():
                continue
            casters.append(index)
            zmin, zmax = min(zmin, lo[2]), max(zmax, hi[2])
            magnitude = max(magnitude, abs(world).max())
        pad = max(roundoff(max(magnitude, abs(zmin), abs(zmax))), .001)
        depth_span = zmax - zmin + 2 * pad
        matrix = np.eye(4)
        matrix[:2, :3] = rotation[:2] / half
        matrix[:2, 3] = -xy / half
        matrix[2, :3] = -rotation[2] / depth_span
        matrix[2, 3] = (zmax + pad) / depth_span
        views.append(dict(near=near, far=far, texel_m=float(np.float32(texel)),
                          depth_span_m=depth_span, casters=casters,
                          matrix=matrix.astype("f4").tolist()))
        near = far
    return views


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference-root", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--rays", action="store_true", help="rerun the four light rays and four texel-centre rays")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    start = time.monotonic()
    root = args.reference_root.resolve()
    capture = root / "measurements/m6-gpu/quarry-oracle-1024"
    saved = json.loads((root / ".refs/m6-gpu/quarry-ray-result.json").read_text())
    meta = json.loads((capture / "pose-000.json").read_text())
    scene = json.loads((capture / "scene.json").read_text())
    asset_path = root / "measurements/quarry-quality/detail/full.rla"
    asset_hash = sha256(asset_path)
    if asset_hash != scene["asset_sha256"]:
        raise ValueError("source asset differs from the saved capture")
    source = Source(asset_path)
    view = np.array(meta["identities"]["camera_view"]).reshape(4, 4).T
    projection = np.array(meta["identities"]["camera_projection"]).reshape(4, 4).T
    inverse_camera = np.linalg.inv(projection)
    travel = np.array(json.loads(scene["lighting"])["lights"][0]["travel"], dtype="f4")
    light = -unit(travel.astype("f8"))
    views = replay_views(scene["cameras"][0], view, travel, 1024, source.bounds())
    on = np.array(Image.open(capture / "pose-000.png"), dtype="i2")
    off = np.array(Image.open(capture / "pose-000-disabled.png"), dtype="i2")
    rows = []
    for saved_row in saved["rows"]:
        x, y = saved_row["pixel"]
        world = np.array(saved_row["world"])
        receiver = saved_row["receiver_triangle"]
        if args.rays:
            camera_points = [inverse_camera @ [2 * (x + .5) / 1920 - 1,
                                               1 - 2 * (y + .5) / 1080, z, 1] for z in [0, 1]]
            # Archived captures use forward depth; current captures use reversed depth.
            a, b = sorted((p[:3] / p[3] for p in camera_points),
                          key=lambda p: -(view @ np.r_[p, 1])[2])
            ray = unit(b - a)
            distance, receiver, normal = source.intersect(a, ray)
            if not math.isfinite(distance):
                raise ValueError(f"camera misses source at {(x, y)}")
            world = a + distance * ray
        else:
            triangle = source.triangle(receiver)
            normal = unit(np.cross(triangle[1] - triangle[0], triangle[2] - triangle[0]))
        if normal @ light < 0:
            normal = -normal
        depth = -(view @ np.r_[world, 1])[2]
        layer = next(i for i, v in enumerate(views) if v["near"] <= depth <= v["far"])
        shadow = views[layer]
        matrix = np.array(shadow["matrix"], dtype="f4")
        biased = (world.astype("f4") + normal.astype("f4") * np.float32(.001)).astype("f4")
        ndc = (matrix @ np.r_[biased, np.float32(1)]).astype("f8")[:3]
        uv = ndc[:2] * [.5, -.5] + .5
        pixel = np.clip((uv * 1024).astype(int), 0, 1023)
        sample_ndc = ((pixel + .5) / 1024 - .5) * [2, -2]
        axis = np.array([0., 1., 0.]) if abs(normal[0]) > .9 else np.array([1., 0., 0.])
        tangent = unit(np.cross(normal, axis))
        bitangent = np.cross(normal, tangent)
        plane = np.cross(matrix[:3, :3] @ tangent, matrix[:3, :3] @ bitangent)
        delta = -float(plane[:2] @ (sample_ndc - ndc[:2]) / plane[2])
        allowance = float(sum(abs(plane[:2])) * (2 / 1024) / (16 * abs(plane[2])))
        reference = ndc[2] + delta - allowance - .00002
        inverse = np.linalg.inv(matrix.astype("f8"))
        texel_origin = (inverse @ np.r_[sample_ndc, 0., 1.])[:3]
        clip_camera = projection @ np.r_[world, 1]
        camera_ndc = clip_camera[:3] / clip_camera[3]
        row = dict(pixel=[x, y], receiver_triangle=receiver, world=world.tolist(),
                   saved_receiver_triangle=saved_row["receiver_triangle"],
                   saved_world_distance_m=float(np.linalg.norm(world - saved_row["world"])),
                   normal=normal.tolist(), normal_dot_light=float(normal @ light), cascade=layer,
                   texel_m=shadow["texel_m"], shadow_texel=pixel.tolist(),
                   sample_delta_texels=((sample_ndc - ndc[:2]) * 512).tolist(),
                   plane_delta_m=delta * shadow["depth_span_m"],
                   subpixel_allowance_m=allowance * shadow["depth_span_m"],
                   constant_depth_bias_m=.00002 * shadow["depth_span_m"],
                   reference_depth=reference,
                   proposed_one_texel_cap_depth=float(sum(abs(plane[:2])) * 2 / (1024 * abs(plane[2]))),
                   actual_plane_delta_depth=delta,
                   saved_source_hit_m=saved_row["source_shadow_hit_distance"],
                   baseline_png_on=on[y, x].tolist(), baseline_png_off=off[y, x].tolist(),
                   camera_pixel_reprojected=[float((camera_ndc[0] + 1) * 960),
                                             float((1 - camera_ndc[1]) * 540)])
        if args.rays:
            distance, hit, _ = source.intersect(world + normal * .001, light)
            td, hit_texel, _ = source.intersect(texel_origin, -light)
            if not math.isfinite(td):
                raise ValueError(f"no source hit at reconstructed shadow texel {pixel}")
            hit_world = texel_origin - light * td
            stored = float((matrix.astype("f8") @ np.r_[hit_world, 1])[2])
            gap = (reference - stored) * shadow["depth_span_m"]
            row.update(source_hit_m=distance if math.isfinite(distance) else None, source_hit=hit,
                       sampled_source_triangle=hit_texel, sampled_source_world=hit_world.tolist(),
                       ideal_stored_depth=stored, ideal_false_shadow_gap_m=gap,
                       fixed_texel_extra_normal_offset_to_clear_m=max(0., gap) * float(normal @ light))
        rows.append(row)
        print(json.dumps(row), flush=True)
    roi = off[690:750, 800:1180] - on[690:750, 800:1180]
    report = dict(method=__doc__, asset_sha256=asset_hash,
                  source_triangles=source.triangles, meshes=len(source.meshes), placements=len(source.placements),
                  views=views, rows=rows, supporting_roi=dict(rect=[800, 690, 1180, 750],
                  pixels_mean_rgb_loss_over_1=int(np.count_nonzero(roi.mean(2) > 1))),
                  source_hashes={p: sha256(Path(__file__).resolve().parents[1] / p)
                                 for p in ["src/shadow_plan.rs", "src/shadow_gpu.rs", "shaders/scene.wgsl"]},
                  evidence_hashes={str(p): sha256(p) for p in
                                   [Path(__file__), capture / "pose-000.json", capture / "scene.json",
                                    root / ".refs/m6-gpu/quarry-ray-result.json"]},
                  python=sys.version, numpy=np.__version__,
                  elapsed_seconds=time.monotonic() - start)
    (args.output / "report.json").write_text(json.dumps(report, indent=2, allow_nan=False) + "\n")


if __name__ == "__main__":
    main()
