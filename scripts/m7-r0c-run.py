"""Run one owned CPU job under the shared slot, >=8 GiB free, <1.5 GiB.

Windows-only measurement guard. No renderer, window or GPU calls. Records
process-tree private/working-set peaks; terminates only its own child tree.
"""
import argparse
import ctypes as c
from ctypes import wintypes as w
import datetime as dt
import json
import os
from pathlib import Path
import subprocess
import sys
import time


class Memory(c.Structure):
    _fields_ = [("length", w.DWORD), ("load", w.DWORD)] + [
        (name, c.c_ulonglong) for name in
        ("total", "available", "page_total", "page_available", "virtual_total", "virtual_available", "extended")]


class Process(c.Structure):
    _fields_ = [("size", w.DWORD), ("usage", w.DWORD), ("pid", w.DWORD),
               ("heap", c.c_size_t), ("module", w.DWORD), ("threads", w.DWORD),
               ("parent", w.DWORD), ("priority", w.LONG), ("flags", w.DWORD),
               ("exe", w.WCHAR * 260)]


class Counters(c.Structure):
    _fields_ = [("size", w.DWORD), ("faults", w.DWORD)] + [
        (name, c.c_size_t) for name in ("peak_ws", "ws", "peak_pool", "pool", "peak_nonpool", "nonpool", "page", "peak_page", "private")]


class BasicLimits(c.Structure):
    _fields_ = [("process_time", c.c_longlong), ("job_time", c.c_longlong),
               ("flags", w.DWORD), ("min_ws", c.c_size_t), ("max_ws", c.c_size_t),
               ("active", w.DWORD), ("affinity", c.c_size_t), ("priority", w.DWORD), ("scheduling", w.DWORD)]


class IoCounters(c.Structure):
    _fields_ = [(name, c.c_ulonglong) for name in ("read", "write", "other", "read_bytes", "write_bytes", "other_bytes")]


class JobLimits(c.Structure):
    _fields_ = [("basic", BasicLimits), ("io", IoCounters), ("process_limit", c.c_size_t),
               ("job_limit", c.c_size_t), ("peak_process", c.c_size_t), ("peak_job", c.c_size_t)]


class JobProcesses(c.Structure):
    _fields_ = [("assigned", w.DWORD), ("count", w.DWORD), ("ids", c.c_size_t * 128)]


kernel = c.WinDLL("kernel32", use_last_error=True)
psapi = c.WinDLL("psapi", use_last_error=True)
kernel.CreateToolhelp32Snapshot.restype = w.HANDLE
kernel.OpenProcess.restype = w.HANDLE
kernel.CloseHandle.argtypes = [w.HANDLE]
kernel.Process32FirstW.argtypes = [w.HANDLE, c.POINTER(Process)]
kernel.Process32NextW.argtypes = [w.HANDLE, c.POINTER(Process)]
psapi.GetProcessMemoryInfo.argtypes = [w.HANDLE, c.POINTER(Counters), w.DWORD]
kernel.CreateJobObjectW.restype = w.HANDLE
kernel.SetInformationJobObject.argtypes = [w.HANDLE, c.c_int, c.c_void_p, w.DWORD]
kernel.AssignProcessToJobObject.argtypes = [w.HANDLE, w.HANDLE]
kernel.QueryInformationJobObject.argtypes = [w.HANDLE, c.c_int, c.c_void_p, w.DWORD, c.c_void_p]
ntdll = c.WinDLL("ntdll")
ntdll.NtResumeProcess.argtypes = [w.HANDLE]


def memory():
    m = Memory(); m.length = c.sizeof(m)
    if not kernel.GlobalMemoryStatusEx(c.byref(m)):
        raise c.WinError(c.get_last_error())
    return m.available


def processes():
    snapshot = kernel.CreateToolhelp32Snapshot(2, 0)
    if snapshot == w.HANDLE(-1).value:
        raise c.WinError(c.get_last_error())
    rows = []
    try:
        p = Process(); p.size = c.sizeof(p)
        ok = kernel.Process32FirstW(snapshot, c.byref(p))
        while ok:
            rows.append((p.pid, p.parent, p.exe))
            ok = kernel.Process32NextW(snapshot, c.byref(p))
    finally:
        kernel.CloseHandle(snapshot)
    return rows


def counters(pid):
    handle = kernel.OpenProcess(0x410, False, pid)
    if not handle:
        return None
    try:
        m = Counters(); m.size = c.sizeof(m)
        return m if psapi.GetProcessMemoryInfo(handle, c.byref(m), m.size) else None
    finally:
        kernel.CloseHandle(handle)


