"""P-STILL flicker and error (CPU): P-YARD2's scorer (tools/p-yard/score.py quality) over this
unit's capture runs, so before/after numbers use the same manifest, references and metric.

    py -3 -B tools/p-still/flicker.py RUN...   RUN: a runs/ directory name (NAME-PATH) of a yard capture
"""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
WORK = Path("D:/Coding/renderlab-work/p-still")
spec = importlib.util.spec_from_file_location("pyard_score", HERE.parent / "p-yard/score.py")
score = importlib.util.module_from_spec(spec)
spec.loader.exec_module(score)

root = WORK / "score"
cases = root / "rl" / "quarry" / "capture"
cases.mkdir(parents=True, exist_ok=True)
for run in sys.argv[1:]:
    link = cases / run
    if not link.exists():
        subprocess.check_call(["cmd", "/c", "mklink", "/J", str(link), str(WORK / "runs" / run)], stdout=subprocess.DEVNULL)
score.WORK = root
score.quality(None)
summary = json.loads((root / "quality.json").read_text())["summary"]
for config, states in summary.items():
    moving = states.get("moving", {})
    print(config, "flicker L0 %.3f" % moving["flicker@L0"]["rms"],
          "| error still %.3f moving %.3f cut %.3f recover %.3f" % tuple(
              states[s]["all-covered@L0"]["rms"] for s in ("still", "moving", "cut", "recover")))
