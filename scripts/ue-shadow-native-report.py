"""Consolidate frozen native quarry scores, readbacks and fresh packaged timing."""
import argparse
import csv
import json
from pathlib import Path
import re

import numpy as np

from importlib.util import module_from_spec, spec_from_file_location

ROOT = Path("D:/Coding/renderlab-work/ue-m5/Quarry")
CONFIGS = [("vsm-native", "vsm-baseline"), ("vsm-screen-zero", "vsm-screen-zero"),
           ("vsm-normal-zero", "vsm-normal-zero"), ("hrt-256", "hrt-256"),
           ("hrt-native", "hrt-denoised"), ("hrt-epsilon-zero", "hrt-epsilon-zero")]


def load(name):
    spec = spec_from_file_location(name.replace("-", "_"), Path(__file__).with_name(name + ".py"))
    module = module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def summarize(output):
    decoder, profiler = load("ue-shadow-dump"), load("ue-quarry-profile")
    output.mkdir(parents=True, exist_ok=False)
    cases, timings, controls, launches, epsilon = [], [], {}, {}, []
    populations = []
    for config, short in CONFIGS:
        for pose in range(3):
            version = "02" if pose == 0 and config in ("vsm-native", "hrt-256", "hrt-native") else "01"
            label = f"native-{short}-p{pose}-{version}"
            directory = ROOT / "native-shadow" / label
            case = json.loads((directory / "decoded-v2/scores.json").read_text())
            case["rgb_crosscheck"] = json.loads((directory / "decoded-v2/rgb-crosscheck.json").read_text())
            case["readbacks"] = json.loads((directory / "decoded-v2/readbacks.json").read_text())
            case["evidence_directory"] = str(directory)
            cases.append(case)
            for name, values in case["populations"].items():
                populations.append(dict(config=config, pose=pose, population=name,
                                        reference_n=values["reference_n"], evaluated_n=values["evaluated_n"], missing=values["missing"],
                                        **values["candidate"]))
            native = decoder.Dump(directory)
            all_controls = {row["CVar"]: row for row in csv.DictReader((native.root / "Base/ConsoleVariables.csv").open())}
            named = set(json.loads((directory / "decoded-v2/effective-cvars.json").read_text()))
            named.update(key for key in all_controls if key.startswith(("r.Shadow.Virtual.SMRT.", "r.Shadow.Virtual.NormalBias", "r.Shadow.Virtual.ScreenRayLength")))
            named.update(("r.Shadow.Denoiser.TemporalAccumulation", "r.Shadow.Virtual.ResolutionLodBiasDirectional",
                          "r.Shadow.Virtual.SMRT.RayCountDirectional", "r.Shadow.Virtual.SMRT.SamplesPerRayDirectional",
                          "r.Shadow.Virtual.SMRT.AdaptiveRayCount", "r.Shadow.Virtual.Enable", "r.RayTracing.Shadows",
                          "r.Shadow.Virtual.MaxPhysicalPages", "r.Shadow.Virtual.Cache", "r.Shadow.CacheWPOPrimitives",
                          "r.ContactShadows", "r.Shadow.Virtual.OnePassProjection.SkipScreenShadowMask"))
            controls[label] = {key: all_controls.get(key) for key in sorted(named)}
            decoder.write(directory / "decoded-v2/confirmed-controls.json", controls[label])
        label = f"fresh-time-{config}-p0-01"
        launch = ROOT / "launches" / label
        csv_path, = launch.glob("*.csv")
        timing = profiler.summarize(csv_path)
        timing.update(configuration=config, label=label, process=json.loads((launch / "process.json").read_text()))
        if not timing["log_evidence"]["gpu_tree"]:
            raise ValueError("missing native GPU tree: " + label)
        if config.startswith("hrt") and any(d["native_nanite_quarry_triangles"] != 13034903 for d in timing["log_evidence"]["ray_tracing_dumps"]):
            raise ValueError("native RT geometry count differs")
        timings.append(timing)
    for pose in range(3):
        version = "02" if pose == 0 else "01"
        a = np.load(ROOT / f"native-shadow/native-hrt-256-p{pose}-{version}/decoded-v2/native.npz")["visibility"]
        b = np.load(ROOT / f"native-shadow/native-hrt-epsilon-zero-p{pose}-01/decoded-v2/native.npz")["visibility"]
        epsilon.append(dict(pose=pose, bit_identical=bool(np.array_equal(a, b)), maximum=float(np.abs(a - b).max())))
    for launch in sorted((ROOT / "launches").iterdir()):
        if re.fullmatch(r"(native|fresh-time)-.+", launch.name) and (launch / "process.json").is_file():
            value = json.loads((launch / "process.json").read_text())
            before = json.loads((launch / "preflight.json").read_text(encoding="utf-8-sig"))
            launches[launch.name] = dict(start_utc=value["start_utc"], end_utc=value["end_utc"],
                elapsed_seconds=value["elapsed_seconds"], exit_code=value["exit_code"],
                same_foreground=value.get("same_foreground_before_after"), free_ram_gib=before["memory"]["FreePhysicalMemory"] / 1024 ** 2,
                game_or_renderer_samples=[s for s in value["gpu_samples"] if s["games"] or s["foreign_renderers"]],
                executable_sha256=value["executable_sha256"])
    sources = ["Source/Runtime/RenderCore/Private/DumpGPU.cpp", "Source/Runtime/Engine/Private/UnrealEngine.cpp",
               "Source/Runtime/Engine/Private/Components/DirectionalLightComponent.cpp",
               "Source/Runtime/Renderer/Private/LightRendering.cpp", "Source/Runtime/Renderer/Private/RayTracing/RayTracingShadows.cpp",
               "Shaders/Private/Common.ush", "Shaders/Private/DeferredShadingCommon.ush", "Shaders/Private/DeferredLightingCommon.ush",
               "Shaders/Private/VirtualShadowMaps/VirtualShadowMapProjectionComposite.usf",
               "Shaders/Private/ScreenSpaceDenoise/SSDSignalBufferEncoding.ush", "Shaders/Private/ScreenSpaceDenoise/SSDTemporalAccumulation.usf"]
    engine = Path("C:/Program Files/Epic Games/UE_5.7/Engine")
    decoder.write(output / "engine-source-hashes.json", {name: decoder.digest(engine / name) for name in sources})
    decoder.write(output / "summary.json", dict(cases=cases, timings=timings, controls=controls,
                   epsilon_isolation=epsilon, launches=launches,
                   launch_process_seconds=sum(v["elapsed_seconds"] for v in launches.values()),
                   release=json.loads((ROOT.parent / "gpu-released.json").read_text()),
                   reference_sha256={str(pose): decoder.digest(decoder.REFERENCE / f"pose-{pose}-reference.npz") for pose in range(3)},
                   source_asset_sha256=json.loads((ROOT / "manifest.json").read_text())["inputs"]["asset"]["sha256"]))
    with (output / "population-scores.csv").open("w", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=list(populations[0])); writer.writeheader(); writer.writerows(populations)
    print("Wrote", output, "cases", len(cases), "fresh GPU trees", len(timings), "launches", len(launches))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="new private summary directory")
    summarize(parser.parse_args().output)
