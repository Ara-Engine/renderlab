"""N1c CPU probe: triangles in the clusterLOD cut of the registered quarry package.

Reads only the package metadata (skips embedded resources) and the camera file.
Cut rule (meshoptimizer clusterlod.h): draw a cluster when its owner group's projected
error exceeds the threshold and its refined group's does not (or it is original).

    py -3 -B scripts/n1c-cut-probe.py <package> <cameras.json> [pose ...]
"""
import json
import struct
import sys
from pathlib import Path

import numpy as np


def package(path):
    data = Path(path).read_bytes()
    magic, version, meshes, placements, pages, resources = struct.unpack_from("<4s5I", data, 0)
    assert magic == b"RLV\0" and version == 1
    at = 24 + resources
    out = []
    for _ in range(meshes):
        groups, clusters = struct.unpack_from("<2I", data, at)
        at += 8
        g = np.frombuffer(data, dtype=np.dtype([("depth", "<u4"), ("s", "<f4", 5), ("span", "<u4", 2)]), count=groups, offset=at)
        at += 32 * groups
        c = np.frombuffer(data, dtype=np.dtype([("owner", "<u4"), ("refined", "<u4"), ("b", "<f4", 5), ("material", "<u4"), ("page", "<u4"), ("span", "<u4", 2)]), count=clusters, offset=at)
        at += 44 * clusters
        out.append((g, c))
    p = np.frombuffer(data, dtype=np.dtype([("mesh", "<u4"), ("m", "<f4", 16)]), count=placements, offset=at)
    return out, p


def main():
    meshes, placements = package(sys.argv[1])
    frames = json.loads(Path(sys.argv[2]).read_text())["frames"]
    poses = [int(x) for x in sys.argv[3:]] or [0]
    total = sum(len(c) for _, c in meshes)
    tris = sum(int(c["span"][:, 1].sum()) // 3 for _, c in meshes)
    leaf = sum(int(c["span"][c["refined"] == 0xFFFFFFFF, 1].sum()) // 3 for _, c in meshes)
    print(f"meshes={len(meshes)} placements={len(placements)} clusters={total} dag_triangles={tris} leaf_triangles={leaf}")
    for pose in poses:
        f = frames[pose]
        eye = np.array(f["eye"], np.float64)
        scale = f["height"] * 0.5 / np.tan(f["fov_y_radians"] * 0.5)
        for tau in (-1.0, 0.125, 0.25, 0.5, 1.0, 2.0):
            cut = 0
            cut_leaf = 0
            clusters = 0
            for p in placements:
                g, c = meshes[p["mesh"]]
                m = p["m"].reshape(4, 4).astype(np.float64)  # column-major: m[col][row]
                a = m[:3, :3].T
                s = np.sqrt(np.abs(a).sum(0).max() * np.abs(a).sum(1).max())
                centre = g["s"][:, :3].astype(np.float64) @ a.T + m[3, :3]
                d = np.linalg.norm(centre - eye, axis=1) - g["s"][:, 3] * s
                err = np.where(g["s"][:, 4] == np.float32(np.finfo(np.float32).max), np.inf, g["s"][:, 4] * s / np.maximum(d, f["near"]) * scale)
                owner = err[c["owner"]] > tau
                refined = np.where(c["refined"] == 0xFFFFFFFF, True, err[np.minimum(c["refined"], len(g) - 1)] <= tau)
                keep = owner & refined
                clusters += int(keep.sum())
                cut += int(c["span"][keep, 1].sum()) // 3
                cut_leaf += int(c["span"][keep & (c["refined"] == 0xFFFFFFFF), 1].sum()) // 3
            print(f"pose={pose} tau={tau:g} clusters={clusters} triangles={cut} leaf_triangles={cut_leaf} leaf_share={cut_leaf / max(cut, 1):.3f}")


if __name__ == "__main__":
    main()
