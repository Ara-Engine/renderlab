"""P-MEM's scripts/n8-run.py adapter; read the live root order before taking a lease.

P_MEM_ADMISSION_MIB defaults to 8192; the root-authorized small-scene floor is 7168.
The runtime guard remains 2048 MiB. Leases are at most 15 minutes; no --fast-check.
"""
import argparse
import importlib.util
import os
from pathlib import Path
import sys
from p_mem_schedule import latest_order

repo = Path(__file__).resolve().parents[1]
out = Path(os.environ.get("P_MEM_OUT", "D:/Coding/renderlab-work/p-mem"))
parser = argparse.ArgumentParser(add_help=False)
parser.add_argument("--label", required=True)
parser.add_argument("--minutes", type=float, default=10)
args, _ = parser.parse_known_args()
if not 0 < args.minutes <= 15 or "--fast-check" in sys.argv[1:]:
    raise SystemExit("P-MEM leases require <=15 minutes and the explicit 7/8GiB admission floor")
if "--" not in sys.argv:
    raise SystemExit("separate the leased command with --")
separator = sys.argv.index("--")
if "--runtime-memory-mbytes" not in sys.argv[1:separator]:
    sys.argv[separator:separator] = ["--runtime-memory-mbytes", "2048"]
else:
    at = sys.argv.index("--runtime-memory-mbytes")
    if not float(sys.argv[at + 1]) >= 2048:
        raise SystemExit("the 2GiB runtime guard cannot be reduced")
admission = int(os.environ.get("P_MEM_ADMISSION_MIB", "8192"))
if admission < 7168:
    raise SystemExit("admission below the root-authorized 7GiB small-scene floor")
if "quarry" in args.label.lower() and admission < 8192:
    raise SystemExit("quarry leases require the 8GiB admission floor")
schedule = Path("E:/renderlab-work/gpu-schedule.md")
print(latest_order(schedule), flush=True)
os.environ.update(N8_OWNER="p-mem", N8_OUT=str(out))
spec = importlib.util.spec_from_file_location("p_mem_n8", repo / "scripts/n8-run.py")
n8 = importlib.util.module_from_spec(spec); spec.loader.exec_module(n8)
n8.s1.runner.PREFLIGHT_MEMORY_MBYTES = admission
try:
    raise SystemExit(n8.s1.runner.main())
finally:
    if n8.s1.claimed:
        with schedule.open("a", encoding="utf-8") as f:
            f.write(f"\np-mem {args.label} window released {n8.s1.runner.utc()} (no owned GPU/build job remains).\n")
