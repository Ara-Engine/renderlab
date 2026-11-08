"""RF1–RF10: redistributable reflection fixtures using the U0 scene writer.

Metres, Y up, linear radiance. No scan pack or third-party geometry is embedded.
Tracks have 96 frames at 60 Hz, a 32-frame hold, 48 moving frames and a 16-frame hold.
The zero-shutter centre and finite 180-degree shutter are separate estimands.
"""
import argparse
import copy
import hashlib
import importlib.util
import json
import math
from pathlib import Path

import numpy as np


def load_u0():
    spec = importlib.util.spec_from_file_location("u0_scenes", Path(__file__).with_name("m7-u0-scenes.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


U0 = load_u0()
ROUGHNESSES = (0., .05, .1, .2, .3, .4, .5, .7, 1.)
MEASURES = ("rgb_relative_rms", "luminance_relative_l1", "signed_energy_bias",
            "display_rms_-14", "display_rms_-10", "p99_absolute_display_error",
            "edge_spread_pixels", "reflected_feature_energy", "reflected_feature_centroid",
            "motion_residual", "recovery_by_age_1_2_4_8", "trail_width_pixels")


def base(name, roughnesses=(0., .05, .1)):
    b = U0.Builder(name)
    s = b.scene
    s.update(exposure_stops=-14., camera_background=[0., 0., 0.],
             sun=dict(to_light=U0.to_light(145, 45), lux=[100000., 98000., 94000.], angular_radius=.00465),
             sky=dict(constant=[1500., 1800., 2400.]))
    ground = b.material("ground", base_color=[.18, .16, .14], roughness=.9)
    b.box("ground", [-6, -.12, -5], [6, -.02, 7], ground)
    for i, rough in enumerate(roughnesses):
        x = (i - (len(roughnesses)-1)/2)*1.6
        mat = b.material(f"puddle-r{rough:g}", base_color=[1., 1., 1.], metallic=1., roughness=rough)
        b.box(f"puddle-{i}", [x-.75, -.02, -1.5], [x+.75, 0, 1.5], mat)
    red = b.material("red-object", base_color=[.8, .025, .01], roughness=.65)
    blue = b.material("blue-object", base_color=[.015, .12, .8], roughness=.5)
    # The camera looks down. Tall upper faces project above the primary frustum,
    # while their virtual images remain in the horizontal puddle.
    b.box("offscreen-red", [-5.8, 0, -2.6], [-.5, 3.2, -2.1], red, moving=True)
    b.box("offscreen-blue", [.3, 0, -2.6], [5.8, 3.2, -2.1], blue)
    b.pose("pitch30", [0, 2, 3.4641016], [0, 0, 0])
    s["populations"] = {"reflector": [m["name"] for m in s["meshes"] if m["name"].startswith("puddle-")],
                        "direct-objects": ["offscreen-red", "offscreen-blue"], "ground": ["ground"]}
    s["populations"].update({f"reflector-r{r:g}": [f"puddle-{i}"] for i, r in enumerate(roughnesses)})
    s["registration"] = dict(measures=list(MEASURES), reference="pixel-integrated multi-bounce path tracing; no denoising",
        bands={"B0": [0., .05], "B1": [.05, .4], "B2": [.4, 1.]},
        content="Renderlab-authored procedural geometry; no external assets",
        shutter=dict(fps=60, fraction=.5, convention="centred; object/sun sampled at common shutter time"),
        floor="UE-SW mesh-SDF Epic and UE-HW Hit Epic; SSR rows informative only",
        reflection_probe="none; no planar actors or baked capture substituted for Lumen")
    return b


def camera(b, name, pitch):
    radius = 4.
    b.pose(name, [0, radius*math.sin(math.radians(pitch)), radius*math.cos(math.radians(pitch))], [0, 0, 0])


def track(b, kind):
    s = b.scene
    p = s["poses"][0]
    frames = []
    for frame in range(96):
        t = min(1., max(0., (frame-31)/48))
        eye = np.array(p["eye"]); target = np.array(p["target"])
        movers = {}; sun = s["sun"]["to_light"]
        if kind == "reveal":
            eye[0] += .8*t; target[0] += .8*t
            movers["occluder"] = [2.8*t, 0, 0]
        elif kind == "object-sun":
            movers["offscreen-red"] = [-1.2*t, .3*math.sin(math.pi*t), 0]
            sun = U0.to_light(145+30*t, 45)
        frames.append(dict(frame=frame, seconds=frame/60, eye=eye.tolist(), target=target.tolist(),
                           movers=movers, to_light=sun, phase="hold" if frame<32 else "motion" if frame<80 else "settled"))
    s["tracks"] = [dict(name=kind, frames=frames)]
    s["registration"]["witness_frames"] = [31, 32, 33, 35, 39, 48, 64, 79, 80, 81, 83, 87, 95]


def rf1():
    b = base("RF1")
    camera(b, "pitch20", 20); camera(b, "pitch40", 40)
    b.scene["registration"]["purpose"] = "Planar puddles r=0/.05/.1; off-screen cliffs; mirror identity"
    return b.scene


def rf2():
    b = base("RF2", ())
    b.scene["meshes"] = [m for m in b.scene["meshes"] if m["name"] == "ground"]
    for row, metal in enumerate((0., 1.)):
        for i, rough in enumerate(ROUGHNESSES):
            mat = b.material(f"{'metal' if metal else 'dielectric'}-r{rough:g}",
                base_color=[.7, .6, .4] if metal else [.2, .3, .4], metallic=metal, roughness=rough)
            b.sphere(f"row{row}-r{rough:g}", [(i-4)*.55, .3, -.45+row*.9], .25, mat, segments=40, rings=20)
    b.pose("sweep", [0, 2.8, 4.2], [0, .2, 0])
    b.scene["poses"] = b.scene["poses"][-1:]
    b.scene["populations"] = {f"{'metal' if row else 'dielectric'}-r{r:g}": [f"row{row}-r{r:g}"]
        for row in (0, 1) for r in ROUGHNESSES}
    # A bright wall gives the curved lobes a finite, nonuniform reflection target.
    mat = b.material("wall", base_color=[.65, .25, .08], roughness=.8)
    b.box("rock-wall", [-3, 0, -1.5], [3, 2.2, -1.4], mat)
    b.scene["registration"]["purpose"] = "Curved dielectric/metal roughness sweep; per-material B0/B1/B2 transitions"
    return b.scene


def rf3():
    b = base("RF3")
    mat = b.material("occluder", base_color=[.025, .025, .025], roughness=.8)
    b.box("occluder", [-1.4, 0, .7], [-.1, 1.6, .9], mat, moving=True)
    b.scene["populations"]["occluder"] = ["occluder"]
    track(b, "reveal")
    b.scene["registration"]["purpose"] = "Moving blocker and camera strafe reveal; disocclusion/ghosting"
    return b.scene


def rf4():
    b = base("RF4")
    mat = b.material("rails", base_color=[.8, .8, .8], metallic=1., roughness=.2)
    names = []
    for i, width in enumerate((.001, .003, .01, .03, .1)):
        x = (i-2)*.6
        for suffix, lo, hi in (("pole", [x, .1, -.4], [x+width, 1.6, -.4+width]),
                               ("rail", [x-.2, .8, -.4], [x+.3, .8+width, -.4+width])):
            name = f"{suffix}-{width:g}m"; names.append(name)
            b.box(name, lo, hi, mat, thin=True)
    b.scene["populations"]["thin-features"] = names
    b.scene["registration"]["purpose"] = "1/3/10/30/100 mm rails and poles; reflection thickness and feature energy"
    return b.scene


def rf5():
    b = base("RF5")
    track(b, "object-sun")
    b.scene["registration"]["purpose"] = "Moving reflected object plus 30-degree sun turn; response/settle"
    return b.scene


def rf6():
    b = base("RF6")
    b.scene["poses"] = []
    camera(b, "grazing5", 5); camera(b, "grazing10", 10)
    b.scene["registration"]["purpose"] = "Grazing Fresnel and stretched highlights at 5/10 degrees"
    return b.scene


def rf7():
    b = base("RF7", ())
    mat = b.material("facing-gloss", base_color=[.95, .95, .95], metallic=1., roughness=.05)
    b.box("panel-left", [-1.5, 0, -1.8], [-1.48, 2.8, 1.4], mat)
    b.box("panel-right", [1.48, 0, -1.8], [1.5, 2.8, 1.4], mat)
    b.scene["poses"] = []
    b.pose("between-panels", [.15, 1.2, 2.5], [-1.2, 1.2, -.5])
    b.scene["populations"]["reflector"] = ["panel-left", "panel-right"]
    b.scene["registration"].update(purpose="Facing glossy panels; recursive specular paths", depth_controls=[2, 4, 8, 16, 1024])
    return b.scene


def rf8():
    b = base("RF8", ())
    names = []
    for i, rough in enumerate((.3, .4, .5)):
        mat = b.material(f"wet-rock-r{rough:g}", base_color=[.18, .15, .11], metallic=0., roughness=rough)
        name = f"wet-rock-{i}"; names.append(name)
        b.sphere(name, [(i-1)*1.35, .55, 0], .65, mat)
        m = b.scene["meshes"][-1]
        p = np.asarray(m["positions"]); centre = np.array([(i-1)*1.35, .55, 0])
        d = p-centre
        # Smooth deterministic scan-like relief on a closed redistributable rock.
        scale = 1+.09*np.sin(11*d[:, 0])*np.cos(13*d[:, 2])+.045*np.sin(19*d[:, 1])
        m["positions"] = (centre+d*scale[:, None]).round(7).tolist()
        # Geometric flat normals prevent an unmeasured normal-map discrepancy.
        triangles = np.asarray(m["indices"]).reshape(-1, 3)
        vertices = np.asarray(m["positions"])[triangles]
        normals = np.cross(vertices[:, 1]-vertices[:, 0], vertices[:, 2]-vertices[:, 0])
        normals /= np.linalg.norm(normals, axis=1)[:, None]
        m["positions"] = vertices.reshape(-1, 3).tolist()
        m["normals"] = np.repeat(normals, 3, axis=0).tolist()
        m["indices"] = list(range(len(m["positions"])))
    b.scene["populations"]["reflector"] = names
    b.scene["registration"]["purpose"] = "Procedural wet quarry-like rocks r=.3/.4/.5; rough-band crevice occlusion"
    b.scene["registration"]["real_scan_scope"] = "Generated rocks only; no private quarry-pack redistribution or real-scan acceptance claim"
    return b.scene


def rf9():
    b = base("RF9")
    mat = b.material("shell", base_color=[.8, .2, .08], roughness=.7)
    # Normal points away from the puddle. The reflection must see its back face.
    b.faces("open-shell-back", [[[-1.2, 1.2, -1.0], [-1.2, 2.4, -1.0], [1.2, 2.4, -1.0], [1.2, 1.2, -1.0]]], mat, thin=True)
    b.scene["populations"]["shell"] = ["open-shell-back"]
    b.scene["registration"]["purpose"] = "Double-sided open shell reached from behind; missing-card black"
    return b.scene


def rf10():
    b = base("RF10")
    # Front lighting keeps the toon witness unoccluded wherever its quantised
    # direct band is nonzero. Both reference adapters check this registration.
    b.scene["sun"]["to_light"] = U0.to_light(0, 45)
    mat = b.material("toon", base_color=[.2, .7, .3], roughness=.7)
    b.sphere("toon-object", [0, .8, -.6], .6, mat)
    b.scene["materials"][mat]["program"] = "toon"
    b.scene["populations"]["toon"] = ["toon-object"]
    b.scene["registration"].update(purpose="Toon program evaluated at the reflected hit", toon_program="shaders/programs/toon.wgsl",
        toon_reference_requirement="An ordinary GGX/Lambert rendering is not a qualified toon reference; evaluate the toon view direction at every secondary hit")
    return b.scene


BENCHES = {f"RF{i}": globals()[f"rf{i}"] for i in range(1, 11)}


def state_at(scene, track_name, frame):
    """Interpolate the authored 60-Hz track for reference shutter samples and UE macros."""
    track = next(t for t in scene["tracks"] if t["name"] == track_name)["frames"]
    a = track[max(0, min(len(track)-1, int(math.floor(frame))))]
    b = track[max(0, min(len(track)-1, int(math.ceil(frame))))]
    t = min(1., max(0., frame-math.floor(frame)))
    mix = lambda x, y: ((1-t)*np.asarray(x)+t*np.asarray(y)).tolist()
    movers = {name: mix(a["movers"].get(name, [0, 0, 0]), b["movers"].get(name, [0, 0, 0]))
              for name in a["movers"] | b["movers"]}
    light = np.asarray(mix(a["to_light"], b["to_light"]))
    return dict(eye=mix(a["eye"], b["eye"]), target=mix(a["target"], b["target"]),
                movers=movers, to_light=(light/np.linalg.norm(light)).tolist())


def snapshot(scene, state):
    result = copy.deepcopy(scene)
    result["poses"] = [dict(name="centre", eye=state["eye"], target=state["target"])]
    result["sun"]["to_light"] = state["to_light"]
    for mesh in result["meshes"]:
        if mesh["name"] in state["movers"]:
            mesh["positions"] = (np.asarray(mesh["positions"])+state["movers"][mesh["name"]]).tolist()
    return result


def generate(output, only=None):
    summary = output/"fixtures.json"
    report = json.loads(summary.read_text()) if summary.exists() else {}
    for name in only or BENCHES:
        scene = BENCHES[name]()
        scene["triangles"] = sum(len(m["indices"])//3 for m in scene["meshes"])
        directory = output/name; directory.mkdir(parents=True, exist_ok=True)
        (directory/"scene.json").write_text(json.dumps(scene, separators=(",", ":")))
        U0.write_gltf(scene, directory)
        hashes = {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                  for p in sorted(directory.iterdir()) if p.name in ("scene.json", "scene.gltf", "scene.bin")}
        (directory/"identities.json").write_text(json.dumps(hashes, indent=2)+"\n")
        report[name] = dict(scene_sha256=hashes["scene.json"], triangles=scene["triangles"],
                            poses=[p["name"] for p in scene["poses"]], tracks=[t["name"] for t in scene["tracks"]])
    (output/"fixtures.json").write_text(json.dumps(report, indent=2)+"\n")
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--output", type=Path, default=Path("E:/renderlab-work/m7-r0a/scenes"))
    parser.add_argument("--only", nargs="+", choices=list(BENCHES))
    args = parser.parse_args()
    print(json.dumps(generate(args.output, args.only), indent=2))
