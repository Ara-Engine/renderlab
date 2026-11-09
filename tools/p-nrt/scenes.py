"""CPU-only: long single-phase profile scenes from ARA1's frozen 1080p timing scenes.

    py -3 -B tools/p-nrt/scenes.py [--frames 2000]

Each scene keeps ARA1's asset, lights, exposure and effects, warms 60 still frames,
then repeats one phase: `still` (frame-0 pose), `move` (ARA1's 0.02 m/frame pan,
wrapped back and forth over its 32-frame range) or `cut` (every frame a cut between
the still pose and ARA1's cut pose, -0.8 m). Nsight traces start well inside it.
"""
import argparse
import copy
import json
from pathlib import Path

ARA1 = Path("D:/Coding/renderlab-work/ara1/phase2-scenes")
OUT = Path("D:/Coding/renderlab-work/p-nrt/scenes")
CASES = ("game-base", "ara-pbr-oriented", "quarry-daylight")


def shifted(frame, shift, phase, cut):
    return dict(frame, eye=[frame["eye"][0] + shift, *frame["eye"][1:]],
                target=[frame["target"][0] + shift, *frame["target"][1:]], phase=phase, cut=cut, capture=False)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--frames", type=int, default=2000)
    a = p.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    for case in CASES:
        spec = json.loads((ARA1 / f"{case}-timing.json").read_text())
        base = spec["frames"][0]
        warm = [shifted(base, 0, "still", False) for _ in range(60)]
        phases = {
            "still": [shifted(base, 0, "still", False) for _ in range(a.frames)],
            # A triangle wave over ARA1's pan keeps the same view region indefinitely.
            "move": [shifted(base, .02 * (i % 62 if i % 62 < 31 else 62 - i % 62), "move", False) for i in range(a.frames)],
            "cut": [shifted(base, -.8 * (i % 2), "cut", True) for i in range(a.frames)],
        }
        for phase, frames in phases.items():
            item = copy.deepcopy(spec)
            item["frames"] = warm + frames
            item["name"] += f"-pnrt-{phase}"
            (OUT / f"{case}-{phase}.json").write_text(json.dumps(item, indent=1) + "\n")
    print(OUT)


if __name__ == "__main__":
    main()
