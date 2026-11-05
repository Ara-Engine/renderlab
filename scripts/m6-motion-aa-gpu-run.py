"""Guarded offscreen T2a GPU runs (docs/m6-motion-aa.md); evidence stays on E:.

py -3 -B scripts/m6-motion-aa-gpu-run.py --label <name> --manifest <json> -- <test-binary> <test> --ignored --exact --nocapture --test-threads=1
Same lease (D:/Coding/renderlab-work/gpu-slot.lock with owner.json), preflight (no game, Unreal
Editor or other renderlab GPU job, at least 6 GiB available) and ledger as scripts/m6-film-run.py.
"""
import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location("film_run", Path(__file__).with_name("m6-film-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
runner.OUT = Path("E:/renderlab-work/t2a")
runner.OWNER = "t2a-motion-records"

if __name__ == "__main__":
    raise SystemExit(runner.main())
