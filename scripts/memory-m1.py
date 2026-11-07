"""Headless quarry allocation/lifetime inventory; run under scripts/n8-run.py.

    py -3 -B scripts/memory-m1.py <test-binary> E:/renderlab-work/m1-memory/baseline

One process per device/path, one asset load per process, both poses. The renderer
records individual allocator allocations and blocks after upload and warm-up.
WDDM Dedicated Usage is sampled with memory-wddm.ps1 (same OS quantity as DXGI).
The lifetime/usage catalogue describes ownership, not inferred last-use aliasing.
Unknown labels fail the inventory so new resources cannot silently disappear.
"""
import argparse
import csv
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
QUARRY = Path("D:/Coding/renderlab-work/m6-adaptive/gpu-phase-quarry/quarry-manifest.json")
DETAIL = Path("D:/Coding/projects/desktop/renderlab/measurements/quarry-quality/detail")
PATHS = ("nort", "rt", "traced-virtual")


def resource(name):
    """Audited against the resource creation/retirement sites, not GPU address guesses."""
    if name == "mesh vertices":
        return "VERTEX/STORAGE/COPY_DST; BLAS_INPUT on RT devices (shared)", "Mesh; BLAS also retains the same buffer on RT devices"
    if name.startswith("mesh indices"):
        return "INDEX/STORAGE/COPY_SRC/COPY_DST; BLAS_INPUT on RT devices; adjacency suffix", "Mesh; original buffer also retained by BLAS after LOD attachment on RT devices"
    if name == "mesh tangents":
        return "vertex/storage tangent stream", "Mesh"
    if name == "N1c cluster vertices, triangles and primitive tables":
        return "STORAGE/COPY_DST: exact positions, packed u8 local triangles, primitive IDs", "Mesh cluster hierarchy"
    if name in ("N1c cluster instances", "N1c LOD groups"):
        return "STORAGE/COPY_DST: cluster placements or hierarchy groups", "VisibilityBuffer or VirtualState caster generation; matching immutable tables shared since M2"
    if name.startswith("renderlab hardware"):
        return "acceleration structure", "Mesh BLAS or active/retired TLAS generation; retired after last submission"
    if name == "immutable material texture":
        return "sampled RGBA8 texture with all source mips; COPY_DST", "Texture/Material handle (three renderer fallback textures persist)"
    if name == "streamed material texture levels":
        return "sampled RGBA8 texture holding resident source levels top..; COPY_SRC/COPY_DST", "Streamed Texture handle (M3); replaced at each residency change, the old copy retires after its last submission"
    if name == "texture residency level probe":
        return "sampled R16F 64x64 level probe, level j = 2^-j", "PrimaryState, from the first traced frame (M3)"
    if name == "texture residency words and requests":
        return "STORAGE/COPY_SRC/COPY_DST residency words and per-frame request/miss/survey counters", "PrimaryState, from the first traced frame (M3)"
    if name == "texture residency readback":
        return "MAP_READ/COPY_DST request counters", "Renderer flight ring slot or one verification (M3)"
    if name == "immutable environment cube":
        return "sampled environment array; COPY_DST", "EnvironmentMap handle (two renderer fallback cubes persist)"
    if name == "prepared DFG RG16F":
        return "sampled BRDF integration LUT", "Renderer"
    if name in ("instance transforms", "normal transforms", "visible instance IDs"):
        return "vertex/storage instance data; COPY_DST", "Instances"
    if name.startswith("immutable material") or name == "material program parameters":
        return "uniform/storage material data", "Material; renderer fallback material persists"
    if name.startswith("N8g walk") or name == "N8g absent walk records":
        return "storage geometry walk records/build uniforms", "VirtualState draw-list generation; rebuilt on geometry/settings change"
    if name.startswith("N8"):
        return "virtual-shadow storage/clipmap cache/raster data/uniforms", "VirtualState cache/draw-list generation; retired after last submission"
    if name.startswith("N1c"):
        return "visibility-buffer cluster/cut/HZB/raster storage or uniforms", "VisibilityBuffer draw-list/size generation; retired after last submission"
    if name.startswith("primary placement") or name == "primary absent storage":
        return "storage draw/placement tables", "PrimaryState geometry generation; retired after last submission"
    if name in ("primary hit records", "primary hard receiver epsilon", "primary tile coverage"):
        return "sampled/storage image; test COPY_SRC", "PrimaryState target-size generation; retired after last submission"
    if name.startswith("primary edge") or name == "primary sparse fallback and changed receivers":
        return "STORAGE/INDIRECT/COPY_DST (work queues also COPY_SRC); ABI-sized placeholders outside traced Interactive", "PrimaryState target-size/repair-capacity generation; retired after last submission"
    if name.startswith("primary"):
        return "uniform/storage camera/shading region", "PrimaryState"
    if name.startswith("fresh"):
        return "receiver/normal/count images (headless check: output attachment)", "FreshState target-size generation (headless check: harness)"
    if name == "exposed linear HDR":
        return "render/storage/sampled RGBA16F HDR target", "Lighting target-size generation; retired after last submission"
    if name in ("camera", "light records", "lighting eye/count/environment/exposure"):
        return "uniform/storage frame lighting", "Renderer/LightingState"
    if name in ("shadow sampling records", "disjoint shadow raster matrices", "redrawn hard shadow depth array"):
        return "uniform/storage/sampled depth shadow resources (fallback in these paths)", "ShadowState generation; retired after last submission"
    if name in ("aligned timestamp resolve", "flight timestamps"):
        return "QUERY_RESOLVE/COPY_SRC or COPY_DST/MAP_READ", "Renderer timestamp flight ring"
    if name.startswith("(wgpu"):
        return "wgpu internal initialization/build/staging resource", "wgpu-managed; build scratch/staging retired by submission completion"
    raise ValueError(f"Uncatalogued allocation: {name!r}")