def cpu_time(pid):
    handle = kernel.OpenProcess(0x400, False, pid)
    if not handle:
        return 0
    try:
        values = [c.c_ulonglong() for _ in range(4)]
        if not kernel.GetProcessTimes(w.HANDLE(handle), *(c.byref(v) for v in values)):
            return 0
        return values[2].value + values[3].value
    finally:
        kernel.CloseHandle(handle)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--label", required=True)
    parser.add_argument("--light", action="store_true", help="bounded light checks: 512 MiB cap; may overlap a foreign build")
    parser.add_argument("--output", type=Path, default=Path("E:/renderlab-work/m7-r0c"))
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("a command is required")
    slot = Path("D:/Coding/renderlab-work/cpu-slot.lock")
    args.output.mkdir(parents=True, exist_ok=True)
    # Atomic acquisition: never remove or reuse someone else's slot.
    try:
        slot.mkdir()
    except FileExistsError:
        print("deferred: shared CPU slot is occupied")
        return 75
    token = f"m7-r0c-{os.getpid()}-{time.time_ns()}"
    owner = {"owner": "m7-r0c CPU only", "token": token, "pid": os.getpid(), "label": args.label}
    (slot / "owner.json").write_text(json.dumps(owner), encoding="utf-8")
    result = {"label": args.label, "command": command, "start": dt.datetime.now(dt.timezone.utc).isoformat()}
    child = None
    job = None
    try:
        available = memory()
        result["available_at_start"] = available
        rows = processes()
        conflicts = [(pid, name) for pid, _, name in rows if name.lower() in
                     ("cargo.exe", "rustc.exe", "renderlab-cook.exe")]
        # Unreal retains idle shader-worker processes after compilation. Only
        # workers consuming CPU are a competing heavy CPU job.
        shaders = {pid: cpu_time(pid) for pid, _, name in rows if name.lower() == "shadercompileworker.exe"}
        if shaders:
            time.sleep(0.25)
            conflicts.extend((pid, "active ShaderCompileWorker.exe") for pid, before in shaders.items()
                             if cpu_time(pid)-before > 100_000)
        if available < 8 * 2**30 or (conflicts and not args.light):
            result.update(deferred=True, conflicts=conflicts)
            print("deferred:", json.dumps(result))
            return 75
        start = time.monotonic()
        job = kernel.CreateJobObjectW(None, None)
        if not job:
            raise c.WinError(c.get_last_error())
        hard = JobLimits()
        hard.basic.flags = 0x200 | 0x2000  # JOB_MEMORY | KILL_ON_JOB_CLOSE
        hard.job_limit = 512*2**20 if args.light else int(1.4 * 2**30)
        if not kernel.SetInformationJobObject(job, 9, c.byref(hard), c.sizeof(hard)):
            raise c.WinError(c.get_last_error())
        peak_private = peak_ws = peak_single_private = peak_single_ws = 0
        minimum_available = available
        with (args.output / f"{args.label}.stdout").open("wb") as out, (args.output / f"{args.label}.stderr").open("wb") as err:
            child = subprocess.Popen(command, stdout=out, stderr=err,
                                     creationflags=subprocess.CREATE_NO_WINDOW | 4)
            if not kernel.AssignProcessToJobObject(job, w.HANDLE(int(child._handle))):
                child.kill()
                raise c.WinError(c.get_last_error())
            if ntdll.NtResumeProcess(w.HANDLE(int(child._handle))) != 0:
                child.kill()
                raise RuntimeError("could not resume owned CPU job")
            abort = None
            while child.poll() is None:
                active = JobProcesses()
                if not kernel.QueryInformationJobObject(job, 3, c.byref(active), c.sizeof(active), None):
                    raise c.WinError(c.get_last_error())
                owned = active.ids[:active.count]
                readings = [m for pid in owned if (m := counters(pid)) is not None]
                peak_private = max(peak_private, sum(m.private for m in readings))
                peak_ws = max(peak_ws, sum(m.ws for m in readings))
                peak_single_private = max([peak_single_private] + [m.peak_page for m in readings])
                peak_single_ws = max([peak_single_ws] + [m.peak_ws for m in readings])
                minimum_available = min(minimum_available, memory())
                if max(peak_private, peak_ws, peak_single_private, peak_single_ws) >= hard.job_limit:
                    abort = "owned job reached its conservative memory stop threshold"
                    subprocess.run(["taskkill", "/PID", str(child.pid), "/T", "/F"], capture_output=True)
                    break
                time.sleep(0.05)
            code = child.wait()
        observed_job = JobLimits()
        if not kernel.QueryInformationJobObject(job, 9, c.byref(observed_job), c.sizeof(observed_job), None):
            raise c.WinError(c.get_last_error())
        result.update(exit_code=code, abort=abort, wall_seconds=time.monotonic()-start,
                      hard_job_private_cap=hard.job_limit, peak_job_private=observed_job.peak_job,
                      peak_tree_private=peak_private, peak_tree_working_set=peak_ws,
                      peak_single_private=peak_single_private, peak_single_working_set=peak_single_ws,
                      minimum_available=minimum_available)
        print(json.dumps(result))
        return code if not abort else 76
    finally:
        if job:
            kernel.CloseHandle(job)
        (args.output / f"{args.label}.resource.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
        if json.loads((slot / "owner.json").read_text(encoding="utf-8"))["token"] == token:
            (slot / "owner.json").unlink()
            slot.rmdir()


if __name__ == "__main__":
    sys.exit(main())
