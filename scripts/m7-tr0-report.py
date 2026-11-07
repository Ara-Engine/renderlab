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

checkpoint_frames = [r["frame"] for r in quarry["w-e-n-c2"]["rows"]
                     if "t1b_replay_same_physical_reference" in r["measures"]["L0"]["all-covered"]]
assert len(set(checkpoint_frames)) == 60
figure, axes = plt.subplots(1, 2, figsize=(11, 4.7), layout="constrained")
for axis, channel in zip(axes, ("L0", "L4"), strict=True):
    for name, method, label, colour in methods:
        indexed = {r["frame"]: r for r in quarry[name]["rows"]}
        values = []
        for frame in checkpoint_frames:
            value = indexed[frame]["measures"][channel]["all-covered"]
            if method == "t1b":
                value = value["t1b_replay_same_physical_reference"]
            values.append(value["rms_noise_subtracted"])
        axis.plot(checkpoint_frames, values, color=colour, label=label, marker=".", markersize=4, linewidth=.8)
    axis.axvspan(64, 96, color="#888888", alpha=.12)
    axis.set_xticks((0, 32, 64, 96, 128, 160))
    axis.set_title(channel)
    axis.set_xlabel("Registered Q-walk frame")
    axis.set_ylabel("All-covered reference-noise-subtracted RMS codes")
    axis.grid(alpha=.2)
axes[0].legend(fontsize=8)
figure.suptitle("Q-walk: all 60 original T1b checkpoints on the new physical target", fontsize=11)
figure.supxlabel("Each marker is a measured checkpoint; joining lines do not fill unscored frames. Grey: camera hold.\nCurrent covered population, no age restriction, shared between methods at each frame. Combined final-colour error; not the frozen Exact bar.", fontsize=8)
save(figure, "quarry-checkpoint-error-curves")

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

quarter_paths = [ROOT / "runs" / label / "physical-scores.json" for label in ("g25-e-n-s1", "g25-e-r-s1")]
if all(path.exists() for path in quarter_paths):
    quarter = [(label, read("runs/" + label + "/physical-scores.json")) for label in ("g25-e-n-s1", "g25-e-r-s1")]
    assert all(len(score["rows"]) == 160 and not score["missing_frames"] for _, score in quarter)
    figure, axes = plt.subplots(2, 2, figsize=(11, 7), layout="constrained")
    for index, (_, score) in enumerate(quarter):
        label, colour = ("No RT", "#285e9b") if index == 0 else ("HRT", "#b16c1f")
        frames = [r["frame"] for r in score["rows"]]
        axes[0, 0].plot(frames, [r["measures"]["L0"]["all-covered"]["rms_noise_subtracted"] for r in score["rows"]], label=label, color=colour)
        axes[0, 1].plot(frames, [r["highlights"]["metal-0.1"]["energy_ratio"] for r in score["rows"]], label=label, color=colour)
        flicker = [r for r in score["motion_residuals"] if r["channel"] == "L0" and r["tsr_rms"] is not None]
        axes[1, 0].plot([r["frames"][1] for r in flicker], [r["tsr_rms"] for r in flicker], label=label, color=colour)
        for population, style in (("all-covered", "-"), ("background-centre", "--")):
            pools = [score["transient_pools"][f"revealed/{age}/{population}/L0"] for age in range(8)]
            axes[1, 1].plot(range(8), [p["tsr_rms_noise_subtracted"] for p in pools], label=label + " " + population,
                            color=colour, linestyle=style, marker="o", markersize=3)
    for axis in (axes[0, 0], axes[0, 1], axes[1, 0]):
        axis.axvspan(64, 95, color="#888888", alpha=.12)
        axis.set_xlabel("Registered input frame")
    axes[0, 0].set_title("Current covered population: L0 RMS")
    axes[0, 1].set_title("Metal r=0.1: highlight energy / physical target")
    axes[0, 1].axhline(1, color="#555555", linestyle=":", linewidth=1)
    axes[1, 0].set_title("Reference-subtracted L0 flicker, same instance / age ≥8")
    axes[1, 1].set_title("Revealed age pools: L0 RMS")
    axes[1, 1].set_xlabel("Shared geometry age")
    axes[1, 1].set_xticks(range(8))
    axes[0, 0].legend(fontsize=8)
    axes[1, 1].legend(fontsize=8)
    for axis in axes.flat:
        axis.set_ylim(bottom=0)
        axis.grid(alpha=.2)
    figure.suptitle("S2G quarter-pixel camera track — 160 frames on each Epic TSR path", fontsize=12)
    figure.supxlabel("D8 physical target, black primary, zero shutter; combined lighting/shading and reconstruction error. Grey: camera hold.\nEnergy masks are each frame's reference top 5%. Age pools contain different pixels; they are not one event's settle trajectory.", fontsize=8)
    save(figure, "s2g-quarter-comparison")

