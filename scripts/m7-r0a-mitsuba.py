"""Independent CPU check of recursive reflections and RF10's directional toon closure.

Run under the retained Mitsuba venv with -I. Uses Mitsuba's intersections, path
integrator, GGX sampler, camera and pixel filter, and an independently expressed
directional emitter for the custom program (a program without scatter terminates).
This emitter has zero next-event sampling weight, matching the reference tracer's
BSDF-hit-only access to custom programs. No Mitsuba source is copied.
"""
import argparse
import importlib.util
import json
import math
from pathlib import Path
import time


def main():
    import mitsuba as mi
    import drjit as dr
    import numpy as np
    mi.set_variant("scalar_rgb")
    dr.set_thread_count(2)
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("scene", type=Path)
    p.add_argument("output", type=Path)
    p.add_argument("--spp", type=int, default=4096)
    p.add_argument("--seed", type=int, default=1)
    p.add_argument("--sun-sampler-probe", action="store_true")
    args = p.parse_args()
    scene = json.loads((args.scene/"scene.json").read_text())
    if args.sun_sampler_probe:
        sun = scene["sun"]; direction = np.asarray(sun["to_light"], dtype="f8")
        direction /= np.linalg.norm(direction)
        rows = []
        for angular_radius in (sun["angular_radius"], .05):
            radius = 1e5*math.sin(angular_radius)
            radiance = np.asarray(sun["lux"])/(math.pi*math.sin(angular_radius)**2)
            native = mi.load_dict({"type":"scene", "sun":{"type":"sphere",
                "center":(direction*1e5).tolist(), "radius":radius,
                "emitter":{"type":"area", "radiance":{"type":"rgb", "value":radiance.tolist()}}}})
            emitter = native.emitters()[0]
            for point, normal in (([0., 0., 0.], [0., 1., 0.]), ([-3., 1.6, -2.1], [0., 0., 1.])):
                it = mi.Interaction3f(); it.p = mi.Point3f(point)
                total = np.zeros(3); pdfs = []
                for y in range(32):
                    for x in range(32):
                        ds, weight = emitter.sample_direction(it, mi.Point2f((x+.5)/32, (y+.5)/32))
                        total += np.asarray(weight)*max(0., float(dr.dot(ds.d, mi.Vector3f(normal))))/1024
                        pdfs.append(float(ds.pdf))
                expected = np.asarray(sun["lux"])*float(np.dot(direction, normal))
                rows.append(dict(angular_radius=angular_radius, point=point, normal=normal,
                    measured_irradiance=total.tolist(), expected_far_sun_irradiance=expected.tolist(),
                    ratio=(total/expected).tolist(), pdf_min=min(pdfs), pdf_max=max(pdfs)))
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(dict(mitsuba=mi.__version__, samples=1024,
            method="direct emitter sample_direction integral, no occluders or BSDF; same U0 far sphere", rows=rows), indent=2)+"\n")
        return

    class Toon(mi.Emitter):
        def __init__(self, props):
            props["sampling_weight"] = 0.
            super().__init__(props)
            self.m_flags = mi.EmitterFlags.Surface | mi.EmitterFlags.SpatiallyVarying
            self.base = mi.Color3f([float(x) for x in props["base"]])

        def eval(self, si, active=True):
            v = si.to_world(si.wi)
            normal = si.sh_frame.n
            if dr.dot(normal, v)<0:
                normal = -normal
            illumination = max(0., min(1., (dr.dot(normal, mi.Vector3f(scene["sun"]["to_light"]))-.12)/.78))
            level = math.floor(3*illumination+.5)/3
            tint = mi.Color3f(.12, .08, .24)*(1-level)+mi.Color3f(level)
            rim = (1-max(0., min(1., dr.dot(normal, v))))**4
            return self.base*mi.Color3f(scene["sky"]["constant"])*.25 + self.base*mi.Color3f(scene["sun"]["lux"])*tint/math.pi \
                + mi.Color3f(.12, .2, .3)*rim*2**(-scene["exposure_stops"])

        def sample_direction(self, it, sample, active=True):
            return mi.DirectionSample3f(), mi.Color3f(0)

        def pdf_direction(self, it, ds, active=True):
            return 0.

        def bbox(self):
            return mi.ScalarBoundingBox3f()

        def to_string(self):
            return "R0aDirectionalToon[sampling_weight=0]"

    mi.register_emitter("r0a_toon", lambda props: Toon(props))
    xml = (args.scene/"mitsuba/scene.xml").read_text()
    # The writer emits a zero-scattering Lambert placeholder for the toon shape;
    # add the independent directional emitter to that shape alone.
    for mesh in scene["meshes"]:
        material = scene["materials"][mesh["material"]]
        if material.get("program")!="toon":
            continue
        needle = f'<ref id="m{mesh["material"]}"/></shape>'
        rgb = ", ".join(map(str, material["base_color"]))
        emitter = f'<ref id="m{mesh["material"]}"/><emitter type="r0a_toon"><rgb name="base" value="{rgb}"/></emitter></shape>'
        # Limit replacement to the registered PLY shape; no material-wide alias.
        start = xml.index(f'<shape type="ply"><string name="filename" value="{mesh["name"]}.ply"')
        end = xml.index('</shape>', start)+len('</shape>')
        shape = xml[start:end]
        if needle not in shape:
            raise ValueError("toon emitter shape adapter changed")
        xml = xml[:start]+shape.replace(needle, emitter)+xml[end:]
    start = time.perf_counter()
    mi.Thread.thread().file_resolver().append(str(args.scene/"mitsuba"))
    native = mi.load_string(xml)
    image = np.array(mi.render(native, spp=args.spp, seed=args.seed), dtype=np.float32)[..., :3]
    if not np.isfinite(image).all():
        raise ValueError("nonfinite Mitsuba output")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    np.save(args.output, image)
    args.output.with_suffix(".json").write_text(json.dumps(dict(spp=args.spp, seed=args.seed,
        seconds=time.perf_counter()-start, mitsuba=mi.__version__, variant="scalar_rgb", shape=list(image.shape),
        custom_closure="independent directional emitter; sampling_weight 0"), indent=2)+"\n")


if __name__ == "__main__":
    main()
