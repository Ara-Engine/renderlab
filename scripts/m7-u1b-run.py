"""U1 offscreen executable runner: shared lease, reservation/release, game and 4/2 GiB guards.

Compile and run all Cargo commands before calling this. This runner rejects Cargo.
"""
import importlib.util
import sys
from pathlib import Path

spec = importlib.util.spec_from_file_location("contact_runner", Path(__file__).with_name("bench-contact-fold-run.py"))
contact = importlib.util.module_from_spec(spec)
spec.loader.exec_module(contact)
contact.runner.OUT = Path("E:/renderlab-work/m7-u1b")
contact.runner.OWNER = "m7-u1b"

if __name__ == "__main__":
    command = sys.argv[sys.argv.index("--") + 1:]
    if not command or "cargo" in Path(command[0]).stem.lower():
        raise SystemExit("Run the compiled executable directly; no Cargo inside a U1 GPU lease")
    try:
        raise SystemExit(contact.runner.main())
    finally:
        if contact.claimed:
            with contact.SCHEDULE.open("a", encoding="utf-8") as f:
                f.write(f"\nm7-u1b window released {contact.runner.utc()} (no owned GPU/build job remains).\n")
