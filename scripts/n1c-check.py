"""Batched N1c CPU gate; invoke under n1c-run.py with an E:-local Cargo target."""
from pathlib import Path
import subprocess
import sys

target = str(Path(sys.argv[1]))
for command in (
    ["cargo", "fmt", "--all", "--", "--check"],
    ["cargo", "test", "-p", "renderlab", "--lib", "-j", "2", "--target-dir", target],
    # main aee5007 already has nonminimal_bool in virtual_shadow_gpu.rs:71.
    # N8b owns that file. Keep every other Clippy warning an error.
    ["cargo", "clippy", "-p", "renderlab", "--lib", "-j", "2", "--target-dir", target,
     "--", "-D", "warnings", "-A", "clippy::nonminimal_bool"],
):
    print(" ".join(command), flush=True)
    result = subprocess.run(command)
    if result.returncode:
        raise SystemExit(result.returncode)
