"""Read the shared append-only GPU schedule without copying its full history."""
from pathlib import Path


def latest_order(path: Path) -> str:
    latest = None
    with path.open(encoding="utf-8", errors="replace") as lines:
        for line in lines:
            if line.startswith("[root order]"):
                latest = line.rstrip("\r\n")
    if latest is None:
        raise RuntimeError("GPU schedule has no root order")
    return latest
