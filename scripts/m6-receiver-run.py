"""R1 uses the F0 hidden-process, bounded GPU lease and game/memory guard."""
import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location("film_run", Path(__file__).with_name("m6-film-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
runner.OUT = Path("D:/Coding/renderlab-work/m6-exact-quality/r1")
runner.OWNER = "m6-receiver-policy-r1"

if __name__ == "__main__":
    raise SystemExit(runner.main())