matrix = read("s2g/synchronized-candidates.json")
summary_cases = []
for case in matrix:
    score_path = ROOT / "runs" / case["label"] / "physical-scores.json"
    if not score_path.exists():
        summary_cases.append(dict(case, complete=False, measures={}))
        continue
    score = read("runs/" + case["label"] + "/physical-scores.json")
    complete = {r["frame"] for r in score["rows"]} == set(range(160)) and not score["missing_frames"]
    if not complete:
        summary_cases.append(dict(case, complete=False, measures={}))
        continue
    recipe = read("runs/" + case["label"] + "/recipe.json")
    rows_by_frame = {r["frame"]: r for r in score["rows"]}
    measures = {}

    def metric(name, values):
        measures[name] = values

    def quantiles(values):
        return np.quantile(values, [.5, .95]).tolist() if values else None

    for channel in ("L0", "L4"):
        metric(f"All-frame covered {channel} RMS, p50 / p95", quantiles([
            r["measures"][channel]["all-covered"]["rms_noise_subtracted"] for r in score["rows"]]))
    for population in ("all-covered", "metal-0.1", "dielectric-0.2", "edge", "background-centre"):
        metric(f"Frame 64 {population} L0 / L4 RMS", [rows_by_frame[64]["measures"][c][population]["rms_noise_subtracted"]
                                                          for c in ("L0", "L4")])
    for population, ages in (("all-covered", (3, 7)), ("background-centre", (7,))):
        pools = [score["transient_pools"].get(f"revealed/{age}/{population}/L0") for age in ages]
        metric(f"Revealed {population} L0, age " + " / ".join(map(str, ages)),
               [p["tsr_rms_noise_subtracted"] for p in pools] if all(pools) else None)
    changed = rows_by_frame[72].get("changed", {}).get("measures")
    if changed:
        metric("Frame 72 eight-frame changed-mask L0 / L4 RMS", [changed[c]["tsr_rms_noise_subtracted"] for c in ("L0", "L4")])
        metric("Frame 72 changed-mask L0 signed mean / SE", [changed["L0"]["tsr_signed_mean"], changed["L0"]["tsr_se"]])
    for channel in ("L0", "L4"):
        moving = [r["tsr_rms"] for r in score["motion_residuals"] if r["channel"] == channel and r["tsr_rms"] is not None
                  and (not r["static"] or case["track"] == "s2g-sun-0.25deg")]
        metric(f"Input-changing F6 {channel} residual, p50 / p95", quantiles(moving))
    is_camera = case["track"].startswith("s2g-camera-")
    if is_camera:
        steps = [next(r for r in score["motion_residuals"] if r["frames"] == [80, 81] and r["channel"] == c)
                 for c in ("L0", "L4")]
        metric("Hold 80 to 81 age-at-least-8 L0 / L4 step", [r["tsr_step_rms"] for r in steps])
        diagnostic = read("runs/" + case["label"] + "/hold-diagnostic.json")
        metric("Full-image self-final settle proxy, frames", diagnostic["settle_offset"])
    for frame in (64, 95 if is_camera else 159):
        metric(f"Frame {frame} metal / dielectric highlight energy ratio", [rows_by_frame[frame]["highlights"][p]["energy_ratio"]
                                                                              for p in ("metal-0.1", "dielectric-0.2")])
        for population in ("metal-0.1", "dielectric-0.2"):
            metric(f"Frame {frame} {population} centroid / peak drift, pixels", [rows_by_frame[frame]["highlights"][population][k]
                                                                                   for k in ("centroid_drift_px", "peak_drift_px")])
    for channel in ("L0", "L4"):
        extent = rows_by_frame[64]["background_error_extent"][channel]["values"]["tsr"]
        metric(f"Frame 64 age-at-least-32 background error pixels over 1 / 4 / 8 {channel} codes", [extent[t]["pixels"] for t in ("1", "4", "8")])
    summary_cases.append(dict(case, complete=True, measures=measures,
                              score_sha256=sources[str(score_path)], age_protocol=score["age_protocol"]))

