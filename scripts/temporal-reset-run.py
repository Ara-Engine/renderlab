"""Temporal reset reproduction/check batch: shared lease, schedule and 4/2 GiB guard."""
import importlib.util
import os
from pathlib import Path
import sys

os.environ.setdefault("RENDERLAB_MOTION_ROOT", "E:/renderlab-work/temporal-reset")

spec = importlib.util.spec_from_file_location("motion", Path(__file__).with_name("bench-motion-run.py"))
motion = importlib.util.module_from_spec(spec)
spec.loader.exec_module(motion)
runner = motion.s1.runner
runner.OUT = Path("E:/renderlab-work/temporal-reset")
runner.OWNER = "temporal-reset-identity"

if __name__ == "__main__":
    try:
        raise SystemExit(runner.main())
    finally:
        if motion.s1.claimed:
            with motion.s1.SCHEDULE.open("a", encoding="utf-8") as f:
                label = sys.argv[sys.argv.index("--label") + 1]
                f.write(f"\ntemporal-reset-identity {label} window released {runner.utc()} (no owned GPU/build job remains).\n")
