"""U-SPECK2 flights (one lease, resumable): the game scene's flight with its cut at frame 24
(180 frames, 1080p, frames 14, 21, 98 among every seventh) and its settled still (`--still`:
steady fire, still camera and objects, the last of 96 frames), on both paths. A run's name
picks its binary and switches; a finished run leaves `done.json` and is skipped on a rerun.

    py -3 -B tools/speck2/flights.py OUT name[@moving|@still]=EXE[,ENV=VALUE...][,--arg...] ...
"""
import json
import os
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
WORK = Path("D:/Coding/renderlab-work/unit-speck2")
FOX = "D:/Coding/renderlab-work/m7-sk1/Fox.glb"
KINDS = {
    "moving": ["--frames", "180", "--every", "7", "--diag", "cut=1"],
    "still": ["--still", "--frames", "96", "--every", "95"],
}


def main():
    out = Path(sys.argv[1])
    runs = sys.argv[2:]
    (WORK / "tmp").mkdir(parents=True, exist_ok=True)
    base = {k: v for k, v in os.environ.items() if k not in ("VK_INSTANCE_LAYERS", "VK_LAYER_PATH", "RENDERLAB_ABLATE", "RENDERLAB_REFLECTION_LONE")}
    base |= {"RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK": "I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK", "TMP": str(WORK / "tmp"), "TEMP": str(WORK / "tmp")}
    worst = 0
    for spec in runs:
        name, rest = spec.split("=", 1)
        name, _, only = name.partition("@")
        kinds = [only] if only else list(KINDS)
        exe, *switches = rest.split(",")
        env = dict(s.split("=", 1) for s in switches if not s.startswith("--"))
        extra = [part for s in switches if s.startswith("--") for part in s.split(" ")]
        for kind in kinds:
            for path in ("nonrt", "rt"):
                run = out / f"{kind}-{path}-{name}"
                if (run / "done.json").exists():
                    continue
                run.mkdir(parents=True, exist_ok=True)
                start = time.monotonic()
                r = subprocess.run([exe, "--path", path, "--fox", FOX, "--size", "1920x1080", *KINDS[kind], *extra, "--out", str(run),
                                    "--pipeline-cache", str(WORK / "cache")], cwd=REPO, env=base | env, capture_output=True, text=True, timeout=600)
                frames = len(list(run.glob("frame-*.png")))
                print(f"{run.name}: exit {r.returncode}, {frames} frames, {time.monotonic() - start:.0f} s", flush=True)
                if r.returncode == 0:
                    (run / "done.json").write_text(json.dumps(dict(exe=exe, env=env, extra=extra, frames=frames, tail=(r.stdout + r.stderr)[-400:])))
                else:
                    print((r.stdout + r.stderr)[-1500:], flush=True)
                worst = max(worst, r.returncode)
    return worst


if __name__ == "__main__":
    raise SystemExit(main())
