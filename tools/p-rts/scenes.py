"""Supplemental P-RTS stream scenes: a frozen ARA1 timing scene plus 24 repeated cuts.

    scenes.py OUT_DIR SCENE...

Frames 0-87 are the frozen scene unchanged; frames 88-111 each cut at frame 72's pose,
so one process samples 24 first-cut frames (ARA1 has one per process). Not a
replacement for the frozen scene's identity; its JSON hash is recorded with each run.
"""
import json
from pathlib import Path
import sys

SCENES = Path("D:/Coding/renderlab-work/ara1/phase2-scenes")


def main():
    out = Path(sys.argv[1]);out.mkdir(parents=True, exist_ok=True)
    for name in sys.argv[2:]:
        scene = json.loads((SCENES / f"{name}.json").read_text())
        cut = scene["frames"][72]
        assert cut["cut"], "frame 72 is the registered first cut"
        scene["frames"] += [dict(cut, phase="cut_stream", cut=True, capture=False) for _ in range(24)]
        scene["name"] += " + P-RTS 24-cut stream"
        (out / f"{name.removesuffix('-timing')}-stream.json").write_text(json.dumps(scene, indent=2) + "\n")


if __name__ == "__main__":
    main()
