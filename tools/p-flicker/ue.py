"""P-FLICKER Unreal captures: tools/p-yard/ue.py unchanged, writing under p-flicker/ue.
P-YARD2's quarry captures were deleted after scoring; this recaptures them for the
side-by-side clips and the per-population split. Same arguments as tools/p-yard/ue.py.

    py -3 -B tools/p-flicker/host.py lease LABEL 15 -- py -3 -B tools/p-flicker/ue.py capture quarry ue-hw-e ue-sw-e
    py -3 -B tools/p-flicker/ue.py decode quarry    # CPU, after the lease
"""
import importlib.util
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("pyard_ue", HERE.parent / "p-yard/ue.py")
ue = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ue)
ue.WORK = Path("D:/Coding/renderlab-work/p-flicker/ue")
# Unreal's exec paths are package-relative, so the macros share its drive (tiny; removed at the end).
ue.MACROS = Path("E:/renderlab-work/m7-u0b/m/pflicker")

if __name__ == "__main__":
    ue.main()
