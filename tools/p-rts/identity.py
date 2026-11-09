"""P-RTS default identity check inside tools/p-rts/lease.py (prebuilt, no cargo): the
listing-off default against main on the matched scenes, the listing-on quarry row for the
record, then naga and the library suite on the default build (lib last).

    identity.py ROOT BASE_EXE OFF_DIR DUAL_EXE
"""
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).parent))
from final import REPO, SCENES, step  # noqa: E402


def main():
    root, base, off, dual = Path(sys.argv[1]), sys.argv[2], Path(sys.argv[3]), sys.argv[4]
    root.mkdir(parents=True, exist_ok=True)
    failures = step("naga", [str(off / "renderlab-tests.exe"), "reconstruction_modules_validate_without_ray_queries"], root / "naga.log")
    failures += step("ara1-ab", [sys.executable, "-B", str(REPO / "tools/p-rts/ab.py"), str(root / "ab"),
                                 "--binary", f"base={base}", "--binary", f"off={off / 'ara1_capture.exe'}", "--binary", f"dual={dual}",
                                 "--reps", "1", "--quality", "quarry-daylight-quality", "game-base-quality", "ara-pbr-oriented-quality",
                                 "--timing", str(SCENES / "quarry-daylight-stream.json")], root / "ab.log")
    failures += step("lib", [str(off / "renderlab-tests.exe")], root / "lib.log")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
