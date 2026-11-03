"""Run one owned scan command with source identity, logs and observed resource use."""
import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time


class ProcessMemory(ctypes.Structure):
    _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD)] + [
        (name, ctypes.c_size_t) for name in ("PeakWorkingSetSize", "WorkingSetSize",
        "QuotaPeakPagedPoolUsage", "QuotaPagedPoolUsage", "QuotaPeakNonPagedPoolUsage",
        "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage")]


def filetime(value):
    return (value.dwHighDateTime << 32) + value.dwLowDateTime


def cpu_times(handle=None):
    values = [wintypes.FILETIME() for _ in range(4 if handle else 3)]
    call = ctypes.windll.kernel32.GetProcessTimes if handle else ctypes.windll.kernel32.GetSystemTimes
    arguments = [wintypes.HANDLE(handle)] if handle else []
    if not call(*arguments, *(ctypes.byref(v) for v in values)):
        return None
    return [filetime(v) for v in values]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path)
    parser.add_argument("--gpu", action="store_true")
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--measurement-label", default="Development baseline observation with validation/capture/monitor overhead. Not controlled performance.")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    command[0] = str(Path(command[0]).absolute())
    evidence = args.evidence.resolve()
    evidence.parent.mkdir(parents=True, exist_ok=True)
    if evidence.with_suffix(".process.json").exists():
        raise FileExistsError("choose a fresh evidence label")
    env = os.environ.copy()
    scratch = Path("measurements/quarry-scan/tmp").resolve()
    scratch.mkdir(parents=True, exist_ok=True)
    env.update(TEMP=str(scratch), TMP=str(scratch), PYTHONDONTWRITEBYTECODE="1")
    if args.gpu:
        tasks = subprocess.check_output(["tasklist.exe", "/FO", "CSV"], creationflags=subprocess.CREATE_NO_WINDOW)
        if any(name in tasks.lower() for name in (b"valorant-win64-shipping", b"fortniteclient", b"eldenring")):
            raise RuntimeError("protected game active")
        if Path("D:/AraBuild/render-v3/quiet-window.lock").exists():
            raise RuntimeError("another GPU quiet window is active")
        implicit = Path("measurements/quarry-scan/empty-vulkan-implicit").resolve()
        implicit.mkdir(exist_ok=True)
        env["VK_IMPLICIT_LAYER_PATH"] = str(implicit)
    with Path(command[0]).open("rb") as binary:
        binary_hash = hashlib.file_digest(binary, "sha256").hexdigest()
    paths = sorted(p for folder in ("src", "viewer/src", "crates", "examples", "support", "shaders")
                   for p in Path(folder).rglob("*") if p.is_file())
    sources = {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}
    samples = []
    start = time.perf_counter()
    memory = ProcessMemory()
    memory.cb = ctypes.sizeof(memory)
    stop_reason = None
    host_previous = None
    with evidence.with_suffix(".stdout.log").open("wb") as stdout, evidence.with_suffix(".stderr.log").open("wb") as stderr:
        process = subprocess.Popen(command, env=env, stdout=stdout, stderr=stderr,
                                   creationflags=subprocess.CREATE_NO_WINDOW)
        while process.poll() is None:
            observer_start = time.perf_counter()
            if time.perf_counter() - start > args.timeout:
                process.kill()
                process.wait()
                stop_reason = f"owned process exceeded {args.timeout}s"
                break
            ctypes.windll.psapi.GetProcessMemoryInfo(wintypes.HANDLE(process._handle), ctypes.byref(memory), memory.cb)
            row = {"elapsed_seconds": time.perf_counter() - start,
                   "working_set_bytes": memory.WorkingSetSize,
                   "peak_working_set_bytes": memory.PeakWorkingSetSize}
            host = cpu_times()
            own = cpu_times(process._handle)
            if host and host_previous:
                idle, kernel, user = (a - b for a, b in zip(host, host_previous))
                total = kernel + user
                row["host_cpu_busy_percent"] = 100 * (total - idle) / total if total else None
            host_previous = host
            row["process_cpu_seconds"] = sum(own[2:]) / 10_000_000 if own else None
            if args.gpu:
                tasks = subprocess.check_output(["tasklist.exe", "/FO", "CSV"], creationflags=subprocess.CREATE_NO_WINDOW).lower()
                if any(name in tasks for name in (b"valorant-win64-shipping", b"fortniteclient", b"eldenring")):
                    process.kill()
                    process.wait()
                    stop_reason = "game started; stopped only the owned task process"
                    break
                row["gpu"] = subprocess.check_output(["nvidia-smi", "--query-gpu=utilization.gpu,memory.used,memory.total", "--format=csv,noheader,nounits"],
                                                     creationflags=subprocess.CREATE_NO_WINDOW).decode().strip()
            row["observer_seconds"] = time.perf_counter() - observer_start
            samples.append(row)
            time.sleep(1)
        ctypes.windll.psapi.GetProcessMemoryInfo(wintypes.HANDLE(process._handle), ctypes.byref(memory), memory.cb)
    changed = [str(p) for p in paths if hashlib.sha256(p.read_bytes()).hexdigest() != sources[str(p)]]
    times = [wintypes.FILETIME() for _ in range(4)]
    have_times = ctypes.windll.kernel32.GetProcessTimes(wintypes.HANDLE(process._handle),
                                                        *(ctypes.byref(value) for value in times))
    cpu_seconds = sum((value.dwHighDateTime << 32) + value.dwLowDateTime for value in times[2:]) / 10_000_000 if have_times else None
    report = {"command": command, "binary_sha256": binary_hash, "exit": process.returncode,
              "stop_reason": stop_reason,
              "wall_seconds": time.perf_counter() - start,
              "process_cpu_seconds": cpu_seconds,
              "peak_working_set_bytes": max([memory.PeakWorkingSetSize] + [s["peak_working_set_bytes"] for s in samples]),
              "observations": samples, "sources": sources, "changed_sources": changed,
              "VK_IMPLICIT_LAYER_PATH": env.get("VK_IMPLICIT_LAYER_PATH"),
              "measurement_label": args.measurement_label}
    evidence.with_suffix(".process.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({k: v for k, v in report.items() if k not in ("sources", "observations")}), flush=True)
    if changed or process.returncode:
        raise SystemExit(process.returncode or 1)


if __name__ == "__main__":
    main()
