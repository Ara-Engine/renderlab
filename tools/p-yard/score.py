"""P-YARD2 scoring (CPU only): full-frame error against U0b's path-traced quarry references,
GPU frame time per state, the error-vs-ms chart and CSV, and crop sheets.

    py -3 -B tools/p-yard/score.py identity     # reference scene identity vs the current inputs
    py -3 -B tools/p-yard/score.py quality      # every captured config, per frame, per state
    py -3 -B tools/p-yard/score.py timing       # GPU ms per state for both engines
    py -3 -B tools/p-yard/score.py chart        # yardstick.csv + yardstick.png
    py -3 -B tools/p-yard/score.py sheets       # quarry and bench crop sheets

Error is U0b's protocol (scripts/m7-u0b-score.py): the frozen F0 transform (x/(1+x), sRGB,
8-bit codes) at -14 stops (L0) and +4 stops (L4) on the pre-tonemap HDR each engine hands
its tone mapper (Unreal: the Tonemap pass input after TSR; Renderlab: the HDR target after
TR1), against the unexposed reference mean. "Full frame" is every covered pixel
(`all-covered`); `image` adds the black background. Noise-subtracted RMS removes the
reference's own split-half noise. Flicker is the frame-to-frame step minus the reference's
step on consecutive moving frames (U0b's residual).
"""
import csv
import functools
import hashlib
import json
from pathlib import Path
import sys

import numpy as np
from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
WORK = Path("D:/Coding/renderlab-work/p-yard2")
REFS = Path("E:/renderlab-work/m7-u0b/reference/quarry-final")
MANIFEST = json.loads((HERE / "manifest.json").read_text())
POPS = ("all-covered", "image", "edge", "silhouette", "occluded", "contact", "lit", "solid")
STATES = ("still", "walk", "hold", "return", "cut", "recover")
LABELS = {"ue-low": "UE Low", "ue-med": "UE Medium", "ue-sw-h": "UE SW Lumen High", "ue-sw-e": "UE SW Lumen Epic",
          "ue-sw-c": "UE SW Lumen Cinematic", "ue-hw-h": "UE HW Lumen High", "ue-hw-e": "UE HW Lumen Epic",
          "ue-hw-c": "UE HW Lumen Cinematic", "rl-rt": "Renderlab RT", "rl-nonrt": "Renderlab non-RT"}
PATH = {"ue-low": "non-RT", "ue-med": "non-RT", "ue-sw-h": "non-RT", "ue-sw-e": "non-RT", "ue-sw-c": "non-RT",
        "ue-hw-h": "RT", "ue-hw-e": "RT", "ue-hw-c": "RT", "rl-rt": "RT", "rl-nonrt": "non-RT"}


def save(path, value):
    path.write_text(json.dumps(value, indent=1, allow_nan=False) + "\n")


def sha(path):
    with Path(path).open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


def codes(x, stops):
    x = np.maximum(x * np.float32(2.0 ** stops), 0)
    x = x / (1 + x)
    return 255 * np.where(x <= .0031308, 12.92 * x, 1.055 * np.power(x, 1 / 2.4) - .055)


@functools.lru_cache(maxsize=3)
def reference(name):
    meta = json.loads((REFS / f"{name}.json").read_text())
    path = REFS / f"{name}.npz"
    if meta.get("camera_background") != [0, 0, 0] or meta.get("accepted_for_scoring") is False:
        raise ValueError("unverified reference " + name)
    with np.load(path) as r:
        halves = [r["L0"], r["L1"]]
        pops = {k[4:]: r[k] for k in r.files if k[4:] in POPS}
        hit = r["hit"][..., 0]
    mean = (halves[0] + halves[1]) / 2
    stops = MANIFEST["scenes"]["quarry"]["exposure_stops"]
    display = {o: (codes(mean, stops + o), [codes(h, stops + o) for h in halves]) for o in (0, 4)}
    return dict(display=display, pops=pops, hit=hit, samples=meta["samples"], sha256=meta["archive_sha256"])


