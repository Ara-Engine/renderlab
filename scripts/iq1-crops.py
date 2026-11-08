"""IQ1 (docs/iq1-quarry-image.md): 3x crops of Islam's two quarry regions (face: x1150 y400,
edge: x880 y480, 240x160) through U0b's F0 transform at -14 stops, one strip per call:
the reference first, then each named dump. Pinholes (scripts/iq1-analyse.py) are ringed
in red on a second strip when --mark is given. CPU only.

  py -3 -B scripts/iq1-crops.py <out dir> <tag> [--mark] <dump.f32> ...
"""
import importlib.util
import sys
from pathlib import Path

import numpy as np
from PIL import Image

spec = importlib.util.spec_from_file_location("iq1", Path(__file__).with_name("iq1-analyse.py"))
iq1 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(iq1)
REGIONS = {"face": (1150, 400), "edge": (880, 480)}
SIZE, SCALE = (240, 160), 3


def main():
    out, tag = Path(sys.argv[1]), sys.argv[2]
    args = sys.argv[3:]
    mark = "--mark" in args
    files = [Path(a) for a in args if a != "--mark"]
    pose = int(files[0].stem.rsplit("pose", 1)[1][0])
    with np.load(iq1.U0B / f"quarry-p{pose}.npz") as r:
        ref = (r["L0"] + r["L1"]) / 2
    images = [ref] + [iq1.radiance(f) for f in files]
    out.mkdir(parents=True, exist_ok=True)
    for name, (x, y) in REGIONS.items():
        tiles = []
        for k, img in enumerate(images):
            rgb = np.clip(np.rint(iq1.sc.codes(img, iq1.EXPOSURE)), 0, 255).astype(np.uint8)
            if mark and k > 0:
                m = iq1.pinholes(img, ref, img.sum(-1) > 0)
                ring = (iq1.neighbours(m.astype(np.uint8)).max(0) > 0) & ~m
                rgb = rgb.copy()
                rgb[ring] = (255, 0, 0)
            crop = rgb[y:y + SIZE[1], x:x + SIZE[0]]
            tiles.append(Image.fromarray(crop).resize((SIZE[0] * SCALE, SIZE[1] * SCALE), Image.NEAREST))
        strip = Image.new("RGB", (sum(t.width for t in tiles) + 10 * (len(tiles) - 1), tiles[0].height), (255, 255, 255))
        offset = 0
        for t in tiles:
            strip.paste(t, (offset, 0))
            offset += t.width + 10
        strip.save(out / f"{tag}-{name}.png")


if __name__ == "__main__":
    main()
