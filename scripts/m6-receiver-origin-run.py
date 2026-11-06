"""Guarded offscreen receiver-origin verification; each lease is at most 45 minutes."""
import importlib.util
import hashlib
import datetime
from pathlib import Path

spec = importlib.util.spec_from_file_location("film_run", Path(__file__).with_name("m6-film-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
runner.OUT = Path("E:/renderlab-work/receiver-origin")
runner.OWNER = "receiver-origin"
runner.PREFLIGHT_MEMORY_MBYTES = 4096
runner.RUNTIME_MEMORY_MBYTES = 2048

# Dynamic WGSL controls must remain attributable after their scratch files change.
original_save = runner.save
reservation = None


def save_with_shader_identity(path, value):
    global reservation
    if path == runner.LOCK / "owner.json" and value.get("owner") == runner.OWNER:
        reservation = value["utc"]
        schedule = Path("E:/renderlab-work/gpu-schedule.md")
        end = datetime.datetime.fromisoformat(reservation) + datetime.timedelta(minutes=value["expected_minutes"])
        lines = [line for line in schedule.read_text().splitlines() if not line.startswith("[receiver-origin reservation]")]
        schedule.write_text("\n".join(lines) + f"\n[receiver-origin reservation] owner=receiver-origin end={end.isoformat()}\n")
    if isinstance(value, dict) and "command" in value and isinstance(value.get("manifest"), dict):
        hashes = {}
        for key in ("footprint_source", "film_frozen_modules", "candidate_modules", "frozen_modules"):
            name = value["manifest"].get(key)
            if name:
                target = Path(name)
                for shader in target.glob("*.wgsl") if target.is_dir() else (target,):
                    if shader.is_file():
                        hashes[str(shader)] = hashlib.sha256(shader.read_bytes()).hexdigest()
        value["shader_sha256"] = hashes
    original_save(path, value)


runner.save = save_with_shader_identity

if __name__ == "__main__":
    try:
        raise SystemExit(runner.main())
    finally:
        if reservation is not None:
            schedule = Path("E:/renderlab-work/gpu-schedule.md")
            lines = [line for line in schedule.read_text().splitlines() if not line.startswith(("[receiver-origin reservation]", "receiver request owner=receiver-origin"))]
            schedule.write_text("\n".join(lines) + f"\nreceiver window released {runner.utc()} (receiver-origin; no owned GPU job remains).\n")
