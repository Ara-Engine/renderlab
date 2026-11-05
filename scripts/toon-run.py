"""Run phase 2 offscreen gates with the shared GPU lease and process/RAM guard.

Uses determinism-run.py's preflight (games, Unreal, renderer jobs, >=6 GiB RAM).
No build or window launch. Each invocation owns one bounded lease. Raw captures
are retained for inspection/comparison; summarise them before deleting bulk data.
"""
from pathlib import Path
import argparse
import json
import os
import runpy
import subprocess
import threading
import time
import uuid

HELPERS = runpy.run_path(str(Path(__file__).with_name("determinism-run.py")))
LOCK, FLAGS = HELPERS["LOCK"], HELPERS["FLAGS"]
preflight, utc, save, sha = (HELPERS[k] for k in ("preflight", "utc", "save", "sha"))
PRIMARY = "renderer::fresh_shadow_tests::primary_ray_gpu_tests::"
TESTS = {
    "fixture": "renderer::program_gpu::toon_tests::gpu_toon_material_fixture",
    "quarry": "renderer::program_gpu::toon_tests::gpu_toon_quarry",
    "primary": PRIMARY + "gpu_primary_traced_quarry_acceptance",
    "interactive": PRIMARY + "soft_shadow_probe_tests::determinism_tests::gpu_static_repeat",
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--test", choices=TESTS, required=True)
    parser.add_argument("--minutes", type=float, default=10)
    parser.add_argument("--reference-hashes", type=Path)
    args = parser.parse_args()
    if not 0 < args.minutes <= 44:
        parser.error("minutes must be in (0,44]")
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    token = uuid.uuid4().hex
    owner = dict(owner="toon-phase-2", token=token, utc=utc(), launcher_pid=os.getpid(),
                 expected_minutes=args.minutes, output=str(out))
    try:
        LOCK.mkdir()
    except FileExistsError:
        save(out / "deferred.json", dict(utc=utc(), reason="GPU slot held"))
        print("DEFERRED: GPU slot held", flush=True)
        return 3
    save(LOCK / "owner.json", owner)
    try:
        state = preflight()
        save(out / "preflight.json", state)
        if state["conflicts"] or state["available_mb"] < 6144:
            print("DEFERRED: GPU process or available memory preflight", flush=True)
            return 3
        manifest = dict(json.loads(args.manifest.read_text()), output=str(out), validate=True)
        save(out / "manifest.json", manifest)
        settings = out / "vk_layer_settings.txt"
        settings.write_text("khronos_validation.validate_sync = true\n")
        implicit = out / "empty-implicit"
        implicit.mkdir()
        env = dict(os.environ, RENDERLAB_PRIMARY_MANIFEST=str(out / "manifest.json"),
                   RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
                   VK_LAYER_PATH=str(Path(os.environ.get("VULKAN_SDK", "C:/VulkanSDK/1.4.363.0")) / "Bin"),
                   VK_IMPLICIT_LAYER_PATH=str(implicit), VK_LAYER_SETTINGS_PATH=str(settings),
                   VK_INSTANCE_LAYERS="VK_LAYER_KHRONOS_validation")
        for key in ("VK_LOADER_LAYERS_DISABLE", "VK_LAYER_ENABLES", "VK_LAYER_DISABLES"):
            env.pop(key, None)
        command = [str(args.binary.resolve()), TESTS[args.test], "--ignored", "--exact", "--nocapture", "--test-threads=1"]
        start = time.monotonic()
        stop, reasons = threading.Event(), []
        with (out / "test.log").open("w", encoding="utf-8") as log:
            child = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT,
                                     env=env, creationflags=FLAGS)

            def monitor():
                while not stop.wait(3):
                    try:
                        state = preflight((child.pid,))
                        if state["conflicts"]:
                            reasons.append(state)
                            child.kill()
                            return
                    except Exception as error:
                        reasons.append(dict(monitor_error=str(error)))
                        child.kill()
                        return

            thread = threading.Thread(target=monitor)
            thread.start()
            try:
                child.wait(timeout=args.minutes * 60)
            except subprocess.TimeoutExpired:
                reasons.append(dict(timeout=True))
                child.kill()
                child.wait()
            finally:
                stop.set()
                thread.join()
        captures = {p.name: sha(p) for p in out.iterdir()
                    if p.suffix in (".rgba16f", ".f32", ".u32")}
        mismatches = []
        if args.reference_hashes:
            reference = json.loads(args.reference_hashes.read_text())
            reference = reference.get("hashes", reference.get("captures", reference))
            mismatches = [name for name in set(reference) | set(captures)
                          if reference.get(name) != captures.get(name)]
        result = dict(test=args.test, command=command, binary_sha256=sha(args.binary),
                      seconds=time.monotonic()-start, exit=child.returncode,
                      stop_reasons=reasons, captures=captures, reference_mismatches=mismatches)
        save(out / "run.json", result)
        print(json.dumps({k: v for k, v in result.items() if k != "captures"}), flush=True)
        return int(bool(child.returncode or reasons or mismatches))
    finally:
        assert json.loads((LOCK / "owner.json").read_text())["token"] == token
        (LOCK / "owner.json").unlink()
        LOCK.rmdir()
        save(out / "gpu-window.json", dict(acquired=owner["utc"], released=utc()))


if __name__ == "__main__":
    raise SystemExit(main())
