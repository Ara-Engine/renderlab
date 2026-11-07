"""M7-U0d: generated, redistributable M7 benches, authored once for every renderer.

Scenes: SV (sky visibility), S4 (thin room), S5 (mirror corner), S2G (S2-glossy) and the two
small reference cross-check scenes X1 (open box) and X2 (sun disk + glossy plane).

Each scene directory gets:
  scene.json     schema 2: per-mesh materials, sun, sky, poses, tracks, registered populations.
                 Read by the reference path tracer and the Renderlab harnesses (src/reference_tests.rs).
  scene.gltf/bin the same meshes and materials for the Unreal glTF import path (bench-motion-ue.py
                 axis convention: source (x,y,z) is baked as (-z, y, x)).
  mitsuba/       scene.xml + PLY meshes (X1, X2, SV, S5) for the independent Mitsuba 3 cross-check.

Units: metres, Y up, radiance in cd/m^2, sun illuminance in lux at normal incidence.
Usage: py -3 -B scripts/m7-u0-scenes.py --output E:/renderlab-work/m7-u0/scenes [--only SV S4]
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct

import numpy as np

GREY = dict(base_color=[.5, .5, .5], metallic=0., roughness=.8, specular=1., emission=[0, 0, 0], double_sided=True)


def to_light(azimuth, elevation):
    a, e = map(math.radians, (azimuth, elevation))
    return [math.cos(e) * math.sin(a), math.sin(e), math.cos(e) * math.cos(a)]


class Builder:
    """Accumulates named meshes; every quad and box is wound counter-clockwise seen from outside."""

    def __init__(self, bench):
        self.scene = dict(schema=2, bench=bench, size=[1920, 1080], fov_y_degrees=45., exposure_stops=-8.,
                          smith="height-correlated", sun=None, sky=dict(constant=[0., 0., 0.]),
                          materials=[], meshes=[], poses=[], tracks=[], populations={})
        self.material_index = {}

    def material(self, name, **values):
        if name not in self.material_index:
            self.material_index[name] = len(self.scene["materials"])
            self.scene["materials"].append(dict(GREY, name=name, **values))
        return self.material_index[name]

    def mesh(self, name, positions, normals, indices, material, moving=False, thin=False):
        self.scene["meshes"].append(dict(name=name, positions=np.asarray(positions, float).round(7).tolist(),
                                         normals=np.asarray(normals, float).round(7).tolist(),
                                         indices=[int(i) for i in indices], material=material, moving=moving, thin=thin))

    def faces(self, name, quads, material, moving=False, thin=False):
        """One mesh from many planar quads (flat normals)."""
        p, n, idx = [], [], []
        for q in quads:
            q = np.asarray(q, float)
            normal = np.cross(q[1] - q[0], q[2] - q[0])
            normal /= np.linalg.norm(normal)
            base = len(p)
            p.extend(q.tolist()); n.extend([normal.tolist()] * 4)
            idx.extend([base, base + 1, base + 2, base, base + 2, base + 3])
        self.mesh(name, p, n, idx, material, moving, thin)

    def box(self, name, lo, hi, material, moving=False, thin=False):
        self.faces(name, box_quads(lo, hi), material, moving, thin)

    def sphere(self, name, centre, radius, material, segments=64, rings=32):
        c = np.asarray(centre, float)
        p, n = [], []
        for j in range(rings + 1):
            theta = math.pi * j / rings
            for i in range(segments + 1):
                phi = 2 * math.pi * i / segments
                d = np.array([math.sin(theta) * math.cos(phi), math.cos(theta), math.sin(theta) * math.sin(phi)])
                p.append((c + radius * d).tolist()); n.append(d.tolist())
        idx = []
        for j in range(rings):
            for i in range(segments):
                a = j * (segments + 1) + i; b = a + segments + 1
                if j != 0:
                    idx += [a, a + 1, b]
                if j != rings - 1:
                    idx += [a + 1, b + 1, b]
        self.mesh(name, p, n, idx, material)

    def pose(self, name, eye, target):
        self.scene["poses"].append(dict(name=name, eye=list(map(float, eye)), target=list(map(float, target))))


def box_quads(lo, hi):
    lo, hi = np.asarray(lo, float), np.asarray(hi, float)
    quads = []
    for axis in range(3):
        for side in (0, 1):
            u, v = [a for a in range(3) if a != axis]
            corners = []
            for a, b in ((0, 0), (1, 0), (1, 1), (0, 1)):
                point = lo.copy()
                point[axis] = (lo, hi)[side][axis]
                point[u] = (lo, hi)[a][u]; point[v] = (lo, hi)[b][v]
                corners.append(point)
            outward = np.zeros(3); outward[axis] = 1 if side else -1
            if np.dot(np.cross(corners[1] - corners[0], corners[2] - corners[0]), outward) < 0:
                corners.reverse()
            quads.append(corners)
    return quads


def sv():
    """Sky visibility: creases, an overhang, an underside, 1-100 mm gaps and a contact sphere; sky only."""
    b = Builder("SV")
    s = b.scene
    s.update(sky=dict(constant=[4000., 4000., 4000.]), exposure_stops=-11.)
    grey = b.material("grey", base_color=[.5, .5, .5], roughness=.8)
    b.box("ground", [-2, -.1, -2], [2, 0, 2], grey)
    b.box("wall-90", [-1.2, 0, -1.2], [-1.1, .6, .2], grey)          # 90-degree crease with the ground
    run = .5; rise = run * math.tan(math.radians(30))                 # 30-degree wedge crease
    b.faces("wedge-30", [[(-.9, 0, .5), (-.9 + run, rise, .5), (-.9 + run, rise, .9), (-.9, 0, .9)],
                         [(-.9 + run, 0, .5), (-.9 + run, 0, .9), (-.9 + run, rise, .9), (-.9 + run, rise, .5)]], grey)
    b.box("overhang-post", [-.2, 0, -1.0], [0., .17, -.6], grey)
    b.box("overhang", [0., .15, -1.0], [.6, .17, -.6], grey)          # 150 mm crevice below an underside
    for i, gap in enumerate((.001, .003, .01, .03, .1)):
        x = .2 + i * .3
        b.box(f"gap-{gap * 1000:g}mm-a", [x, 0, .2], [x + .02, .3, .5], grey)
        b.box(f"gap-{gap * 1000:g}mm-b", [x + .02 + gap, 0, .2], [x + .04 + gap, .3, .5], grey)
    b.sphere("contact-sphere", [.3, .1, 1.0], .1, grey)
    b.pose("overview", [1.2, 1.4, 2.4], [0., .1, -.1])
    b.pose("gaps", [1.0, .5, 1.2], [.8, .15, .35])
    s["populations"] = {"crease-90": ["wall-90"], "wedge-30": ["wedge-30"], "underside": ["overhang", "overhang-post"],
                        "gaps": [m["name"] for m in s["meshes"] if m["name"].startswith("gap-")],
                        "contact": ["contact-sphere"], "open": ["ground"]}
    s["registration"] = dict(reference="M7 reference path tracer, sky only, pixel-integrated, >=4096 spp",
                             measures="indirect HDR error per population; ground within 0.3 m of each feature")
    return s


def s4():
    """Thin room: 1/5/100 mm walls, skylight, sliding door, off-screen red panel; sun + sky."""
    b = Builder("S4")
    s = b.scene
    # Sun from behind-left (azimuth 200 deg): its patch through the skylight lands on the off-screen red
    # panel, and it lights the exteriors of the 1 mm (left) and 5 mm (back) walls.
    s.update(sun=dict(to_light=to_light(200, 60), lux=[100000., 98000., 94000.], angular_radius=.00465),
             sky=dict(constant=[6000., 7000., 9000.]), exposure_stops=-12.)
    wall = b.material("wall", base_color=[.7, .7, .7], roughness=.9)
    red = b.material("red-panel", base_color=[.8, .05, .05], roughness=.9)
    door = b.material("door", base_color=[.4, .3, .2], roughness=.7)
    b.box("ground", [-6, -.2, -6], [6, -.1, 6], b.material("ground", base_color=[.3, .3, .3]))
    b.box("floor", [-2.1, -.1, -2.1], [2.1, 0, 2.1], wall)
    # Ceiling with a 0.8 m skylight over the left middle of the room (x -1.6..-0.8, z -0.4..0.4).
    for name, lo, hi in (("ceiling-a", (-2.1, 2.5, -2.1), (2.1, 2.6, -.4)), ("ceiling-b", (-2.1, 2.5, .4), (2.1, 2.6, 2.1)),
                         ("ceiling-c", (-2.1, 2.5, -.4), (-1.6, 2.6, .4)), ("ceiling-d", (-.8, 2.5, -.4), (2.1, 2.6, .4))):
        b.box(name, lo, hi, wall)
    b.box("wall-left-1mm", [-2.001, 0, -2.0], [-2.0, 2.5, 2.0], wall, thin=True)
    b.box("wall-back-5mm", [-2.0, 0, -2.005], [2.0, 2.5, -2.0], wall, thin=True)
    b.box("wall-right-100mm", [2.0, 0, -2.0], [2.1, 2.5, 2.0], wall)
    # Front wall (100 mm) with a 1 m x 2 m doorway at x in [0.5, 1.5].
    b.box("wall-front-a", [-2.1, 0, 2.0], [.5, 2.5, 2.1], wall)
    b.box("wall-front-b", [1.5, 0, 2.0], [2.1, 2.5, 2.1], wall)
    b.box("wall-front-lintel", [.5, 2.0, 2.0], [1.5, 2.5, 2.1], wall)
    b.box("door", [.5, 0, 2.1], [1.5, 2.0, 2.14], door, moving=True)   # slides +x along the exterior
    b.box("red-panel", [-1.6, 0, 1.2], [-.6, 1.0, 1.25], red)          # behind the camera
    b.pose("interior", [0., 1.4, 1.6], [0., 1.0, -2.0])
    frames = []
    for f in range(160):
        open_m = min(1., max(0., (f - 32) / 64))
        frames.append(dict(frame=f, seconds=f / 60, eye=[0., 1.4, 1.6], target=[0., 1.0, -2.0],
                           to_light=s["sun"]["to_light"], movers={"door": [open_m, 0., 0.]},
                           phase="closed" if f < 32 else "opening" if f < 96 else "open"))
    s["tracks"].append(dict(name="door", frames=frames))
    s["populations"] = {"behind-1mm": ["wall-left-1mm"], "behind-5mm": ["wall-back-5mm"], "behind-100mm": ["wall-right-100mm"],
                        "red-bounce": ["floor", "wall-left-1mm"], "door-reveal": ["floor"]}
    s["registration"] = dict(leak="interior radiance of each thin wall against the reference; sun on the exterior",
                             offscreen_bounce="red/green ratio on the visible floor and walls (red panel off-screen)",
                             events="door: t90, overshoot and frames to within the bar after frame 32")
    return s


def s5():
    """Mirror corner: 1 m mirror, roughness-0.15 panel, off-screen moving cube; sun + sky."""
    b = Builder("S5")
    s = b.scene
    s.update(sun=dict(to_light=to_light(30, 40), lux=[100000., 98000., 94000.], angular_radius=.00465),
             sky=dict(constant=[6000., 7000., 9000.]), exposure_stops=-14.)
    grey = b.material("grey", base_color=[.5, .5, .5], roughness=.8)
    mirror = b.material("mirror", base_color=[.95, .95, .95], metallic=1., roughness=0.)
    gloss = b.material("gloss-0.15", base_color=[.9, .7, .4], metallic=1., roughness=.15)
    cube = b.material("cube", base_color=[.1, .4, .8], roughness=.6)
    b.box("floor", [-2, -.1, -2], [2, 0, 2], grey)
    b.box("wall-back", [-2, 0, -2], [2, 2, -1.9], grey)
    b.box("wall-left", [-2, 0, -1.9], [-1.9, 2, 2], grey)
    # The mirror (back wall) reflects the region behind-left of the camera, where the cube sits off-screen.
    b.box("mirror", [-1.0, .3, -1.9], [0., 1.3, -1.89], mirror)        # 1 m x 1 m on the back wall
    b.box("gloss-panel", [-1.9, .3, -1.5], [-1.89, 1.3, -.5], gloss)   # roughness 0.15 on the left wall
    b.box("cube", [-1.6, 0, 1.5], [-1.3, .3, 1.8], cube, moving=True)  # off-screen, seen in the mirror
    b.pose("corner", [.6, 1.1, 1.6], [-.8, .8, -1.9])
    frames = []
    for f in range(160):
        dz = -.3 * min(1., max(0., (f - 32) / 96))
        frames.append(dict(frame=f, seconds=f / 60, eye=[.6, 1.1, 1.6], target=[-.8, .8, -1.9],
                           to_light=s["sun"]["to_light"], movers={"cube": [0., 0., dz]}))
    s["tracks"].append(dict(name="cube", frames=frames))
    s["populations"] = {"mirror": ["mirror"], "gloss": ["gloss-panel"], "open": ["floor", "wall-back", "wall-left"]}
    s["registration"] = dict(mirror="edge spread, off-screen cube energy and position in the mirror",
                             gloss="roughness-0.15 band error, highlight energy and peak drift")
    return s


def s2g(cells=512):
    """S2-glossy: generated scan-like relief (sub-pixel triangles), low roughness, near-grazing sun."""
    b = Builder("S2G")
    s = b.scene
    # Sun low in front of the camera (azimuth 180, elevation 22 deg): mirror reflection needs about 4 deg of
    # micro-slope, which the relief has everywhere, so the metal glints in sub-pixel sparkles.
    s.update(sun=dict(to_light=to_light(180, 22), lux=[100000., 98000., 94000.], angular_radius=.00465),
             sky=dict(constant=[600., 700., 900.]), exposure_stops=-12., seed=0x52476C)
    metal = b.material("metal-0.1", base_color=[.9, .85, .8], metallic=1., roughness=.1)
    dielectric = b.material("dielectric-0.2", base_color=[.4, .38, .35], roughness=.2)
    rng = np.random.default_rng(s["seed"])
    size = .35
    for name, x0, material in (("relief-metal", -.37, metal), ("relief-dielectric", .02, dielectric)):
        z, x = np.meshgrid(np.linspace(-size / 2, size / 2, cells + 1), np.linspace(x0, x0 + size, cells + 1), indexing="ij")
        y = np.zeros_like(x)
        for octave in range(6):  # fBm relief, 0.1-3 mm amplitudes, ~4 deg slope per octave, ~1 mm finest wavelength
            f = 18 * 2 ** octave
            phase = rng.uniform(0, 2 * math.pi, 2)
            y += (.003 / 2 ** octave) * np.sin(f * x + phase[0]) * np.cos(f * 1.13 * z + phase[1])
        dz, dx = np.gradient(y, size / cells, size / cells)
        n = np.stack([-dx, np.ones_like(y), -dz], -1); n /= np.linalg.norm(n, axis=-1)[..., None]
        p = np.stack([x, y + .01, z], -1)
        a = np.arange(cells)[:, None] * (cells + 1) + np.arange(cells)[None, :]
        idx = np.stack([a, a + cells + 1, a + 1, a + 1, a + cells + 1, a + cells + 2], -1).ravel()
        b.mesh(name, p.reshape(-1, 3), n.reshape(-1, 3), idx, material)
    b.box("base", [-.6, -.05, -.4], [.6, 0, .4], b.material("grey", base_color=[.3, .3, .3]))
    eye, target = np.array([0., .45, .75]), np.array([0., 0., -.02])
    b.pose("glossy", eye, target)
    forward = (target - eye) / np.linalg.norm(target - eye)
    focal = 1080 / (2 * math.tan(math.radians(45) / 2))
    metres_per_pixel = float(np.linalg.norm(target - eye)) / focal
    for step in (.25, .5, 1.):
        frames = []
        for f in range(160):
            offset = step * metres_per_pixel * (f if f < 64 else 64 if f < 96 else 159 - f)
            shift = np.array([offset, 0, 0])
            frames.append(dict(frame=f, seconds=f / 60, eye=(eye + shift).tolist(), target=(target + shift).tolist(),
                               to_light=s["sun"]["to_light"], phase="out" if f < 64 else "hold" if f < 96 else "return"))
        s["tracks"].append(dict(name=f"camera-{step:g}px", pixels_per_frame=step, frames=frames))
    frames = []
    for f in range(160):
        frames.append(dict(frame=f, seconds=f / 60, eye=eye.tolist(), target=target.tolist(), to_light=to_light(180 + .25 * f, 22)))
    s["tracks"].append(dict(name="sun-0.25deg", frames=frames))
    s["forward"] = forward.tolist()
    s["metres_per_pixel_at_target"] = metres_per_pixel
    s["populations"] = {"metal-0.1": ["relief-metal"], "dielectric-0.2": ["relief-dielectric"]}
    s["registration"] = dict(flicker="RMS of (c_t - c_{t-1}) - (r_t - r_{t-1}) on each population",
                             highlights="reference-masked highlight energy and peak-position drift", shutter=0)
    return s


def x1():
    """Cross-check: open box with colour bleeding, a block and a rough F=1 conductor sphere; delta sun + sky."""
    b = Builder("X1")
    s = b.scene
    s.update(size=[320, 240], fov_y_degrees=40., smith="separable",
             sun=dict(to_light=[-.35, .85, .4], lux=[3000., 3000., 3000.], angular_radius=0.),
             sky=dict(constant=[200., 220., 260.]), exposure_stops=-9.)
    white = b.material("white", base_color=[.8, .8, .8], specular=0.)
    red = b.material("red", base_color=[.75, .1, .1], specular=0.)
    green = b.material("green", base_color=[.1, .6, .15], specular=0.)
    conductor = b.material("conductor-0.3", base_color=[1., 1., 1.], metallic=1., roughness=.55)
    b.box("floor", [-1, -.05, -1], [1, 0, 1], white)
    b.box("back", [-1, 0, -1.05], [1, 1.5, -1], white)
    b.box("left", [-1.05, 0, -1], [-1, 1.5, 1], red)
    b.box("right", [1, 0, -1], [1.05, 1.5, 1], green)
    b.box("block", [-.6, 0, -.6], [-.2, .7, -.2], white)
    b.sphere("sphere", [.35, .3, -.1], .3, conductor)
    b.pose("front", [0., .9, 3.2], [0., .5, -.3])
    return s


def x2():
    """Cross-check: sun disk (r = 0.05 rad) penumbra and its reflection in a roughness-0.32 F=1 plane."""
    b = Builder("X2")
    s = b.scene
    s.update(size=[320, 240], fov_y_degrees=40., smith="separable",
             sun=dict(to_light=to_light(0, 35), lux=[5000., 5000., 5000.], angular_radius=.05),
             sky=dict(constant=[100., 100., 100.]), exposure_stops=-9.)
    white = b.material("white", base_color=[.6, .6, .6], specular=0.)
    conductor = b.material("conductor-0.1", base_color=[1., 1., 1.], metallic=1., roughness=.32)
    b.box("ground", [-2, -.05, -2], [2, 0, 2], white)
    b.box("occluder", [-.5, .4, -.05], [.1, .42, .3], white)
    b.box("mirror-plane", [.2, 0, -.9], [1.2, .02, .1], conductor)
    b.pose("front", [.7, .25, -2.2], [.5, .1, 0.])
    return s


def write_gltf(s, out):
    """glTF 2.0 with one material per scene material; UE axis bake as bench-motion-ue.py expects."""
    blob = bytearray(); views, accessors, meshes = [], [], []

    def accessor(values, code, component, kind):
        while len(blob) % 4:
            blob.append(0)
        start = len(blob)
        flat = np.asarray(values, dtype="<f4" if code == "f" else "<u4").ravel()
        blob.extend(flat.tobytes())
        views.append(dict(buffer=0, byteOffset=start, byteLength=len(blob) - start))
        entry = dict(bufferView=len(views) - 1, componentType=component, count=len(values), type=kind)
        if kind == "VEC3":
            arr = np.asarray(values, float)
            entry.update(min=arr.min(0).tolist(), max=arr.max(0).tolist())
        accessors.append(entry)
        return len(accessors) - 1

    for m in s["meshes"]:
        convert = lambda v: np.asarray(v, float)[:, [2, 1, 0]] * [-1, 1, 1]
        p = accessor(convert(m["positions"]), "f", 5126, "VEC3")
        n = accessor(convert(m["normals"]), "f", 5126, "VEC3")
        i = accessor(m["indices"], "I", 5125, "SCALAR")
        meshes.append(dict(name=m["name"], primitives=[dict(attributes=dict(POSITION=p, NORMAL=n), indices=i, material=m["material"])]))
    materials = [dict(name=mat["name"], doubleSided=mat["double_sided"], emissiveFactor=mat["emission"],
                      pbrMetallicRoughness=dict(baseColorFactor=mat["base_color"] + [1.], metallicFactor=mat["metallic"],
                                                roughnessFactor=mat["roughness"])) for mat in s["materials"]]
    doc = dict(asset=dict(version="2.0", generator="Renderlab M7-U0 generated bench"), buffers=[dict(uri="scene.bin", byteLength=len(blob))],
               bufferViews=views, accessors=accessors, meshes=meshes, materials=materials,
               nodes=[dict(name=m["name"], mesh=i) for i, m in enumerate(s["meshes"])], scenes=[dict(nodes=list(range(len(meshes))))], scene=0)
    (out / "scene.bin").write_bytes(bytes(blob))
    (out / "scene.gltf").write_text(json.dumps(doc, indent=1))


def write_ply(path, positions, normals, indices):
    p = np.asarray(positions, "<f4"); n = np.asarray(normals, "<f4"); idx = np.asarray(indices, "<u4").reshape(-1, 3)
    header = (f"ply\nformat binary_little_endian 1.0\nelement vertex {len(p)}\nproperty float x\nproperty float y\nproperty float z\n"
              f"property float nx\nproperty float ny\nproperty float nz\nelement face {len(idx)}\nproperty list uchar uint vertex_indices\nend_header\n")
    faces = np.zeros(len(idx), dtype=[("n", "u1"), ("i", "<u4", 3)]); faces["n"] = 3; faces["i"] = idx
    path.write_bytes(header.encode() + np.hstack([p, n]).astype("<f4").tobytes() + faces.tobytes())


def write_mitsuba(s, out, pose=0, spp=4096, max_depth=-1):
    """Mitsuba 3 scene with the same geometry, camera, BSDFs and lights (no shared code)."""
    out.mkdir(exist_ok=True)
    w, h = s["size"]; v = s["poses"][pose]
    lines = ['<scene version="3.0.0">', f'<integrator type="path"><integer name="max_depth" value="{max_depth}"/></integrator>',
             '<sensor type="perspective">', '<string name="fov_axis" value="y"/>', f'<float name="fov" value="{s["fov_y_degrees"]}"/>',
             '<transform name="to_world"><lookat origin="{}" target="{}" up="0, 1, 0"/></transform>'.format(
                 ", ".join(map(str, v["eye"])), ", ".join(map(str, v["target"]))),
             f'<sampler type="independent"><integer name="sample_count" value="{spp}"/></sampler>',
             f'<film type="hdrfilm"><integer name="width" value="{w}"/><integer name="height" value="{h}"/>'
             '<string name="pixel_format" value="rgb"/><rfilter type="box"/></film>', '</sensor>']
    for k, mat in enumerate(s["materials"]):
        rgb = ", ".join(map(str, mat["base_color"]))
        if mat["metallic"] >= 1.:
            if mat["base_color"] != [1., 1., 1.]:
                raise ValueError("cross-check conductors must have F0 = 1")
            alpha = max(mat["roughness"] ** 2, 1e-4)
            inner = f'<bsdf type="roughconductor"><string name="material" value="none"/><string name="distribution" value="ggx"/><float name="alpha" value="{alpha}"/></bsdf>'
        elif mat["metallic"] == 0. and mat["specular"] == 0.:
            inner = f'<bsdf type="diffuse"><rgb name="reflectance" value="{rgb}"/></bsdf>'
        else:
            raise ValueError("cross-check materials are pure Lambert or F0 = 1 GGX conductors")
        lines.append(f'<bsdf type="twosided" id="m{k}">{inner}</bsdf>')
    for m in s["meshes"]:
        name = m["name"] + ".ply"
        write_ply(out / name, m["positions"], m["normals"], m["indices"])
        lines.append(f'<shape type="ply"><string name="filename" value="{name}"/><boolean name="face_normals" value="false"/><ref id="m{m["material"]}"/></shape>')
    sky = s["sky"]["constant"]
    lines.append(f'<emitter type="constant"><rgb name="radiance" value="{", ".join(map(str, sky))}"/></emitter>')
    sun = s["sun"]
    if sun:
        d = np.asarray(sun["to_light"], float); d /= np.linalg.norm(d)
        if sun["angular_radius"] == 0:
            lines.append('<emitter type="directional"><vector name="direction" value="{}"/><rgb name="irradiance" value="{}"/></emitter>'.format(
                ", ".join(map(str, (-d).tolist())), ", ".join(map(str, sun["lux"]))))
        else:
            # A uniform far sphere subtends exactly the disk; it also occludes the sky behind it, as in the tracer.
            distance = 1e5; r = sun["angular_radius"]; radius = distance * math.sin(r)
            radiance = [e / (math.pi * math.sin(r) ** 2) for e in sun["lux"]]
            centre = (d * distance).tolist()
            lines.append(f'<shape type="sphere"><point name="center" value="{", ".join(map(str, centre))}"/><float name="radius" value="{radius}"/>'
                         f'<emitter type="area"><rgb name="radiance" value="{", ".join(map(str, radiance))}"/></emitter>'
                         '<bsdf type="diffuse"><rgb name="reflectance" value="0"/></bsdf></shape>')
    lines.append("</scene>")
    (out / "scene.xml").write_text("\n".join(lines) + "\n")


BENCHES = dict(SV=sv, S4=s4, S5=s5, S2G=s2g, X1=x1, X2=x2)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--only", nargs="*", choices=list(BENCHES))
    args = parser.parse_args()
    report = {}
    for name in args.only or list(BENCHES):
        s = BENCHES[name]()
        s["triangles"] = sum(len(m["indices"]) // 3 for m in s["meshes"])
        out = args.output / name
        out.mkdir(parents=True, exist_ok=True)
        (out / "scene.json").write_text(json.dumps(s, separators=(",", ":")))
        write_gltf(s, out)
        if name in ("X1", "X2"):
            write_mitsuba(s, out / "mitsuba")
        hashes = {str(p.relative_to(out)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(out.rglob("*")) if p.is_file() and p.name != "identities.json"}
        (out / "identities.json").write_text(json.dumps(hashes, indent=1))
        report[name] = dict(triangles=s["triangles"], meshes=len(s["meshes"]), materials=len(s["materials"]),
                            tracks=[t["name"] for t in s["tracks"]], scene_sha256=hashes["scene.json"])
    print(json.dumps(report, indent=1))


if __name__ == "__main__":
    main()