def candidates(scene="quarry"):
    """{config: {frame index: loader}} for every finished capture of `scene`."""
    frames = MANIFEST["scenes"][scene]["frames"]
    found = {}
    ue = WORK / "ue" / scene / "capture"
    for case in sorted(ue.iterdir()) if ue.exists() else []:
        if not (case / "native.json").exists():
            continue
        native = json.loads((case / "native.json").read_text())["frames"]
        first = MANIFEST["scenes"][scene]["ue_dump_first"]
        indices = list(range(first, len(frames))) if scene == "quarry" else [r["index"] for r in native]
        if len(native) != len(indices):
            raise ValueError(f"{case}: {len(native)} native frames for {len(indices)} dumped")
        def loader(row):
            def load():
                with np.load(row["archive"]) as c:
                    return c["hdr"][..., :3].astype(np.float32) / np.float32(c["pre_exposure"])
            return load
        found[case.name] = {i: loader(row) for i, row in zip(indices, native) if Path(row["archive"]).exists()}
    rl = WORK / "rl" / scene / "capture"
    for case in sorted(rl.iterdir()) if rl.exists() else []:
        engine = case / "engine.json"
        if not engine.exists():
            continue
        info = json.loads(engine.read_text())["hdr"]
        if info["format"] != "Rgba16Float":
            raise ValueError("unreviewed Renderlab HDR format " + info["format"])
        w, h, pre = info["width"], info["height"], info["pre_exposure"]
        def loader(path, w=w, h=h, pre=pre):
            def load():
                raw = np.fromfile(path, "<f2")
                row = raw.size // h  # rows are padded to 256 bytes
                return raw.reshape(h, row)[:, :w * 4].reshape(h, w, 4)[..., :3].astype(np.float32) / np.float32(pre)
            return load
        found["rl-" + case.name] = {int(p.stem[4:]): loader(p) for p in case.glob("hdr-*.bin")}
    return found


def measures(error, noise, pops):
    e2 = (error ** 2).mean(-1)
    v2 = (noise ** 2).mean(-1)
    out = {}
    for name, mask in pops.items():
        if mask.any():
            out[name] = dict(n=int(mask.sum()), ms=float(e2[mask].mean()), noise_ms=float(v2[mask].mean()),
                             p99=float(np.sqrt(np.quantile(e2[mask], .99))), signed=float(error[mask].mean()))
    return out


def quality(_):
    frames = MANIFEST["scenes"]["quarry"]["frames"]
    results = {}
    for config, loaders in candidates().items():
        rows, previous = [], None
        for i in sorted(loaders):
            f = frames[i]
            if not f["reference"]:
                previous = None
                continue
            ref = reference(f["reference"])
            hdr = loaders[i]()
            if hdr.shape != ref["display"][0][0].shape or not np.isfinite(hdr).all():
                raise ValueError(f"{config} frame {i}: invalid HDR")
            row = dict(index=i, state=f["state"], reference=f["reference"], samples=ref["samples"], measures={}, flicker={})
            display = {}
            for o in (0, 4):
                mean, split = ref["display"][o]
                est = codes(hdr, MANIFEST["scenes"]["quarry"]["exposure_stops"] + o)
                display[o] = est
                row["measures"][str(o)] = measures(est - mean, (split[0] - split[1]) / 2, ref["pops"])
                if previous and previous["index"] == i - 1 and f["state"] in ("walk", "return"):
                    old = previous["display"][o]
                    pref = reference(previous["reference"])
                    common = ref["pops"]["all-covered"] & pref["pops"]["all-covered"] & (ref["hit"] == pref["hit"])
                    residual = est - old - (mean - pref["display"][o][0])
                    noise = ((split[0] - pref["display"][o][1][0]) - (split[1] - pref["display"][o][1][1])) / 2
                    row["flicker"][str(o)] = measures(residual, noise, {"all-covered": common})
            previous = dict(index=i, reference=f["reference"], display=display)
            rows.append(row)
        results[config] = rows
        print("scored", config, len(rows), flush=True)
    summary = {}
    for config, rows in results.items():
        summary[config] = {}
        for state in STATES + ("moving",):
            pick = [r for r in rows if r["state"] == state or (state == "moving" and r["state"] in ("walk", "return"))]
            if not pick:
                continue
            s = {}
            for o in ("0", "4"):
                for pop in POPS:
                    cells = [r["measures"][o][pop] for r in pick if pop in r["measures"][o]]
                    n = sum(c["n"] for c in cells)
                    ms = sum(c["ms"] * c["n"] for c in cells) / n
                    noise = sum(c["noise_ms"] * c["n"] for c in cells) / n
                    s[f"{pop}@L{o}"] = dict(rms=ms ** .5, rms_noise_subtracted=max(0, ms - noise) ** .5, frames=len(cells))
                flick = [r["flicker"][o]["all-covered"] for r in pick if o in r["flicker"]]
                if flick:
                    n = sum(c["n"] for c in flick)
                    s[f"flicker@L{o}"] = dict(rms=(sum(c["ms"] * c["n"] for c in flick) / n) ** .5,
                                             rms_noise_subtracted=max(0, sum((c["ms"] - c["noise_ms"]) * c["n"] for c in flick) / n) ** .5,
                                             pairs=len(flick))
            summary[config][state] = s
    save(WORK / "quality.json", dict(protocol=__doc__, frames=results, summary=summary))