def inventory(directory):
    for path in sorted(directory.glob("memory-*.json")):
        data = json.loads(path.read_text())
        rows = []
        for i, allocation in enumerate(data["allocations"]):
            usage, lifetime = resource(allocation["name"])
            block = next((j for j, b in enumerate(data["blocks"]) if b["allocations"][0] <= i < b["allocations"][1]), None)
            rows.append(dict(allocation=i, block=block, **allocation, usage=usage, lifetime=lifetime))
        with path.with_suffix(".csv").open("w", newline="", encoding="utf-8") as f:
            writer = csv.DictWriter(f, fieldnames=list(rows[0]))
            writer.writeheader()
            writer.writerows(rows)


def run(binary, root, name, memory_only, memory_hints, poses, visits):
    out = root / name
    out.mkdir(parents=True, exist_ok=True)
    m = json.loads(QUARRY.read_text())
    m.update(output=str(out), validate=False, timing=True, reference=False, hash_only=False,
             poses=poses[:1] if memory_only else poses, stage_repeats=0,
             turn_frames=0 if memory_only else 16, move_frames=0 if memory_only else 48,
             no_rt=name == "nort", lod_pixels=[])
    if visits > 1:
        m.update(virtual_variants={f"visit{i}": {} for i in range(visits)}, hash_only=True)
    if name == "nort":
        m.update(lod_pixels=[0.35], software_extent=[32], lod_lift=[1],
                 virtual_cache=str(DETAIL / "cache"), virtual_manifest=str(DETAIL / "virtual.manifest"))
    if name == "rt":
        m["soft_shadows"] = "interactive"
    manifest = out / "manifest.json"
    manifest.write_text(json.dumps(m, indent=2))
    stop = out / "stop"
    stop.unlink(missing_ok=True)
    flags = subprocess.CREATE_NO_WINDOW
    sampler = subprocess.Popen(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                                str(ROOT / "scripts/memory-wddm.ps1"), "-Match", "^" + binary.stem + "$",
                                "-Output", str(out / "gpumem.jsonl"), "-StopFile", str(stop),
                                "-IntervalMs", "250", "-Seconds", "600"], creationflags=flags)
    started = time.monotonic()
    env = dict(os.environ, RENDERLAB_PRIMARY_MANIFEST=str(manifest), VK_INSTANCE_LAYERS="",
               RENDERLAB_MEMORY_HINTS=memory_hints)
    try:
        with (out / "stdout.log").open("w", encoding="utf-8") as f:
            code = subprocess.run([str(binary), "gpu_visibility_quarry", "--ignored", "--nocapture", "--test-threads=1"],
                                  stdout=f, stderr=subprocess.STDOUT, env=env, creationflags=flags).returncode
    finally:
        stop.write_text("stop")
        sampler.wait(timeout=30)
    if code:
        raise RuntimeError(f"{name} exited {code}; see {out / 'stdout.log'}")
    for counts in out.glob("*.counts"):
        hashes = {}
        for extension in ("counts", "hit", "hdr", "world"):
            raw = counts.with_suffix("." + extension)
            hashes[extension + "_sha256"] = hashlib.file_digest(raw.open("rb"), "sha256").hexdigest()
            raw.unlink()
        counts.with_suffix(".hashes.json").write_text(json.dumps(hashes, indent=2))
    inventory(out)
    record = dict(path=name, memory_hints=memory_hints, seconds=time.monotonic() - started,
                  binary_sha256=hashlib.file_digest(binary.open("rb"), "sha256").hexdigest())
    samples = [json.loads(line) for line in (out / "gpumem.jsonl").read_text(encoding="utf-8-sig").splitlines()]
    record["wddm_max_bytes"] = max((s["dedicated"] for s in samples), default=None)
    warmed_at = min(p.stat().st_mtime for p in out.glob("memory-p*.json"))
    warm_samples = [s for s in samples if datetime.datetime.fromisoformat(s["t"]).timestamp() >= warmed_at]
    record["wddm_warm_max_bytes"] = max((s["dedicated"] for s in warm_samples), default=None)
    record["wddm_warm_samples"] = len(warm_samples)
    record["wddm_samples"] = len(samples)
    (out / "run.json").write_text(json.dumps(record, indent=2))
    print(record, flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("paths", nargs="*", help="nort, rt, traced-virtual (default: all)")
    parser.add_argument("--memory-only", action="store_true", help="one warm pose, no motion/hash replay")
    parser.add_argument("--memory-hints", choices=("default", "performance", "memory_usage", "small"), default="default")
    parser.add_argument("--poses", nargs="+", type=int, default=[0, 1])
    parser.add_argument("--visits", type=int, default=1, help="independent replays per pose in one asset load; >1 hashes counts/hits only")
    args = parser.parse_args()
    for name in args.paths or PATHS:
        if name not in PATHS:
            parser.error(f"unknown path: {name}")
        if args.visits < 1:
            parser.error("visits must be positive")
        run(args.binary, args.output, name, args.memory_only, args.memory_hints, args.poses, args.visits)


if __name__ == "__main__":
    main()
