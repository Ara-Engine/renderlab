"""P-YARD2 shared manifest: one frame list per scene that both engines replay frame for frame.

Unreal CSV frame i and Renderlab frame i are the same camera, mover offsets and cut flag.

quarry (1920x1080, U0's matched content, black primary background, -14 EV):
    0-95     warm   p0 still (Unreal's diagnostics: DumpCVars 40/95, ProfileGPU 64)
    96-159   still  p0 still; the still capture is frame 159
    160-223  walk   registered Q-walk frames 1-64 (1 cm and 0.05 deg yaw per frame)
    224-255  hold   Q-walk 65-96 (held at the walk's end)
    256-319  return Q-walk 97-160 (back to p0)
    320      cut    history reset and a jump to q-disocclusion-047 (0.94 m sideways)
    321-351  recover held at the cut pose
Every pose is copied bit for bit from the registered sources, so U0b's path-traced
references match by exact camera key (scripts/m7-u0b-score.py `floor`).

s4-door / s5-cube: U0b's benches with their registered mover tracks. S4 looks at the
sliding door from inside (a revealed sunlit exterior, the ghosting/disocclusion case);
S5 keeps its registered corner pose (the cube moves only in the mirror).

    py -3 -B tools/p-yard/manifest.py      # writes tools/p-yard/manifest.json
"""
import hashlib
import json
from pathlib import Path

import numpy as np

HERE = Path(__file__).resolve().parent
U0B = Path("E:/renderlab-work/m7-u0b")
SCENES = Path("E:/renderlab-work/m7-u0/scenes")
QWALK = Path("E:/renderlab-work/t1-gpu/sequences/q-walk.json")
ASSET = Path("D:/Coding/projects/desktop/renderlab/measurements/quarry-quality/detail/full.rla")
ENVIRONMENT = Path("D:/Coding/renderlab-work/m5-independent-production/sun128.rle2")
# docs/m7-tr1.md: t1b-black-b lighting, the 0.04 rad (4.58 deg diameter) disk Unreal runs.
SUN = dict(to_light=[0.4, 0.8, 0.3], lux=[1000.0, 950.0, 850.0], angular_radius=0.04)
STILL_CAPTURE, CUT, END = 159, 320, 352
# The S4 door from inside the room: the front wall and its sliding door fill the view.
S4_DOOR_VIEW = dict(eye=[-0.3, 1.3, -1.2], target=[1.0, 1.0, 2.0])


def sha(path):
    with Path(path).open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


def key(eye, target, fov):
    return np.asarray(list(eye) + list(target) + [fov], "<f4").tobytes()


def references():
    """Zero-shutter, full-radiance, black-primary quarry references by exact camera key."""
    refs = {}
    for path in (U0B / "reference/quarry-final").glob("*.json"):
        try:
            meta = json.loads(path.read_text())
        except (json.JSONDecodeError, UnicodeDecodeError):
            continue
        frame = meta.get("frame") if isinstance(meta, dict) else None
        if not frame or frame["open"] != frame["close"] or frame.get("sky_only") or not path.with_suffix(".npz").exists():
            continue
        k = key(frame["open"]["eye"], frame["open"]["target"], frame["fov_y"])
        if k not in refs or meta["samples"] > refs[k]["samples"]:
            refs[k] = dict(name=path.stem, samples=meta["samples"])
    return refs


def quarry():
    walk = json.loads(QWALK.read_text())["frames"]
    unit = {f["name"]: f for f in json.loads((U0B / "quarry-unit-sequence.json").read_text())["frames"]}
    cut = unit["q-disocclusion-047"]
    p0 = walk[0]
    pose = lambda f: dict(eye=f["eye"], target=f["target"], fov_y=f["fov_y_radians"])
    frames = []
    for i in range(END):
        if i < 96:
            frames.append(dict(pose(p0), state="warm", source="q-walk-000"))
        elif i < 160:
            frames.append(dict(pose(p0), state="still", source="q-walk-000"))
        elif i < CUT:
            w = i - 159
            state = "walk" if w <= 64 else "hold" if w <= 96 else "return"
            frames.append(dict(pose(walk[w]), state=state, source=f"q-walk-{w:03}"))
        else:
            frames.append(dict(eye=cut["open"]["eye"], target=cut["open"]["target"], fov_y=cut["fov_y"],
                               state="cut" if i == CUT else "recover", source="q-disocclusion-047"))
    refs = references()
    for i, f in enumerate(frames):
        ref = refs.get(key(f["eye"], f["target"], f["fov_y"]))
        f.update(index=i, cut=i == CUT, reference=ref and ref["name"],
                 # Renderlab reads back only scored frames; Unreal's dump is contiguous from 159.
                 capture=i >= STILL_CAPTURE and ref is not None)
    return dict(kind="quarry", asset=str(ASSET), environment=str(ENVIRONMENT), sun=SUN, exposure_stops=-14.0,
                size=[1920, 1080], near=0.005, far=100.0, ue_level="/Game/M6Quarry/View0",
                ue_dump_first=STILL_CAPTURE, frames=frames,
                inputs=dict(asset_sha256=sha(ASSET), environment_sha256=sha(ENVIRONMENT), q_walk_sha256=sha(QWALK),
                            unit_sequence_sha256=sha(U0B / "quarry-unit-sequence.json")))


def bench(name, pose=None, dump=(0, 160)):
    path = SCENES / name / "scene.json"
    scene = json.loads(path.read_text())
    track = scene["tracks"][0]
    fov = float(np.radians(scene["fov_y_degrees"]))
    frames = []
    for f in track["frames"]:
        p = pose or dict(eye=f["eye"], target=f["target"])
        frames.append(dict(eye=p["eye"], target=p["target"], fov_y=fov, state=f.get("phase", track["name"]),
                           movers=f["movers"], index=f["frame"], cut=False, capture=dump[0] <= f["frame"] < dump[1]))
    return dict(kind="bench", bench=str(path), bench_sha256=sha(path), sun=None,
                exposure_stops=float(scene["exposure_stops"]), size=scene["size"], near=0.005, far=100.0,
                ue_level=f"/Game/M7U0b/{name}/Bench", ue_dump_first=dump[0], track=track["name"], frames=frames)


def main():
    manifest = dict(version=1, unit="P-YARD2", scenes=dict(
        quarry=quarry(),
        # Door slides frames 32-96; S5's cube moves frames 32-128. Capture the motion and its tail.
        **{"s4-door": bench("S4", S4_DOOR_VIEW, (24, 112)), "s5-cube": bench("S5", None, (24, 144))}))
    out = HERE / "manifest.json"
    out.write_text(json.dumps(manifest, indent=1) + "\n", encoding="utf-8")
    q = manifest["scenes"]["quarry"]["frames"]
    print(out, "quarry frames", len(q), "scored", sum(f["capture"] for f in q),
          "by state", {s: sum(1 for f in q if f["state"] == s and f["reference"]) for s in
                       ("still", "walk", "hold", "return", "cut", "recover")})


if __name__ == "__main__":
    main()
