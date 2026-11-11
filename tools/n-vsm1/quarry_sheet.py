"""Side-by-side sheets from the quarry window batch (CPU): Unreal software Lumen at Epic, today's
default (Surface) and the Disk lookup, on a still frame, a walking frame, the cut frame and the
frames after it, full frame and two crops. Also Disk-minus-Surface statistics per frame."""
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

RUNS = Path("D:/Coding/renderlab-work/n-vsm1/runs")
UE = Path("D:/Coding/renderlab-work/p-yard2/ue/quarry/capture/ue-sw-e")
OUT = Path("D:/Coding/renderlab-work/n-vsm1/sheets")
M = json.loads(Path("D:/Coding/projects/desktop/renderlab/tools/p-yard/manifest.json").read_text())["scenes"]["quarry"]
STOPS = M["exposure_stops"]


def codes(x):
    x = np.maximum(x * np.float32(2.0 ** STOPS), 0)
    x = x / (1 + x)
    return np.clip(255 * np.where(x <= .0031308, 12.92 * x, 1.055 * np.power(x, 1 / 2.4) - .055), 0, 255)


def rl(run, i):
    info = json.loads((RUNS / run / "engine.json").read_text())["hdr"]
    w, h, pre = info["width"], info["height"], info["pre_exposure"]
    raw = np.fromfile(RUNS / run / f"hdr-{i:04d}.bin", "<f2")
    return codes(raw.reshape(h, raw.size // h)[:, :w * 4].reshape(h, w, 4)[..., :3].astype(np.float32) / np.float32(pre))


native = json.loads((UE / "native.json").read_text())["frames"]
first = M["ue_dump_first"]


def ue(i):
    with np.load(native[i - first]["archive"]) as c:
        return codes(c["hdr"][..., :3].astype(np.float32) / np.float32(c["pre_exposure"]))


have = sorted(int(p.stem[4:]) for p in (RUNS / "q-y5-diskcap-nonrt").glob("hdr-*.bin"))
states = {i: f.get("state") for i, f in enumerate(M["frames"])}
print("captured", have[0], "..", have[-1], len(have), {s: [i for i in have if states[i] == s][:3] for s in ("still", "walk", "hold", "return", "cut", "recover")})
pick = [min(i for i in have if states[i] == "still"), [i for i in have if states[i] == "walk"][len([i for i in have if states[i] == "walk"]) // 2],
        next(i for i in have if states[i] == "cut")]
pick += [i for i in have if i > pick[2]][:2]
labels = ["still", "walking", "cut frame", "cut + 1", "cut + 2"]
boxes = {"full": None, "contact": (700, 560, 1180, 830), "far": (1180, 300, 1660, 570)}
for name, box in boxes.items():
    tiles = []
    for i in pick:
        row = [ue(i), rl("q-y5-surfacecap-nonrt", i), rl("q-y5-diskcap-nonrt", i)]
        if box:
            row = [r[box[1]:box[3], box[0]:box[2]] for r in row]
        tiles.append([Image.fromarray(r.astype(np.uint8)) for r in row])
    tw = 640 if box is None else 480
    th = int(tiles[0][0].height * tw / tiles[0][0].width)
    sheet = Image.new("RGB", (tw * 3, (th + 14) * len(pick)), (20, 20, 20))
    d = ImageDraw.Draw(sheet)
    for r, (row, i, label) in enumerate(zip(tiles, pick, labels)):
        for c, (im, col) in enumerate(zip(row, ("Unreal SW Lumen Epic", "Renderlab Surface (default)", "Renderlab Disk"))):
            sheet.paste(im.resize((tw, th), Image.LANCZOS if box is None else Image.NEAREST), (c * tw, r * (th + 14) + 14))
            d.text((c * tw + 3, r * (th + 14) + 1), f"{label} f{i}: {col}", fill=(230, 230, 230))
    sheet.save(OUT / f"quarry-window-{name}.jpg", quality=90)
    print(name, sheet.size)
# Disk against Surface per picked frame: how many pixels differ by more than 8 codes, and specks.
for i, label in zip(pick, labels):
    s, dk = rl("q-y5-surfacecap-nonrt", i), rl("q-y5-diskcap-nonrt", i)
    diff = np.abs(s - dk).max(axis=2)
    lum = dk.max(axis=2)
    c = lum[1:-1, 1:-1]
    nb = np.max(np.stack([lum[1 + dy:lum.shape[0] - 1 + dy, 1 + dx:lum.shape[1] - 1 + dx] for dy in (-1, 0, 1) for dx in (-1, 0, 1) if (dy, dx) != (0, 0)]), axis=0)
    ls = s.max(axis=2); cs = ls[1:-1, 1:-1]
    nbs = np.max(np.stack([ls[1 + dy:ls.shape[0] - 1 + dy, 1 + dx:ls.shape[1] - 1 + dx] for dy in (-1, 0, 1) for dx in (-1, 0, 1) if (dy, dx) != (0, 0)]), axis=0)
    print(f"{label:10s} f{i}: mean |disk-surface| {diff.mean():.3f} codes, >8 codes {100 * (diff > 8).mean():.3f}% , >32 codes {int((diff > 32).sum())} px; "
          f"specks (>40 over all neighbours) disk {int(((c - nb) > 40).sum())} surface {int(((cs - nbs) > 40).sum())}")
