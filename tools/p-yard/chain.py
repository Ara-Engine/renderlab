"""One lease's worth of Unreal steps: timing visit 1, then the S4/S5 mover benches."""
import subprocess
import sys
from pathlib import Path

UE = [sys.executable, "-B", str(Path(__file__).with_name("ue.py"))]
BENCH = ["ue-hw-e", "ue-sw-e", "ue-hw-c", "ue-sw-c"]
steps = [["timing", "quarry", "--visit", "1"], ["capture", "s4-door", *BENCH], ["capture", "s5-cube", *BENCH]]
codes = [subprocess.run(UE + s).returncode for s in steps]
raise SystemExit(max(codes))
