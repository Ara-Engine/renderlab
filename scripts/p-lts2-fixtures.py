"""P-LTS2 baseline/candidate LC1-LC3 and night courtyard checks, under the lease.

The quarry test loads its asset once per path, exercises every retained LC variant,
and writes HDR buffers. Hash those buffers immediately; retain inspected PNGs only.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

import importlib.util

REPO = Path(__file__).resolve().parents[1]
ROOT = Path("D:/Coding/renderlab-work/p-lts2")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("label")
    p.add_argument("--candidate", type=Path, required=True)
    p.add_argument("--game", type=Path)
    p.add_argument("--steps", default="lib,fixture,mixed,quarry,game-still,game-move")
    p.add_argument("--paths", nargs="+", choices=["traced", "nonrt"], default=["traced", "nonrt"])
    p.add_argument("--skip-quarry-controls", action="store_true",
                   help="skip LC3 legacy/walk controls; keep identical soft quarry defaults")
    a = p.parse_args()
    spec = importlib.util.spec_from_file_location("p_lts_guard", REPO / "scripts/p-lts2-run.py")
    guard = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(guard)
    directive = guard.order()
    owner = json.loads(Path("D:/Coding/renderlab-work/gpu-slot.lock/owner.json").read_text())
    if owner["owner"] != "p-lts2":
        raise RuntimeError("P-LTS2 lease required")
    if "quarry" in a.steps.split(",") and owner.get("admission_mib", 0) < 8192:
        raise RuntimeError("full-quarry verification requires 8 GiB lease admission")
    out = ROOT / "runs" / a.label
    out.mkdir(parents=True, exist_ok=False)
    rows = []
    for step in a.steps.split(","):
        for variant in (["candidate"] if step == "lib" else ["base", "candidate"]):
            for mode in ([None] if step == "lib" else a.paths):
                latest = guard.order()
                if latest != directive:
                    if "Running leases are not stopped by this order." not in latest:
                        raise RuntimeError("root order changed between fixture steps")
                    directive = latest
                folder = out / f"{step}-{mode}-{variant}"
                folder.mkdir()
                binary = ROOT / "bin/base-tests.exe" if variant == "base" else a.candidate
                env = os.environ | dict(LC2="1", LC3="1", LC1_PATH=mode or "traced", LC1_OUT=str(folder),
                    LC1_QUARRY="E:/renderlab-work/m7-g1c/quarry/manifest.json", LC1_FRAMES="8",
                    LC2_NEAR="E:/renderlab-work/m7-g1c/near-cache/near-field-v1/rla1-rvg1/packages/743a2fce42fdf444407e23c2b00c0586ff0438806eeb023bfc19d91979c502f3",
                    VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation")
                env.pop("VK_INSTANCE_LAYERS", None)
                if step == "quarry" and a.skip_quarry_controls:
                    env["LC3"] = "0"
                if step.startswith("game"):
                    binary = ROOT / "bin/base-game.exe" if variant == "base" else a.game
                    command = [str(binary), "--path", "rt" if mode == "traced" else "nonrt", "--out", str(folder),
                        "--frames", "16" if step == "game-still" else "32", "--every", "8", "--night-lanterns",
                        "--timing-frames", "8", "--pipeline-cache", str(out / f"program-cache-{mode}")]
                    if step == "game-still":
                        command.append("--still")
                    fox = Path("D:/Coding/renderlab-work/m7-sk1/Fox.glb")
                    if fox.exists():
                        command += ["--fox", str(fox)]
                elif step == "lib":
                    command = [str(binary), "--test-threads=2"]
                elif step == "mixed":
                    env.update(RENDERLAB_ES1_PATH="rt" if mode == "traced" else "nonrt",
                               RENDERLAB_ES1_OUT=str(folder))
                    command = [str(binary), "renderer::primary_ray_gpu::program_lists::gpu_tests::gpu_mp1_mixed_programs",
                               "--ignored", "--exact", "--nocapture", "--test-threads=1"]
                else:
                    command = [str(binary), "renderer::light_cluster_tests::" +
                               ("gpu_lc2_penumbra" if step == "fixture" else "gpu_lc1_quarry_night"),
                               "--ignored", "--exact", "--nocapture", "--test-threads=1"]
                start = time.monotonic()
                start_unix = time.time()
                with (folder / "process.log").open("w") as log:
                    result = subprocess.run(command, cwd=REPO, env=env, stdout=log, stderr=subprocess.STDOUT,
                                            creationflags=subprocess.CREATE_NO_WINDOW, timeout=180)
                end_unix = time.time()
                hashes = {str(f.relative_to(folder)): hashlib.sha256(f.read_bytes()).hexdigest()
                          for f in sorted(folder.rglob("*")) if f.suffix in (".f32", ".png")}
                if step != "lib" and result.returncode == 0 and not hashes:
                    raise RuntimeError("fixture produced no image evidence")
                for f in folder.rglob("*.f32"):
                    if f.stat().st_size > 5_000_000:
                        f.unlink()
                row = dict(step=step, variant=variant, mode=mode, binary=str(binary),
                    binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), seconds=time.monotonic()-start,
                    exit=result.returncode, hashes=hashes,
                    start_unix=start_unix, end_unix=end_unix,
                    skip_quarry_controls=a.skip_quarry_controls)
                rows.append(row)
                different = []
                if variant == "candidate" and step != "lib":
                    baseline = next(r for r in rows if r["step"] == step and r["mode"] == mode and r["variant"] == "base")
                    different = [name for name in hashes.keys() | baseline["hashes"].keys()
                                 if hashes.get(name) != baseline["hashes"].get(name)]
                    row["identical_to_baseline"] = not different
                    row["different_images"] = different
                (out / "summary.json").write_text(json.dumps(dict(lease=owner, rows=rows), indent=2))
                print(step, variant, mode, row["exit"], row["seconds"], flush=True)
                if result.returncode:
                    raise RuntimeError((folder / "process.log").read_text()[-4000:])
                if different:
                    raise RuntimeError("pure optimization changed fixture images: " + ", ".join(different))


if __name__ == "__main__":
    main()
