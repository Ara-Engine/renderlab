"""CPU-only scientific figures from TR0's retained measurement summaries."""
import hashlib
import json
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np

ROOT = Path("E:/renderlab-work/m7-tr0")
OUT = ROOT / "report"
OUT.mkdir(exist_ok=True)
sources = {}


def read(path):
    path = ROOT / path
    sources[str(path)] = hashlib.sha256(path.read_bytes()).hexdigest()
    return json.loads(path.read_text())


def save(figure, name):
    figure.savefig(OUT / (name + ".png"), dpi=180)
    figure.savefig(OUT / (name + ".svg"))
    plt.close(figure)


rows = read("cost-summary.json")["rows"]
cases = [(track, path) for track in ("q-walk", "s2g-camera-1px") for path in ("vsm-nort", "hrt")]
names = ["Quarry\nno RT", "Quarry\nHRT", "S2G\nno RT", "S2G\nHRT"]
figure, axes = plt.subplots(1, 2, figsize=(11, 4.6), layout="constrained")
for axis, field, title in zip(axes, ("frame_increment_ms", "tsr_gpu_ms"),
                              ("Paired end-to-end TSR increment", "Whole GPU frame with TSR"), strict=True):
    for quality, offset, colour, label in ((3, -.18, "#285e9b", "Epic"), (2, .18, "#b16c1f", "High")):
        selected = [next(r for r in rows if (r["track"], r["path"]) == case and r["quality"] == quality and r["scope"] == "quiet") for case in cases]
        assert all(r["pairs"] == 8 for r in selected)
        medians = np.array([r["stats"][field]["p50"] for r in selected])
        tails = np.array([r["stats"][field]["p95"] for r in selected])
        axis.bar(np.arange(4) + offset, medians, .32, color=colour, label=label,
                 yerr=np.stack((np.zeros(4), tails - medians)), capsize=3)
        if quality == 3:
            axis.plot(np.arange(4), medians * .5, "_", color="#2a7942", markersize=24,
                      markeredgewidth=2, label="TR1 half-Epic ceiling")
    axis.set_xticks(range(4), names)
    axis.set_ylim(bottom=0)
    axis.set_ylabel("Milliseconds")
    axis.set_title(title)
    axis.grid(axis="y", alpha=.2)
    axis.set_axisbelow(True)
axes[0].legend(fontsize=8)
figure.suptitle("UE 5.7.4 TSR at 100%, 1920×1080 — quiet paired full-track measurements", fontsize=11)
figure.supxlabel("Bars: pooled p50; whiskers: p95. Eight pairs/configuration. Direct/environment subset, no Lumen.\nGPUTime reporting lag is uncalibrated per pose; retained outlier is included. See m7-tr0.md.", fontsize=8)
save(figure, "quiet-cost-curve")

controls = [read("runs/" + name + "/colour-control-readback.json") for name in
            ("gc-sun-aa0-a", "gc-sun-tsr-a", "gc-sun-tsr-m4-a")]
figure, axis = plt.subplots(figsize=(9, 4.7), layout="constrained")
for index, (population, colour, label) in enumerate((("metal-0.1", "#285e9b", "Metal r=0.1"),
                                                    ("dielectric-0.2", "#b16c1f", "Dielectric r=0.2"))):
    values = [controls[0]["rows"][population]["tonemap_input"]["highlight_energy_ratio"],
              controls[1]["rows"][population]["reject_input"]["highlight_energy_ratio"],
              controls[1]["rows"][population]["tonemap_input"]["highlight_energy_ratio"],
              controls[2]["rows"][population]["tonemap_input"]["highlight_energy_ratio"]]
    positions = np.arange(4) + (index - .5) * .34
    axis.bar(positions, values, .31, color=colour, label=label)
    for x, value in zip(positions, values, strict=True):
        axis.text(x, value + .02, f"{value:.3f}", ha="center", fontsize=9)
axis.axhline(1, color="#555555", linestyle="--", linewidth=1)
axis.set_ylim(0, 1.12)
axis.set_xticks(range(4), ["No AA", "TSR RejectShading\nactual input", "TSR output\nregistered −12", "TSR output\n−4-stop diagnostic"])
axis.set_ylabel("Linear energy / physical-direct reference")
axis.set_title("S2G static f64 sun-only control: fixed physical-D highlight masks")
axis.legend(loc="lower left")
axis.grid(axis="y", alpha=.2)
axis.set_axisbelow(True)
figure.supxlabel("Diagnostic HDR is unexposed using measured PE, then evaluated at registered −12. No fitted gain.\nThe entire pre-TSR buffer matches byte for byte after ×16 RGB conversion. BF-qualified f64 target; D8 corpus is separate.", fontsize=8)
save(figure, "s2g-exposure-control")

quarry = {name: read("runs/" + name + "/physical-scores.json") for name in
          ("w-e-n-c2", "w-e-r-c1", "w-h-v-c1")}
