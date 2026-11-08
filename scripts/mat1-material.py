"""MAT1 physical material controls; generated shaders and captures stay outside Git.

prepare MANIFEST creates a source-sky GGX cubature probe. It is diagnostic, not a
shipping replacement: no scene visibility is evaluated. score CAPTURES REFERENCES
compares every registered population using U0b's fixed F0 transform.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import struct

import numpy as np

REPO = Path(__file__).resolve().parent.parent


def cube_directions(size):
    axis = (np.arange(size) + 0.5) * (2 / size) - 1
    x, y = np.meshgrid(axis, axis)
    one = np.ones_like(x)
    directions = np.stack([np.stack(face, -1) for face in
        [(one, -y, -x), (-one, -y, x), (x, one, y), (x, -one, -y),
         (x, -y, one), (-x, -y, -one)]])
    norm = np.linalg.norm(directions, axis=-1)
    # Midpoint solid angles; the diagnostic's source cube is 128 square.
    omega = (2 / size) ** 2 / norm ** 3
    return directions / norm[..., None], omega


def source_cube(path):
    data = path.read_bytes()
    if data[:4] != b"RLE2":
        raise ValueError("MAT1 probe requires the registered RLE2 input")
    diffuse, size, levels = struct.unpack_from("<III", data, 64)
    yaw = struct.unpack_from("<f", data, 24)[0]
    offset = 128 + 4 * levels + 6 * diffuse * diffuse * 12
    return np.frombuffer(data, "<f4", 6 * size * size * 3, offset).reshape(-1, 3), size, yaw


SPECULAR = """
// The declared single-scatter GGX/Schlick model, without Lambert or scene visibility.
fn mat1_spec(n:vec3<f32>,v:vec3<f32>,l:vec3<f32>,f0:vec3<f32>,rough:f32)->vec3<f32> {
    let nl=max(dot(n,l),0.0);let nv=max(dot(n,v),1e-7);let h=unit_normal(l+v);
    let nh=max(dot(n,h),0.0);let vh=clamp(dot(v,h),0.0,1.0);
    let alpha=max(rough*rough,1e-4);let a2=alpha*alpha;
    let c=cross(n,h);let denom=max(dot(c,c)+a2*nh*nh,1e-20);
    let d=a2/(PI*denom*denom);
    let visibility=0.5/max(nl*sqrt(a2+(1.0-a2)*nv*nv)+nv*sqrt(a2+(1.0-a2)*nl*nl),1e-20);
    return d*visibility*(f0+(vec3<f32>(1.0)-f0)*pow(1.0-vh,5.0))*nl;
}
"""


def prepare(args):
    manifest = json.loads(args.manifest.read_text())
    out = Path(manifest["output"])
    out.mkdir(parents=True, exist_ok=True)
    source = (REPO / "shaders/programs/standard.wgsl").read_text()
    light, size, yaw = source_cube(Path(manifest["environment"]))
    directions, omega = cube_directions(size)
    directions = directions.reshape(-1, 3)
    omega = omega.ravel()
    c, s = np.cos(yaw), np.sin(yaw)
    # Inverse of the tracer's world-to-source yaw; actual registered quarry yaw is zero.
    directions = directions @ np.array([[c, 0, s], [0, 1, 0], [-s, 0, c]])
    luminance = light @ np.array([0.2126, 0.7152, 0.0722])
    mass = (luminance + 0.01 * max(luminance.mean(), 1e-10)) * omega
    pdf = mass / mass.sum()
    cdf = np.cumsum(pdf)
    (out / "baseline.wgsl").write_text(source)
    programs = [dict(name="baseline", source=str(out / "baseline.wgsl"))]
    old = """        let radiance=environment_reflection(reflected,rough);
        exposed=min(diffuse+environment_product(radiance,f0*ab.x+vec3<f32>(ab.y)),vec3<f32>(65504.0));"""
    if source.count(old) != 1:
        raise ValueError("standard program changed: inspect the probe splice")
    for count in args.samples:
        selected = np.searchsorted(cdf, (np.arange(count) + 0.5) / count)
        energy = light[selected] * (omega[selected] / pdf[selected] / count)[:, None]
        values = [v for pair in zip(directions[selected], energy) for v in pair]
        table = f"const MAT1_ENV:array<vec3<f32>,{2 * count}>=array<vec3<f32>,{2 * count}>(\n"
        table += ",\n".join("vec3<f32>(" + ",".join(f"{x:.9e}" for x in v) + ")" for v in values) + "\n);\n"
        replacement = f"""        var integral=vec3<f32>(0.0);
        for(var j=0u;j<{count}u;j+=1u) {{
            integral+=environment_product(MAT1_ENV[2u*j+1u],mat1_spec(n,v,MAT1_ENV[2u*j],f0,rough));
        }}
        exposed=min(diffuse+integral,vec3<f32>(65504.0));"""
        name = f"physical{count}"
        path = out / (name + ".wgsl")
        path.write_text(table + SPECULAR + source.replace(old, replacement))
        programs.append(dict(name=name, source=str(path)))
    manifest["programs"] = programs
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2))


def score(args):
    spec = importlib.util.spec_from_file_location("mat1_f0", REPO / "scripts/m7-u0b-score.py")
    scorer = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(scorer)
    report = {}
    for pose in (0, 1):
        path = args.references / f"quarry-p{pose}.npz"
        metadata = json.loads(path.with_suffix(".json").read_text())
        if metadata["archive_sha256"] != scorer.sha(path) or metadata["camera_background"] != [0, 0, 0]:
            raise ValueError("reference archive identity/background mismatch")
        with np.load(path) as archive:
            reference = (archive["L0"] + archive["L1"]) / 2
            populations = {k[4:]: archive[k] for k in archive.files if k.startswith("pop_") and archive[k].any()}
            hit = archive["hit"][..., 0]
            ground = archive["pop_all-covered"] & ~archive["pop_edge"] & archive["pop_lit"] & (archive["geometric_normal"][..., 1] > 0.97)
        rows = {}
        for name in args.programs:
            image = np.fromfile(args.captures / f"{name}-p{pose}.f32", "<f4").reshape(reference.shape)
            measures = {}
            for stops in (-14, -10):
                error = scorer.codes(image, stops) - scorer.codes(reference, stops)
                squared = (error * error).mean(-1)
                measures[str(stops)] = {p: dict(n=int(m.sum()), rms=float(np.sqrt(squared[m].mean())),
                    bias=float(error[m].mean())) for p, m in populations.items()}
            luminance = lambda image: image @ np.array([0.2126, 0.7152, 0.0722])
            ratio = luminance(image) / np.maximum(luminance(reference), 1e-8)
            measures["ground"] = {str(k): dict(n=int((mask := ground & (hit == k)).sum()),
                ratio=float(np.median(ratio[mask]))) for k in np.unique(hit[ground]) if (ground & (hit == k)).sum() > 1000}
            rows[name] = measures
        report[f"p{pose}"] = rows
    args.output.write_text(json.dumps(report, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("prepare")
    p.add_argument("manifest", type=Path)
    p.add_argument("--samples", nargs="+", type=int, default=[64, 256, 1024])
    p.set_defaults(run=prepare)
    p = sub.add_parser("score")
    p.add_argument("captures", type=Path)
    p.add_argument("references", type=Path)
    p.add_argument("output", type=Path)
    p.add_argument("--programs", nargs="+", default=["baseline", "physical256"])
    p.set_defaults(run=score)
    args = parser.parse_args()
    args.run(args)
