"""Derive local quarry limits and a held camera without changing canonical inputs."""
import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def positive(value):
    value = int(value)
    if not 1 <= value <= 2**31 - 1:
        raise argparse.ArgumentTypeError("use a finite count from 1 to 2147483647")
    return value


def write_variant(path, value):
    data = (json.dumps(value, indent=2, allow_nan=False) + "\n").encode()
    if path.exists():
        if path.read_bytes() != data:
            raise FileExistsError(f"preserve existing variant; choose another output directory: {path}")
    else:
        with path.open("xb") as handle:
            handle.write(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--limits", type=Path, default=ROOT / "measurements/quarry-scan/limits.json")
    parser.add_argument("--cameras", type=Path, default=ROOT / "measurements/quarry-scan/cameras.json")
    parser.add_argument("--output", type=Path, default=ROOT / "measurements/quarry-tuning/demo")
    parser.add_argument("--read-slots", type=positive, default=8)
    parser.add_argument("--uploads-per-tick", type=positive, default=4)
    parser.add_argument("--pose", type=int, default=0)
    args = parser.parse_args()
    output = args.output.resolve()
    if not output.is_relative_to((ROOT / "measurements").resolve()):
        parser.error("output must be inside this checkout's ignored measurements directory")
    limits_bytes = args.limits.read_bytes()
    cameras_bytes = args.cameras.read_bytes()
    limits = json.loads(limits_bytes)
    cameras = json.loads(cameras_bytes)
    if not 0 <= args.pose < len(cameras["frames"]):
        parser.error("pose is outside the camera file")
    runtime = limits.setdefault("runtime", {})
    runtime["read_slots"] = args.read_slots
    runtime["uploads_per_tick"] = args.uploads_per_tick
    limits_hash = hashlib.sha256(limits_bytes).hexdigest()
    cameras_hash = hashlib.sha256(cameras_bytes).hexdigest()
    candidate = output / f"limits-r{args.read_slots}-u{args.uploads_per_tick}-{limits_hash[:12]}.json"
    held = output / f"camera-{args.pose}-{cameras_hash[:12]}.json"
    if {candidate, held} & {args.limits.resolve(), args.cameras.resolve()}:
        parser.error("input paths cannot be variant outputs")
    output.mkdir(parents=True, exist_ok=True)
    write_variant(candidate, limits)
    write_variant(held, {"frames": [cameras["frames"][args.pose]]})
    print(json.dumps({"limits": str(candidate), "camera": str(held),
                      "baseline_sha256": limits_hash, "cameras_sha256": cameras_hash,
                      "read_slots": args.read_slots, "uploads_per_tick": args.uploads_per_tick}))


if __name__ == "__main__":
    main()
