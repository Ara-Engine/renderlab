"""One guarded T1a integration batch; no window and no asset reload between gates."""
import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location("film_run", Path(__file__).with_name("m6-film-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
runner.OUT = Path("E:/renderlab-work/integrate-t1a")
runner.OWNER = "integrate-t1a-temporal"

if __name__ == "__main__":
    raise SystemExit(runner.main())
