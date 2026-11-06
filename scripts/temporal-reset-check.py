"""Run under temporal-reset-run.py: checked library exit, S3 identity, static T1a."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = Path("E:/renderlab-work/temporal-reset")
TARGET = OUT / "target"


def run(args, **env):
    start = time.monotonic()
    subprocess.run(args, cwd=ROOT, check=True, env=os.environ | env,
                   stdout=sys.stdout, stderr=subprocess.STDOUT,
                   creationflags=subprocess.CREATE_NO_WINDOW)
    return dict(command=args, exit=0, seconds=time.monotonic() - start)


def main():
    checks = []
    checks.append(run(["cargo", "fmt", "--all", "--", "--check"]))
    checks.append(run(["git", "diff", "--check"]))
    checks.append(run(["cargo", "test", "-p", "renderlab", "--lib", "-j", "2",
                       "--target-dir", str(TARGET), "--", "--test-threads=1"]))
    checks.append(run(["cargo", "clippy", "-p", "renderlab", "--lib", "--tests", "-j", "2",
                       "--target-dir", str(TARGET), "--", "-D", "warnings"]))
    binaries = list((TARGET / "debug/deps").glob("renderlab-*.exe"))
    if len(binaries) != 1:
        raise RuntimeError("ambiguous library test binary")
    binary = binaries[0]
    for test, manifest in [("gpu_motion_reset_identity", "final-s3.json"),
                           ("gpu_film_t1_temporal", "static-t1a.json")]:
        checks.append(run([str(binary), test, "--ignored", "--nocapture", "--test-threads=1"],
                          RENDERLAB_PRIMARY_MANIFEST=str(OUT / manifest)))
    sources = [*ROOT.glob("src/**/*.rs"), *ROOT.glob("shaders/**/*.wgsl"), ROOT / "Cargo.toml", ROOT / "Cargo.lock"]
    record = dict(checks=checks, binary_sha256=hashlib.file_digest(binary.open("rb"), "sha256").hexdigest(),
                  sources={str(p.relative_to(ROOT)): hashlib.file_digest(p.open("rb"), "sha256").hexdigest() for p in sources})
    (OUT / "checks.json").write_text(json.dumps(record, indent=2))


if __name__ == "__main__":
    main()
