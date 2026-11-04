"""CPU-only lossless cube-face transport of the registered RLE2 delta radiance.

Unreal performs its own convolution; its SH diffuse and roughness filter remain
known estimator differences from Renderlab's stored diffuse and 65-knot field.
"""
import hashlib
import json
from pathlib import Path
import struct

import numpy as np

SOURCE = Path("D:/Coding/renderlab-work/m5-independent-production/sun128.rle2")
OUTPUT = Path("D:/Coding/renderlab-work/ue-m5/Quarry/environment-v2")
SHA = "3ccd61b142d26b7c04571242eb620729366847674522ef990b06661bd54c1764"


def directions(size):
    v, u = np.meshgrid((np.arange(size) + .5) * 2 / size - 1,
                       (np.arange(size) + .5) * 2 / size - 1, indexing="ij")
    one = np.ones_like(u)
    return np.stack([np.stack(x, -1) for x in ((one, -v, -u), (-one, -v, u),
                     (u, one, v), (u, -one, -v), (u, -v, one), (-u, -v, -one))])


def transport(cube):
    size = cube.shape[1]
    # DDS and the runtime sky shader use DirectX cube directions directly.
    # The Y/Z swap in UE's *longitude-latitude importer* does not apply to DDS.
    ue = directions(size)
    source = ue[..., [1, 2, 0]] * [1, 1, -1]
    major = np.argmax(np.abs(source), axis=-1)
    x, y, z = np.moveaxis(source, -1, 0)
    faces = np.choose(major, [np.where(x > 0, 0, 1), np.where(y > 0, 2, 3), np.where(z > 0, 4, 5)])
    den = np.max(np.abs(source), axis=-1)
    us = np.choose(faces, [-z, z, x, x, x, -x]) / den
    vs = np.choose(faces, [-y, -y, z, -z, -y, -y]) / den
    ix = np.rint((us + 1) * size / 2 - .5).astype(int)
    iy = np.rint((vs + 1) * size / 2 - .5).astype(int)
    addresses = (faces * size + iy) * size + ix
    if len(np.unique(addresses)) != 6 * size * size:
        raise ValueError("basis transport must be an exact texel permutation")
    return cube[faces, iy, ix]


def prepare():
    data = SOURCE.read_bytes()
    if hashlib.sha256(data).hexdigest() != SHA or data[:4] != b"RLE2":
        raise ValueError("registered RLE2 identity differs")
    diffuse, size = struct.unpack_from("<2I", data, 64)
    yaw, scale = struct.unpack_from("<2f", data, 24)
    if yaw != 0 or scale != 1:
        raise ValueError("audit nondefault environment transform before transport")
    offset = 388 + diffuse * diffuse * 6 * 12
    cube = np.frombuffer(data, dtype="<f4", count=6 * size * size * 3, offset=offset).reshape(6, size, size, 3)
    rotated = transport(cube)
    rgba = np.ones((6, size, size, 4), dtype="<f4")
    rgba[..., :3] = rotated / 64  # Exact binary scaling keeps sky filtering within FP16 range.
    header = [124, 0x100F, size, size, size * 16, 0, 1] + [0] * 11
    header += [32, 4, int.from_bytes(b"DX10", "little"), 0, 0, 0, 0, 0]
    header += [0x1008, 0xFE00, 0, 0, 0]
    assert len(header) == 31
    output = b"DDS " + struct.pack("<31I", *header) + struct.pack("<5I", 2, 3, 4, 1, 0) + rgba.tobytes()
    OUTPUT.mkdir(exist_ok=False)
    path = OUTPUT / "quarry-sky-v2.dds"
    path.write_bytes(output)
    record = dict(source=str(SOURCE), source_sha256=SHA, output_sha256=hashlib.sha256(output).hexdigest(),
                  size=size, source_max=float(cube.max()), stored_max=float(rgba[..., :3].max()),
                  exact_texel_permutation=True, source_to_ue="(-Z,X,Y)", radiance_divisor=64,
                  required_skylight_intensity=64, rgba32f=True,
                  mismatches=["UE SH diffuse and native specular convolution replace Renderlab's baked RLE2 filters",
                              "UE processed sky storage/convolution precision requires image calibration"])
    (OUTPUT / "transport.json").write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps(record, indent=2))


if __name__ == "__main__":
    prepare()