def timing(_):
    frames = MANIFEST["scenes"]["quarry"]["frames"]
    state_of = {f["index"]: f["state"] for f in frames}
    out = {}
    for root in sorted((WORK / "ue" / "quarry").glob("timing-*")):
        for case in sorted(root.iterdir()):
            if not (case / "profile.csv").exists():
                continue
            rows = list(csv.reader((case / "profile.csv").open(encoding="utf-8", errors="replace")))
            col = rows[0].index("GPUTime")
            values = {}
            for i in range(len(frames)):
                try:
                    values[i] = float(rows[1 + i][col])
                except (ValueError, IndexError):
                    pass
            out.setdefault(case.name, []).append(dict(visit=root.name, per_frame=values, method="UE CSV GPUTime (-csvGpuStats)"))
    for root in sorted((WORK / "rl" / "quarry").glob("timing-*")):
        for case in sorted(root.iterdir()):
            if not (case / "engine.json").exists():
                continue
            e = json.loads((case / "engine.json").read_text())
            span = {g["frame_id"]: g["span_ms"] for g in e["gpu"]}
            values = {r["index"]: span[r["frame_id"]] for r in e["frames"] if r["frame_id"] in span}
            out.setdefault("rl-" + case.name, []).append(dict(visit=root.name, per_frame=values,
                method="Renderlab GpuSample.render_span (GPU timestamps, first pass to present end)",
                triangles=e["frames"][0]["triangles"], adapter=e["adapter"], debug_build=e["debug_build"]))
    summary = {}
    for config, visits in out.items():
        summary[config] = {}
        for state in STATES + ("moving",):
            per_visit = []
            for v in visits:
                xs = [ms for i, ms in v["per_frame"].items()
                      if state_of.get(int(i)) == state or (state == "moving" and state_of.get(int(i)) in ("walk", "return"))]
                if xs:
                    per_visit.append(dict(visit=v["visit"], n=len(xs), p50=float(np.median(xs)), p95=float(np.quantile(xs, .95))))
            if per_visit:
                summary[config][state] = dict(visits=per_visit, p50=float(np.median([p["p50"] for p in per_visit])),
                                              p50_range=[min(p["p50"] for p in per_visit), max(p["p50"] for p in per_visit)])
    save(WORK / "timing.json", dict(visits=out, summary=summary))


