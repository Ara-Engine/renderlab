"""Exact same-ray proof from GPU f32 bits, f32 vertices and the GPU model matrix.

Consumes the receiver-origin diagnostic's rays.json; never starts a renderer.
All intersection arithmetic, including the affine transform, uses rational numbers.
The RLA file supplies only each returned triangle's index-adjacent neighbours.
"""
import argparse
from fractions import Fraction
import importlib.util
import json
from pathlib import Path
import struct

import numpy as np


def dot(a, b):
    return sum(x * y for x, y in zip(a, b))


def sub(a, b):
    return [x - y for x, y in zip(a, b)]


def cross(a, b):
    return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]


def f32(bits):
    return Fraction.from_float(struct.unpack("<f", struct.pack("<I", bits))[0])


def intersect(origin, direction, vertices):
    ab, ac = sub(vertices[1], vertices[0]), sub(vertices[2], vertices[0])
    h = cross(direction, ac)
    determinant = dot(ab, h)
    if not determinant:
        return None
    s = sub(origin, vertices[0])
    u = dot(s, h) / determinant
    q = cross(s, ab)
    v = dot(direction, q) / determinant
    t = dot(ac, q) / determinant
    return dict(t=float(t), exact_t=str(t), barycentrics=[float(1-u-v), float(u), float(v)],
                valid=t > 0 and u >= 0 and v >= 0 and u+v <= 1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rays", type=Path, required=True)
    parser.add_argument("--asset", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location("source_probe", Path(__file__).with_name("quarry-shadow-probe.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    source = module.Source(args.asset)
    rows = []
    for ray in json.loads(args.rays.read_text()):
        slot, primitive = ray["record"][:2]
        mesh, _ = source.placements[slot-1]
        _, vertices, indices = source.meshes[mesh][0]
        returned = indices[primitive]
        # Read the actual returned f32 barycentrics, not reconstructed camera rays.
        u, v = [float(f32(bits)) for bits in ray["record"][2:]]
        opposite = int(np.argmin([1-u-v, u, v]))
        edge = np.delete(returned, opposite)
        adjacent = np.flatnonzero(np.isin(indices, edge).sum(axis=1) == 2)
        model = [[Fraction.from_float(float(x)) for x in row] for row in ray["model"]]
        origin = [f32(x) for x in ray["origin_bits"]]
        direction = [f32(x) for x in ray["direction_bits"]]
        hits = []
        for identity in adjacent:
            local = vertices[indices[identity], :3]
            world = [[model[k][3] + sum(model[k][j] * Fraction.from_float(float(p[j])) for j in range(3))
                      for k in range(3)] for p in local]
            hit = intersect(origin, direction, world)
            if hit is not None:
                hit.update(primitive=int(identity), returned=bool(identity == primitive),
                           object_vertices_match_gpu=bool(np.array_equal(local, ray["vertices"])) if identity == primitive else None)
                hits.append(hit)
        hits.sort(key=lambda h: h["t"])
        valid = [h for h in hits if h["valid"]]
        spacing = float(np.spacing(np.float32(ray["hit_t"])))
        gap = float(Fraction(valid[1]["exact_t"]) - Fraction(valid[0]["exact_t"]))
        rows.append(dict(pose=ray["pose"], origin_bits=ray["origin_bits"], direction_bits=ray["direction_bits"],
            matches_watch=ray["matches_watch"], gpu_primitive=primitive, gpu_t=ray["hit_t"],
            t_float_spacing=spacing, intersections=hits, exact_closest=valid[0]["primitive"],
            t_gap=gap, gap_float_spacings=gap/spacing))
    args.out.write_text(json.dumps(rows, indent=2))
    for row in rows:
        print(row["pose"], row["gpu_primitive"], "->", row["exact_closest"], row["gap_float_spacings"], "float spacings")


if __name__ == "__main__":
    main()
