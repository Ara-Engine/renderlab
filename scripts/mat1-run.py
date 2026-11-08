"""MAT1 offscreen material controls under the shared lease and game/RAM guards."""
import argparse
import importlib.util
import os
from pathlib import Path
import time

REPO = Path(__file__).resolve().parent.parent
OUT = Path("E:/renderlab-work/mat1")


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), REPO / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("label")
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--binary", type=Path, default=OUT / "bin/mat1-baseline.exe")
    parser.add_argument("--test", action="append")
    parser.add_argument("--path", choices=["rt", "nonrt"], default="rt")
    parser.add_argument("--minutes", type=int, default=5)
    parser.add_argument("--validate", action="store_true", help="require the SDK validation layer; isolate stale implicit overlay registrations")
    args = parser.parse_args()
    host = load("m7-u0b")
    host.OUT = OUT
    guard = host.ue()
    guard.OWNER = "mat1"
    guard.OUT = OUT
    schedule = load("m7-g1-run")
    out = OUT / "runs" / args.label
    out.mkdir(parents=True, exist_ok=False)
    guard.schedule_line(f"[mat1 request] owner=mat1 label={args.label}; short offscreen material controls, no timing claim, no Cargo inside.")
    while schedule.quiet_active(guard, any_lease=False):
        time.sleep(5)
    env = dict(os.environ,
        RENDERLAB_MAT1_MANIFEST=str(args.manifest.resolve()),
        RENDERLAB_EVIDENCE_DIR=str(out / "fixtures"),
        RENDERLAB_TEST_PRIMARY="traced" if args.path == "rt" else "raster",
        RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
        VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation")
    env.pop("VK_INSTANCE_LAYERS", None)
    if args.validate:
        empty = OUT / "empty-implicit-layers"
        empty.mkdir(exist_ok=True)
        env.pop("VK_LOADER_LAYERS_DISABLE", None)
        env["VK_IMPLICIT_LAYER_PATH"] = str(empty)
    with guard.lease(args.label, args.minutes) as owner:
        tests = args.test or ["tests::materials::response::gpu_quarry_material_response"]
        for index, test in enumerate(tests):
            log = out / ("run" if len(tests) == 1 else f"run-{index}")
            guard.guarded([args.binary, test, "--ignored", "--exact", "--nocapture", "--test-threads=1"],
                log, timeout=max(30, int(guard.remaining_seconds(owner)) - 10), cwd=REPO, env=env)


if __name__ == "__main__":
    main()
