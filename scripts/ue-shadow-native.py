"""Guarded native main-view shadow captures; private RDG resources never enter Git.

The existing packaged Development executable is reused, without cooking/building.
DumpGPU frames are diagnostic readbacks, never timing samples. The same native
main-view pipeline can be retimed separately with --timing after a fidelity check.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re

ROOT = Path("D:/Coding/renderlab-work/ue-m5/Quarry")
PACKAGE = ROOT / "package/Windows/Engine/Binaries/Win64/UnrealGame.exe"
CONFIGS = {
    "vsm-native": (),
    "vsm-screen-zero": ("r.Shadow.Virtual.ScreenRayLength 0",),
    "vsm-normal-zero": ("r.Shadow.Virtual.NormalBias 0",),
    "vsm-unshadowed": ("ShowFlag.DynamicShadows 0",),
    "hrt-256": ("r.RayTracing.Shadows.SamplesPerPixel 256", "r.Shadow.Denoiser 0"),
    "hrt-native": (),
    "hrt-unshadowed": ("ShowFlag.DynamicShadows 0",),
    "hrt-epsilon-zero": ("r.RayTracing.Shadows.SamplesPerPixel 256", "r.Shadow.Denoiser 0",
                         "r.RayTracing.Shadows.AvoidSelfIntersectionTraceDistance 0"),
}


def load_runner():
    spec = importlib.util.spec_from_file_location("quarry_runner", Path(__file__).with_name("ue-quarry-run.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def commands(config, dump_directory, descriptors=False, root="Lights"):
    docs = Path(__file__).resolve().parent.parent / "docs"
    # Include every explicitly named control, plus production settings and the
    # history/geometry switches needed to interpret the captured shader parameters.
    named = set()
    for name in ("unreal-shadow-mismatch.md", "unreal-quarry-full-detail-comparison.md"):
        named.update(re.findall(r"\br\.[A-Za-z0-9_.]+", (docs / name).read_text(encoding="utf-8")))
    named.update(("r.RayTracing.Shadows.EnableTwoSidedGeometry", "r.RayTracing.Geometry.InstancedStaticMeshes.Culling",
                  "r.RayTracing.Geometry.InstancedStaticMeshes", "r.RayTracing.Culling",
                  "r.Shadow.Denoiser.HistoryConvolutionSamples", "r.Shadow.Denoiser.MaxBatchSize",
                  "r.Shadow.Denoiser.TemporalAccumulation", "r.Shadow.Virtual.SMRT.RayCountDirectional",
                  "r.Shadow.Virtual.SMRT.SamplesPerRayDirectional", "r.Shadow.Virtual.SMRT.AdaptiveRayCount",
                  "r.ShadowQuality", "r.Shadow.Virtual.ResolutionLodBiasDirectional", "r.Shadow.Virtual.MaxPhysicalPages",
                  "r.Shadow.Virtual.Cache", "r.Shadow.Virtual.OnePassProjection.SkipScreenShadowMask"))
    result = ["r.DumpGPU.Upload 0", "r.DumpGPU.Explore 0", "r.DumpGPU.ConsoleVariables 1",
              "r.DumpGPU.Stream 0", "r.DumpGPU.FrameCount 1", "r.DumpGPU.PassParameters 1",
              f"r.DumpGPU.Texture {1 if descriptors else 2}", "r.DumpGPU.Buffer 1",
              "r.DumpGPU.Directory " + dump_directory.as_posix(), "r.DumpGPU.Root " + root]
    if config.startswith("hrt"):
        result += ["set DirectionalLightComponent LightSourceAngle 4.583662361",
                   "set DirectionalLightComponent ShadowSourceAngleFactor 1",
                   # Development's `set` updates the UObject but leaves its
                   # existing render proxy stale. This reversible toggle invokes
                   # the engine's global render-state recreation callback.
                   "r.Shadow.CacheWPOPrimitives 1", "r.Shadow.CacheWPOPrimitives 0"]
    result.extend(CONFIGS[config])
    result += ["getall DirectionalLightComponent LightSourceAngle", "getall DirectionalLightComponent ShadowSourceAngleFactor",
               "getall DirectionalLightComponent LightSourceSoftAngle", "getall DirectionalLightComponent ContactShadowLength",
               "getall DirectionalLightComponent Intensity", "getall DirectionalLightComponent LightColor",
               "getall CameraComponent FieldOfView", "getall CameraComponent AspectRatio",
               "getall CameraComponent bConstrainAspectRatio", "getall SceneComponent RelativeLocation",
               "getall SceneComponent RelativeRotation", "r.GBufferFormat"]
    result += sorted(named)
    return result


def main(args):
    if re.fullmatch(r"[a-z0-9-]+", args.label) is None:
        raise ValueError("use a lowercase alphanumeric run label")
    directory = ROOT / "native-shadow" / args.label
    directory.mkdir(parents=True, exist_ok=False)
    extra = commands(args.config, directory, args.descriptors, args.root)
    events = None if args.timing else [(63, "getall PlayerCameraManager CameraCachePrivate"),
                                     (64, "DumpGPU"), (90, "D3D12.DumpRayTracingGeometries all")]
    recipe = dict(configuration=args.config, pose=args.pose, commands=extra, csv_commands=events,
                  pipeline="packaged Development main view", timing=args.timing,
                  scripts={p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in
                           (Path(__file__), Path(__file__).with_name("ue-quarry-run.py"),
                            Path(__file__).with_name("ue-quarry-slot.py"), Path(__file__).with_name("ue-quarry-preflight.ps1"))})
    (directory / "recipe.json").write_text(json.dumps(recipe, indent=2) + "\n")
    runner = load_runner()
    runner.run("profile", args.label, "hardware-rt-denoised" if args.config.startswith("hrt") else "vsm-smrt",
               args.timeout, args.pose, PACKAGE, extra, events)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("config", choices=CONFIGS)
    parser.add_argument("label")
    parser.add_argument("--pose", type=int, choices=(0, 1, 2), default=0)
    parser.add_argument("--timeout", type=int, default=600)
    parser.add_argument("--root", default="Lights")
    parser.add_argument("--descriptors", action="store_true", help="discover the exact resource/pass names first")
    parser.add_argument("--timing", action="store_true", help="separate packaged GPU tree/CSV run without DumpGPU")
    main(parser.parse_args())
