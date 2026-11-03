"""Sequential Blender scan jobs. Each job has a fresh process and a finite deadline."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("--ids", nargs="*")
    parser.add_argument("--export", action="store_true")
    parser.add_argument("--preview", action="store_true")
    args = parser.parse_args()
    root = args.root.resolve()
    inventory = json.loads((root / "archive-inventory.json").read_text())
    env = os.environ.copy()
    env.update(TEMP=str(root / "tmp"), TMP=str(root / "tmp"), PYTHONDONTWRITEBYTECODE="1")
    for entry in inventory["scans"]:
        if args.ids is not None and entry["id"] not in args.ids:
            continue
        if args.preview:
            tasks = subprocess.check_output(["tasklist.exe", "/FO", "CSV"], creationflags=subprocess.CREATE_NO_WINDOW).lower()
            if b"valorant-win64-shipping" in tasks or Path("D:/AraBuild/render-v3/quiet-window.lock").exists():
                raise RuntimeError("GPU work deferred for gameplay or quiet-window ownership")
        source = root / "source" / entry["source_entry"]
        if not source.exists():
            raise FileNotFoundError(source)
        output = root / "assets" / entry["id"]
        output.mkdir(parents=True, exist_ok=True)
        command = ["C:/Program Files/Blender Foundation/Blender 5.1/blender.exe",
                   "--background", "--factory-startup", "--threads", "2", "--python-exit-code", "2",
                   "--python", str(Path("scripts/scan-blender.py").resolve()), "--", str(source), str(output)]
        if args.export:
            command.append("--export")
        if args.preview:
            command.append("--preview")
        if args.export and not entry["opaque_candidate"]:
            raise ValueError("opacity asset cannot enter this opaque-only composition")
        start = time.perf_counter()
        log_path = output / ("conversion.log" if args.export else "inventory.log")
        if log_path.exists():
            log_path = log_path.with_name(log_path.stem + f"-{time.time_ns()}" + log_path.suffix)
        with log_path.open("x") as log:
            result = subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT,
                                    creationflags=subprocess.CREATE_NO_WINDOW, timeout=180)
        record = {"id": entry["id"], "command": command, "exit": result.returncode,
                  "wall_seconds": time.perf_counter() - start, "log": str(log_path)}
        with (root / "batch-jobs.jsonl").open("a") as log:
            log.write(json.dumps(record) + "\n")
        print(json.dumps(record), flush=True)
        if result.returncode:
            raise RuntimeError(f"Blender failed for {entry['id']}, see {output}")


if __name__ == "__main__":
    main()
