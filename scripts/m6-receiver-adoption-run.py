"""Guarded offscreen production-receiver runs; bulk evidence stays on E:."""
import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location("film_run", Path(__file__).with_name("m6-film-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
runner.OUT = Path("E:/renderlab-work/receiver-adoption")
runner.OWNER = "receiver-adoption-production"

if __name__ == "__main__":
    raise SystemExit(runner.main())
