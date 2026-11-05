"""Guarded offscreen T1 GPU runs (docs/m6-temporal-primaries.md); evidence stays on E:.

py -3 -B scripts/m6-temporal-gpu-run.py --label <name> --manifest <json> -- <test-binary> <test> --ignored --exact --nocapture --test-threads=1
Same lease, preflight and ledger as scripts/m6-film-run.py.
"""
import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location("film_run", Path(__file__).with_name("m6-film-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
runner.OUT = Path("E:/renderlab-work/t1-gpu")
runner.OWNER = "t1-temporal-primaries"

if __name__ == "__main__":
    raise SystemExit(runner.main())