methods = [("w-e-n-c2", "t1b", "T1b black", "#606060"),
           ("w-e-n-c2", "tsr", "Epic no RT", "#285e9b"),
           ("w-e-r-c1", "tsr", "Epic HRT", "#368165"),
           ("w-h-v-c1", "tsr", "High VSM", "#b16c1f")]
populations = ("all-covered", "partial", "occluded", "edge", "background-centre")
figure, axes = plt.subplots(1, 2, figsize=(11, 4.6), layout="constrained")
for axis, channel in zip(axes, ("L0", "L4"), strict=True):
    for index, (name, method, label, colour) in enumerate(methods):
        row = next(r for r in quarry[name]["rows"] if r["frame"] == 64)
        values = [row["measures"][channel][p]["same_t1b_age32_pixels"][method] for p in populations]
        axis.bar(np.arange(len(populations)) + (index - 1.5) * .19, values, .18, color=colour, label=label)
    axis.set_xticks(range(len(populations)), ["Covered", "Partial", "Occluded", "Edge", "Black\nbackground"])
    axis.set_ylabel("Reference-noise-subtracted RMS codes")
    axis.set_title(channel)
    axis.grid(axis="y", alpha=.2)
    axis.set_axisbelow(True)
axes[0].legend(fontsize=8)
figure.suptitle("Q-walk frame 64 — identical T1b age ≥32 pixels, new physical target", fontsize=11)
figure.supxlabel("Combined final-colour error includes material, lighting and reconstruction differences.\nThis is not the historical frozen Exact bar. High VSM keeps the RT-capable device configuration.", fontsize=8)
save(figure, "quarry-population-comparison")

figure, axes = plt.subplots(1, 2, figsize=(11, 4.5), layout="constrained")
for axis, channel in zip(axes, ("L0", "L4"), strict=True):
    for name, method, label, colour in methods:
        rows = [r for r in quarry[name]["rows"] if 64 <= r["frame"] <= 96]
        assert len(rows) == 33
        if method == "t1b":
            rows = [r for r in rows if "t1b_replay_same_physical_reference" in r["measures"][channel]["all-covered"]]
            values = [r["measures"][channel]["all-covered"]["t1b_replay_same_physical_reference"]["rms_noise_subtracted"] for r in rows]
            axis.scatter([r["frame"] - 64 for r in rows], values, label=label + " checkpoints", color=colour, s=16)
        else:
            values = [r["measures"][channel]["all-covered"]["rms_noise_subtracted"] for r in rows]
            axis.plot([r["frame"] - 64 for r in rows], values, label=label, color=colour)
    axis.axvline(8, color="#555555", linestyle=":", linewidth=1)
    axis.set_title(channel)
    axis.set_xlabel("Frames after camera stops (frame 64)")
    axis.set_ylabel("Reference-noise-subtracted RMS codes")
    axis.grid(alpha=.2)
axes[0].legend(fontsize=8)
figure.suptitle("Q-walk hold — fixed current all-covered population, 33 native measurements", fontsize=11)
figure.supxlabel("The dotted +8 witness is an event time, not a pass threshold. No inferred original film-settle count.\nNative TSR has one preparation sample after reset; the physical target is independent of renderer jitter phase.", fontsize=8)
save(figure, "quarry-hold-error-curves")

figure, axes = plt.subplots(1, 2, figsize=(10.5, 4.6), layout="constrained")
for axis, population, title in zip(axes, ("all-covered", "background-centre"),
                                    ("Revealed covered pixels", "Revealed black-background pixels"), strict=True):
    for name, method, label, colour in methods:
        pools = quarry[name]["transient_pools"]
        selected = [(age, pools[f"revealed/{age}/{population}/L0"]) for age in range(8)
                    if f"revealed/{age}/{population}/L0" in pools]
        axis.plot([age for age, _ in selected], [v[method + "_rms_noise_subtracted"] for _, v in selected],
                  color=colour, label=label, marker="o", markersize=3)
    axis.set_xticks(range(8))
    axis.set_xlabel("Shared geometry age")
    axis.set_ylabel("L0 reference-noise-subtracted RMS codes")
    axis.set_title(title)
    axis.grid(alpha=.2)
axes[0].legend(fontsize=8)
figure.suptitle("Q-walk disocclusion — fixed age pools across all 60 original checkpoints", fontsize=11)
figure.supxlabel("Different pixels contribute at each age; these pools are not one event's settle trajectory.\nNew black-primary physical target. At age 7: 729 covered and 3,738 background-centre pixels.", fontsize=8)
save(figure, "quarry-revealed-age-curves")

(OUT / "figure-provenance.json").write_text(json.dumps(dict(sources=sources,
    script_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest()), indent=2) + "\n")
print("Scientific PNG/SVG figures retained", OUT)