def chart(_):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    q = json.loads((WORK / "quality.json").read_text())["summary"]
    t = json.loads((WORK / "timing.json").read_text())["summary"]
    rows = []
    for config in q:
        for state in ("still", "moving", "cut", "recover"):
            if state not in q[config] or state not in t.get(config, {}):
                continue
            s = q[config][state]
            rows.append(dict(config=config, label=LABELS[config], engine="Renderlab" if config.startswith("rl-") else "Unreal",
                             path=PATH[config], state=state, gpu_ms_p50=round(t[config][state]["p50"], 4),
                             gpu_ms_visit_range=" to ".join(f"{x:.3f}" for x in t[config][state]["p50_range"]),
                             rms_full_L0=round(s["all-covered@L0"]["rms"], 4), rms_full_L4=round(s["all-covered@L4"]["rms"], 4),
                             rms_full_noise_subtracted_L0=round(s["all-covered@L0"]["rms_noise_subtracted"], 4),
                             rms_image_L0=round(s["image@L0"]["rms"], 4), rms_edge_L0=round(s["edge@L0"]["rms"], 4),
                             flicker_L0=round(s["flicker@L0"]["rms"], 4) if "flicker@L0" in s else "",
                             scored_frames=s["all-covered@L0"]["frames"]))
    with (WORK / "yardstick.csv").open("w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0]))
        w.writeheader()
        w.writerows(rows)
    fig, axes = plt.subplots(1, 3, figsize=(16, 5.6), sharey=True)
    colors = {"RT": "#1f6fb4", "non-RT": "#d1782b"}
    for ax, state in zip(axes, ("still", "moving", "cut")):
        for r in [r for r in rows if r["state"] == state]:
            rl = r["engine"] == "Renderlab"
            ax.scatter(r["gpu_ms_p50"], r["rms_full_L0"], s=140 if rl else 60, marker="*" if rl else "o",
                       color=colors[r["path"]], edgecolor="black" if rl else "none", zorder=3)
            ax.annotate(r["label"].replace("UE ", "").replace("Lumen ", ""), (r["gpu_ms_p50"], r["rms_full_L0"]),
                        textcoords="offset points", xytext=(6, 4), fontsize=8)
        for r in [r for r in rows if r["state"] == state and r["config"] in ("ue-hw-e", "ue-sw-e")]:
            ax.axvline(r["gpu_ms_p50"] / 3, color=colors[r["path"]], ls=":", lw=1)
            ax.text(r["gpu_ms_p50"] / 3, .02, f" {r['label'][3:]} / 3", rotation=90, fontsize=7,
                    color=colors[r["path"]], va="bottom", transform=ax.get_xaxis_transform())
        ax.set_title({"still": "Still (p0, frame 159)", "moving": "Moving (Q-walk, walk + return)",
                      "cut": "Cut frame (history reset, 0.94 m jump)"}[state])
        ax.set_xlabel("GPU frame time p50 (ms), 1920x1080")
        ax.grid(alpha=.3)
        ax.set_xlim(left=0)
    axes[0].set_ylabel("Full-frame error vs path-traced reference\n(all covered pixels, RMS F0 codes at -14 EV)")
    fig.suptitle("P-YARD2: matched quarry, RTX 4070 SUPER, Unreal 5.7.4 (dots) vs Renderlab (stars); blue RT, orange non-RT; "
                 "dotted = Epic time / 3", fontsize=10)
    fig.tight_layout()
    fig.savefig(WORK / "yardstick.png", dpi=130)
    print(WORK / "yardstick.csv", WORK / "yardstick.png", len(rows))


def tile(rgb, box, scale):
    x, y, w, h = box
    crop = np.rint(np.clip(rgb[y:y + h, x:x + w], 0, 255)).astype("u1")
    return Image.fromarray(crop).resize((w * scale, h * scale), Image.NEAREST)


def sheet(path, columns, rows, box, scale, title):
    """rows: [(label, {column: rgb})]. Missing cells stay grey."""
    w, h = box[2] * scale, box[3] * scale
    head, side = 22, 170
    img = Image.new("RGB", (side + w * len(columns), head + (h + 4) * len(rows) + 18), (40, 40, 40))
    d = ImageDraw.Draw(img)
    d.text((4, 4), title, fill=(255, 255, 255))
    for j, c in enumerate(columns):
        d.text((side + j * w + 4, head - 12), str(c), fill=(220, 220, 220))
    for i, (label, cells) in enumerate(rows):
        y = head + i * (h + 4)
        d.text((4, y + h // 2), label, fill=(255, 255, 255))
        for j, c in enumerate(columns):
            if c in cells:
                img.paste(tile(cells[c], box, scale), (side + j * w, y))
    img.save(path)
    print(path)


def sheets(_):
    out = WORK / "sheets"
    out.mkdir(exist_ok=True)
    q = MANIFEST["scenes"]["quarry"]
    stops = q["exposure_stops"]
    order = ["rl-rt", "rl-nonrt", "ue-hw-c", "ue-hw-e", "ue-sw-c", "ue-sw-e", "ue-med", "ue-low"]
    found = candidates("quarry")
    # Quarry: still, mid-walk, the cut and its recovery, full frame (half size) and full-res crops.
    columns = [159, 192, 288, 320, 321, 324, 336, 351]
    for name, box, scale in (("full", (0, 0, 1920, 1080), 1), ("crop-a", (700, 380, 300, 200), 2),
                             ("crop-b", (1250, 620, 300, 200), 2)):
        rows = []
        refcells = {c: reference(q["frames"][c]["reference"])["display"][0][0] for c in columns if q["frames"][c]["reference"]}
        rows.append(("path-traced reference", refcells))
        for config in [c for c in order if c in found]:
            rows.append((LABELS[config], {c: codes(found[config][c](), stops) for c in columns if c in found[config]}))
        if name == "full":
            # Half-size thumbnails keep the full-frame sheet viewable.
            rows = [(l, {c: np.asarray(Image.fromarray(np.rint(np.clip(v, 0, 255)).astype("u1")).resize((960, 540), Image.BILINEAR)).astype(np.float32)
                         for c, v in cells.items()}) for l, cells in rows]
            box = (0, 0, 960, 540)
        sheet(out / f"quarry-{name}.png", columns, rows, box, scale,
              f"quarry {name}: frames 159 still, 192 walk, 288 return, 320 cut, 321/324/336/351 after the cut; F0 at -14 EV")
    for scene, columns, box in (("s4-door", [40, 52, 64, 76, 88, 96, 100, 111], (560, 180, 520, 520)),
                                ("s5-cube", [40, 64, 88, 112, 128, 132, 136, 143], (520, 160, 560, 480))):
        found = candidates(scene)
        stops = MANIFEST["scenes"][scene]["exposure_stops"]
        rows = [(LABELS[c], {k: codes(found[c][k](), stops) for k in columns if k in found[c]}) for c in order if c in found]
        if rows:
            sheet(out / f"{scene}.png", columns, rows, box, 1, f"{scene}: mover frames (door 32-96, cube 32-128), F0 at {stops:g} EV")


def identity(_):
    q = MANIFEST["scenes"]["quarry"]
    content = json.loads((REFS / "content.json").read_text())
    names = sorted({f["reference"] for f in q["frames"] if f["reference"]})
    checks = dict(asset=content["asset_sha256"] == q["inputs"]["asset_sha256"],
                  environment=content["environment_sha256"] == q["inputs"]["environment_sha256"])
    per = []
    for name in names:
        meta = json.loads((REFS / f"{name}.json").read_text())
        c = meta["content"]
        per.append(dict(name=name, samples=meta["samples"], archive_ok=sha(REFS / f"{name}.npz") == meta["archive_sha256"],
                        asset_ok=c["asset_sha256"] == q["inputs"]["asset_sha256"],
                        environment_ok=c["environment_sha256"] == q["inputs"]["environment_sha256"],
                        triangles=c["triangles"], black_primary=meta.get("camera_background") == [0, 0, 0]))
    t = json.loads((WORK / "timing.json").read_text())["visits"] if (WORK / "timing.json").exists() else {}
    loaded = {k: v[0].get("triangles") for k, v in t.items() if k.startswith("rl-")}
    result = dict(inputs=checks, references=per, renderlab_loaded_triangles=loaded,
                  all_ok=all(checks.values()) and all(p["archive_ok"] and p["asset_ok"] and p["environment_ok"] and p["black_primary"] for p in per)
                  and all(v == content["triangles"] for v in loaded.values()))
    save(WORK / "reference-identity.json", result)
    print(json.dumps(dict(all_ok=result["all_ok"], references=len(per), inputs=checks, loaded=loaded)))


if __name__ == "__main__":
    globals()[sys.argv[1]](sys.argv[2:])
