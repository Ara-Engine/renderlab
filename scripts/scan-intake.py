"""Private scan archive inventory and bounded, verified extraction. Uses Python stdlib."""
from __future__ import annotations

import argparse
from dataclasses import asdict, dataclass
import hashlib
import json
from pathlib import Path, PurePosixPath
import shutil
import zipfile


@dataclass
class ScanEntry:
    id: str
    source_entry: str
    source_bytes: int
    texture_maps: list[dict]
    opaque_candidate: bool


def digest(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("root", type=Path)
    parser.add_argument("--extract", nargs="*", default=[])
    parser.add_argument("--all-geometry", action="store_true")
    args = parser.parse_args()
    root = args.root.resolve()
    root.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(args.archive) as archive:
        members = archive.infolist()
        scans = []
        for member in members:
            path = PurePosixPath(member.filename)
            if path.suffix.lower() != ".fbx":
                continue
            maps = [{"entry": m.filename, "bytes": m.file_size,
                     "channel": PurePosixPath(m.filename).stem.split("_")[-1]}
                    for m in members if PurePosixPath(m.filename).parent == path.parent
                    and PurePosixPath(m.filename).suffix.lower() in (".jpg", ".png")]
            scans.append(asdict(ScanEntry(path.stem, member.filename, member.file_size,
                                         maps, not any(m["channel"].lower() == "opacity" for m in maps))))
        (root / "archive-inventory.json").write_text(json.dumps({
            "archive": str(args.archive.resolve()), "archive_bytes": args.archive.stat().st_size,
            "entries": len(members), "scans": scans,
        }, indent=2) + "\n")
        selected = set(args.extract)
        directories = {PurePosixPath(s["source_entry"]).parent.as_posix()
                       for s in scans if s["id"] in selected or args.all_geometry}
        for member in members:
            relative = PurePosixPath(member.filename)
            if relative.parent.as_posix() not in directories or member.is_dir():
                continue
            if args.all_geometry and relative.suffix.lower() != ".fbx" and relative.parent.as_posix() not in {
                    PurePosixPath(s["source_entry"]).parent.as_posix() for s in scans if s["id"] in selected}:
                continue
            if relative.is_absolute() or ".." in relative.parts or ":" in member.filename or "\\" in member.filename:
                raise ValueError(f"unsafe archive entry {member.filename!r}")
            destination = (root / "source" / Path(*relative.parts)).resolve()
            if not destination.is_relative_to(root / "source"):
                raise ValueError("archive destination escaped source root")
            if destination.exists():
                if destination.stat().st_size != member.file_size:
                    raise ValueError(f"existing input size mismatch {destination}")
            else:
                destination.parent.mkdir(parents=True, exist_ok=True)
                with archive.open(member) as source, destination.open("xb") as output:
                    shutil.copyfileobj(source, output, 1024 * 1024)
            # Compare archive bytes as well as extracted bytes. Existing files never establish identity.
            with archive.open(member) as source:
                expected = hashlib.file_digest(source, "sha256").hexdigest()
            actual = digest(destination)
            if actual != expected:
                raise ValueError(f"archive hash mismatch {destination}")
            record = {"entry": member.filename, "bytes": member.file_size, "sha256": actual}
            with (root / "source-hashes.jsonl").open("a") as output:
                output.write(json.dumps(record) + "\n")
            print(json.dumps(record), flush=True)


if __name__ == "__main__":
    main()