summary = dict(cases=summary_cases, complete=all(c["complete"] for c in summary_cases),
               script_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
               scope="Combined final-colour physical error, not a decomposition of TSR error. D8, black primary, zero shutter, registered exposure. Missing entries are unavailable, never passes.",
               limitations=["Original post-TSR R_F and R_D estimands unavailable", "Film-settle gate unavailable; self-final proxy is diagnostic only",
                            "Sun turns continuously: no camera disocclusion or stop/hold event", "Centroid/peak drifts are fixed-mask diagnostics, not connected trail widths"])
(OUT / "s2g-score-summary.json").write_text(json.dumps(summary, indent=2, allow_nan=False) + "\n")
table = ["# S2G retained physical-score tables", "", summary["scope"], ""]
for track in ("s2g-camera-0.25px", "s2g-camera-0.5px", "s2g-camera-1px", "s2g-sun-0.25deg"):
    pair = [next(c for c in summary_cases if c["track"] == track and c["path"] == p) for p in ("vsm-nort", "hrt")]
    table += ["## " + track, "", "| Measure | Epic no RT | Epic HRT |", "|---|---:|---:|"]
    names = dict.fromkeys(name for c in pair for name in c["measures"])
    if not names:
        table.append("| Complete registered physical score | Unavailable | Unavailable |")
    for name in names:
        values = []
        for case in pair:
            value = case["measures"].get(name)
            values.append("Unavailable" if value is None else " / ".join(f"{v:.6g}" for v in value) if isinstance(value, list) else f"{value:.6g}")
        table.append("| " + name + " | " + " | ".join(values) + " |")
    table.append("")
table += ["RMS subtracts split-half reference noise. F6 subtracts consecutive physical-reference changes, with shared age >=8.",
          "Camera tracks stop at frame 64; Sun changes continuously. Revealed-age pools contain different pixels at each age.",
          "Self-final settle compares against the last held image, not an independent film bar. Original film-settle counts remain unavailable.", ""]
(OUT / "s2g-score-summary.md").write_text("\n".join(table), encoding="utf-8")
if all((ROOT / "runs" / c["label"] / "physical-scores.json").exists() for c in matrix):
    glossy = [(c, read("runs/" + c["label"] + "/physical-scores.json")) for c in matrix]
    if all(len(s["rows"]) == 160 and not s["missing_frames"] for _, s in glossy):
        tracks = ("s2g-camera-0.25px", "s2g-camera-0.5px", "s2g-camera-1px", "s2g-sun-0.25deg")
        for kind in ("error", "highlight-energy"):
            figure, axes = plt.subplots(2, 2, figsize=(11, 7), layout="constrained")
            for axis, track in zip(axes.flat, tracks, strict=True):
                for case, score in glossy:
                    if case["track"] != track:
                        continue
                    label, colour = ("No RT", "#285e9b") if case["path"] == "vsm-nort" else ("HRT", "#b16c1f")
                    frames = [r["frame"] for r in score["rows"]]
                    if kind == "error":
                        axis.plot(frames, [r["measures"]["L0"]["all-covered"]["rms_noise_subtracted"] for r in score["rows"]],
                                  label=label, color=colour)
                    else:
                        for population, style in (("metal-0.1", "-"), ("dielectric-0.2", "--")):
                            axis.plot(frames, [r["highlights"][population]["energy_ratio"] for r in score["rows"]],
                                      label=label + " " + population, color=colour, linestyle=style)
                if track != "s2g-sun-0.25deg":
                    axis.axvspan(64, 95, color="#888888", alpha=.12)
                if kind == "highlight-energy":
                    axis.axhline(1, color="#555555", linestyle=":", linewidth=1)
                axis.set_title(track.removeprefix("s2g-"))
                axis.set_xlabel("Registered input frame")
                axis.set_ylabel("All-covered L0 RMS codes" if kind == "error" else "Energy / physical full-radiance target")
                axis.set_ylim(bottom=0)
                axis.grid(alpha=.2)
            axes.flat[0].legend(fontsize=8)
            figure.suptitle("S2G Epic TSR — all 640 registered frames on both paths", fontsize=12)
            figure.supxlabel("D8-qualified physical references, black primary, zero shutter, no fitting. Grey: camera hold; sun turns continuously.\nErrors include lighting/shading and reconstruction. Highlight masks are each reference frame's top 5% within its current material.", fontsize=8)
            save(figure, "s2g-" + kind + "-curves")

(OUT / "figure-provenance.json").write_text(json.dumps(dict(sources=sources,
    script_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest()), indent=2) + "\n")
print("Scientific PNG/SVG figures retained", OUT)
