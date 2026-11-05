"""CPU-only summary and inspection images for the bounded retry screen."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import statistics
import time

os.environ.update(OPENBLAS_NUM_THREADS="2", OMP_NUM_THREADS="2")
import numpy as np
from PIL import Image, ImageDraw


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def codes(path):
    values = np.fromfile(path, "<f2").reshape(1080, 1920, 4)[..., :3].astype("f4")
    values = np.maximum(0, values)
    values /= 1 + values
    return np.uint8(np.clip(255 * np.where(values <= .0031308, 12.92 * values, 1.055 * values ** (1/2.4) - .055), 0, 255))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    args = parser.parse_args()
    start = time.perf_counter()
    manifest = json.loads(args.manifest.read_text())
    out = Path(manifest["output"])
    screen = json.loads((out / "screen.json").read_text())
    samples = json.loads((out / "timing/paired-timing.json").read_text())["samples"]
    timing = {}
    for mode in ("hard", "exact", "interactive"):
        old = {v["pair"]: v for v in samples if v["mode"] == mode and v["baseline"]}
        new = {v["pair"]: v for v in samples if v["mode"] == mode and not v["baseline"]}
        timing[mode] = {field: dict(adjacency_ms=statistics.median(v[field] for v in old.values()),
            retry_ms=statistics.median(v[field] for v in new.values()),
            paired_percent=statistics.median(100 * (new[k][field] / old[k][field] - 1) for k in old))
            for field in ("receiver_ms", "span_ms")}
    differences = {f"{pose}-{mode}": json.loads((out / f"diff-{pose}-{mode}.json").read_text())
        for pose in range(3) for mode in ("hard", "exact", "interactive")}
    witness_proof = module("retry_proof", Path(__file__).with_name("m6-receiver-ray-proof.py"))
    source = module("retry_source", Path(__file__).with_name("quarry-shadow-probe.py")).Source(Path(manifest["asset"]))
    rays = json.loads(Path(manifest["witness_rays"]).read_text())
    proofs = json.loads(Path(manifest["witness_rays"]).with_name("exact-intersections.json").read_text())
    witnesses = []
    for variant in screen["witnesses"]:
        for pose, (record, ray, proof) in enumerate(zip(variant["records"], rays, proofs)):
            hits = []
            for label, slot, primitive in (("raw", *record[:2]), ("repaired", *record[4:6])):
                mesh, _ = source.placements[slot - 1]
                _, vertices, indices = source.meshes[mesh][0]
                model = [[witness_proof.Fraction.from_float(float(x)) for x in row] for row in ray["model"]]
                local = vertices[indices[primitive], :3]
                world = [[model[k][3] + sum(model[k][j] * witness_proof.Fraction.from_float(float(p[j])) for j in range(3))
                          for k in range(3)] for p in local]
                hit = witness_proof.intersect([witness_proof.f32(x) for x in ray["origin_bits"]],
                    [witness_proof.f32(x) for x in ray["direction_bits"]], world)
                hits.append(dict(stage=label, placement=slot, primitive=primitive, intersection=hit,
                    exact_nearer_shared_facet=primitive == proof["exact_closest"]))
            witnesses.append(dict(variant=variant["variant"], pose=pose, hits=hits,
                shared_edge_exact_t_gap=proof["t_gap"], shared_edge_gap_float_spacings=proof["gap_float_spacings"]))
    sheet = Image.new("RGB", (960, 3 * 290), (15, 20, 26))
    draw = ImageDraw.Draw(sheet)
    for pose in range(3):
        for col, variant in enumerate(("adjacency", "retry")):
            image = Image.fromarray(codes(out / f"pose-{pose}-interactive-{variant}.rgba16f"))
            image.save(out / f"quarry-p{pose}-{variant}.png")
            sheet.paste(image.resize((480,270)), (col*480, pose*290+20))
            draw.text((col*480+8,pose*290+4),f"Pose {pose}: {variant}",fill="white")
    sheet.save(out / "quarry-comparison.png")
    changed = [(pose,x,y) for pose in range(3) for x,y in next(row["first_pixels"] for row in differences[f"{pose}-interactive"] if row["buffer"] == "hit")]
    crops = Image.new("RGB",(768, max(1,len(changed)) * 280),(15,20,26)); draw = ImageDraw.Draw(crops)
    for row,(pose,x,y) in enumerate(changed):
        a,b = [Image.open(out / f"quarry-p{pose}-{v}.png") for v in ("adjacency","retry")]
        delta = np.abs(np.array(a).astype("i2") - np.array(b).astype("i2")).max(2)
        highlight = np.zeros((1080,1920,3),dtype="u1");highlight[delta!=0]=[255,80,80]
        for col,(label,image) in enumerate((("adjacency",a),("retry",b),("display difference",Image.fromarray(highlight)))):
            crop=image.crop((x-16,y-16,x+16,y+16)).resize((256,256),Image.Resampling.NEAREST)
            crops.paste(crop,(col*256,row*280+24));draw.text((col*256+4,row*280+6),f"p{pose} ({x},{y}) {label}",fill="white")
    crops.save(out / "difference-crops.png")
    hashes = {str(p.relative_to(out)):hashlib.file_digest(p.open("rb"),"sha256").hexdigest()
        for p in out.rglob("*") if p.is_file() and p.suffix in (".wgsl", ".rgba16f", ".hits")}
    result = dict(timing=timing,differences=differences,witness_exact_classification=witnesses,
        source_sha256=hashes,seconds=time.perf_counter()-start)
    (out / "analysis.json").write_text(json.dumps(result,indent=2))
    print(json.dumps(dict(timing=timing,fold_identical=screen["fold_identical"],micro=screen["micro"],winding=screen["winding"],
        identity=[{k:v for k,v in row.items() if k!="samples"} for row in screen["identity"]],witnesses=witnesses),indent=2))


if __name__ == "__main__":
    main()
