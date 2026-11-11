"""P-FLICKER error and flicker with P-YARD2's own scorer (score.py quality) over runs/ captures.

    py -3 -B tools/p-flicker/quality.py RUN...   RUN: runs/ directory (NAME-PATH)
"""
import os
import importlib.util
import json
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
UNIT = Path("D:/Coding/renderlab-work/p-flicker")  # Unreal captures and the clip manifest
WORK = Path(os.environ.get("P_FLICKER_WORK", UNIT))  # runs, sheets, builds (another unit sets its own)
spec = importlib.util.spec_from_file_location("pyard_score", HERE.parent / "p-yard/score.py")
score = importlib.util.module_from_spec(spec)
spec.loader.exec_module(score)
root = WORK / "score"
cases = root / "rl" / "quarry" / "capture"
cases.mkdir(parents=True, exist_ok=True)
for link in cases.iterdir():
    subprocess.call(["cmd", "/c", "rmdir", str(link)], stdout=subprocess.DEVNULL)  # junctions only
for run in sys.argv[1:]:
    subprocess.check_call(["cmd", "/c", "mklink", "/J", str(cases / run), str(WORK / "runs" / run)], stdout=subprocess.DEVNULL)
score.WORK = root
score.quality(None)
summary = json.loads((root / "quality.json").read_text())["summary"]
for config, states in summary.items():
    print(config, "flicker L0 %.3f L4 %.3f" % (states["moving"]["flicker@L0"]["rms"], states["moving"]["flicker@L4"]["rms"]),
          "| error still %.3f moving %.3f cut %.3f recover %.3f" % tuple(states[s]["all-covered@L0"]["rms"] for s in ("still", "moving", "cut", "recover")),
          "| edge moving %.2f" % states["moving"]["edge@L0"]["rms"])
