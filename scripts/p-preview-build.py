"""P-PREVIEW: one Cargo command under the shared build protocol (as p-lts4-build):
outer Global\\stillkit-cargo-one-at-a-time mutex, then admission (no GPU lease lock, no
foreign compiler, >= 8 GiB available, >= 5 GB free on D:), then the atomic
D:/Coding/renderlab-work/build-slot.lock with owner JSON; request/release lines in the
shared schedule. CARGO_BUILD_JOBS=2.

    py -3 -I -B build.py LABEL CWD TARGET_DIR -- cargo ...
"""
import ctypes
import datetime
import json
import os
import re
import shutil
import subprocess as sp
import sys
import time
import uuid
from ctypes import wintypes
from pathlib import Path

OWNER = "p-preview"
SCHEDULE = Path("E:/renderlab-work/gpu-schedule.md")
SLOT = Path("D:/Coding/renderlab-work/build-slot.lock")
GPU_LOCK = Path("D:/Coding/renderlab-work/gpu-slot.lock")
OUT = Path("D:/Coding/renderlab-work/p-preview/builds")
COMPILERS = {"cargo.exe", "rustc.exe", "link.exe", "lld-link.exe", "cl.exe", "clang-cl.exe", "clippy-driver.exe"}
NO_WINDOW = 0x08000000


def utc():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def post(line):
    for _ in range(50):
        try:
            with SCHEDULE.open("a", encoding="utf-8") as f:
                f.write("\n" + line + "\n")
            break
        except PermissionError:
            time.sleep(0.2)
    print(line, flush=True)


def order():
    for _ in range(50):
        try:
            latest = None
            with SCHEDULE.open(encoding="utf-8", errors="replace") as lines:
                for line in lines:
                    if line.startswith("[root order]"):
                        latest = line.strip()
            return latest
        except PermissionError:
            time.sleep(0.2)
    raise RuntimeError("schedule unreadable")


class Memory(ctypes.Structure):
    _fields_ = [("size", wintypes.DWORD), ("load", wintypes.DWORD)] + [
        (n, ctypes.c_ulonglong) for n in ("total", "available", "pt", "pa", "vt", "va", "ext")]


def available_mib():
    m = Memory()
    m.size = ctypes.sizeof(m)
    ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(m))
    return m.available / 2**20


def compilers():
    out = sp.run(["tasklist", "/FO", "CSV", "/NH"], capture_output=True, text=True, creationflags=NO_WINDOW).stdout
    return sorted({line.split(",")[0].strip('"').lower() for line in out.splitlines()} & COMPILERS)


def pending_requests():
    # Track labels, since one owner can queue several operations before the first
    # finishes. A release of one operation must not clear its other requests.
    pending = set()
    started = False
    with SCHEDULE.open(encoding="utf-8", errors="replace") as lines:
        for line in lines:
            if line.startswith("[root order] 2026-11-10T05:19:35"):
                started = True
            if not started:
                continue
            request = re.match(r"\[(ara-pr-gate|ara-integration|root|p-s4c|p-show) request\] owner=\S+ (?:label=)?([^ :]+)", line)
            if request:
                pending.add(request.groups())
            released = re.match(r"(\S+) (\S+) window released", line)
            if released:
                owner, label = released.groups()
                if owner == "n8-virtual-shadow" and label.startswith("root-"):
                    owner, label = "root", label.removeprefix("root-")
                for key in list(pending):
                    who, task = key
                    if who == owner and (label == task or label == who+"-"+task or label.startswith(task+"-")):
                        pending.remove(key)
            for owner in ("ara-pr-gate", "ara-integration", "root", "p-s4c", "p-show"):
                if line.startswith(owner+" unit complete"):
                    pending = {key for key in pending if key[0] != owner}
    latest = order() or ""
    if "Queue: p-show" in latest:
        completed = False
        with SCHEDULE.open(encoding="utf-8", errors="replace") as lines:
            for line in lines:
                if line.strip() == latest: completed = False
                if line.startswith("p-show unit complete"): completed = True
        if not completed: pending.add(("p-show", "root-priority"))
    return pending


def main():
    label, cwd, target = sys.argv[1], Path(sys.argv[2]), Path(sys.argv[3])
    cmd = sys.argv[sys.argv.index("--") + 1:]
    OUT.mkdir(parents=True, exist_ok=True)
    kernel = ctypes.windll.kernel32
    kernel.CreateMutexW.restype = wintypes.HANDLE
    mutex = kernel.CreateMutexW(None, False, "Global\\stillkit-cargo-one-at-a-time")
    post(f"[{OWNER} request] owner={OWNER} {label}: one Cargo command, atomic build slot + stillkit mutex; >=8 GiB/5 GB; no foreign compiler; -j 2 {utc()}")
    while pending_requests() or SLOT.exists() or compilers():
        if "HOLD:" in (order() or ""):
            raise RuntimeError("root hold: no build admitted")
        time.sleep(5)
    waited = kernel.WaitForSingleObject(mutex, 45 * 60 * 1000)
    if waited not in (0, 0x80):
        post(f"{OWNER} {label} window released {utc()} (mutex wait expired; no build ran).")
        return 3
    try:
        deadline = time.monotonic() + 45 * 60
        while True:
            if pending_requests() or "HOLD:" in (order() or ""):
                raise RuntimeError("higher-priority request arrived before admission")
            busy = GPU_LOCK.exists() or compilers() or available_mib() < 8192 or shutil.disk_usage("D:/").free < 5e9
            if not busy:
                try:
                    SLOT.mkdir()
                    break
                except FileExistsError:
                    pass
            if time.monotonic() > deadline:
                post(f"{OWNER} {label} window released {utc()} (admission expired; no build ran).")
                return 3
            time.sleep(1)
        token = uuid.uuid4().hex
        (SLOT / "owner.json").write_text(json.dumps(dict(owner=OWNER, token=token, pid=os.getpid(), utc=utc(), command=cmd, root_order=order())))
        start = time.monotonic()
        try:
            env = os.environ | {"CARGO_TARGET_DIR": str(target), "CARGO_BUILD_JOBS": "2"}
            with (OUT / f"{label}.log").open("w", encoding="utf-8") as log:
                code = sp.run(cmd, cwd=cwd, env=env, stdout=log, stderr=sp.STDOUT, creationflags=NO_WINDOW).returncode
        finally:
            owner = json.loads((SLOT / "owner.json").read_text())
            assert owner["token"] == token, "build slot ownership changed"
            (SLOT / "owner.json").unlink()
            SLOT.rmdir()
        seconds = time.monotonic() - start
        (OUT / f"{label}.json").write_text(json.dumps(dict(label=label, command=cmd, cwd=str(cwd), target=str(target), exit=code, compile_seconds=seconds, utc=utc())))
        post(f"{OWNER} {label} window released {utc()} (build exit {code}, {seconds:.0f} s; no owned build remains).")
        return code
    finally:
        kernel.ReleaseMutex(mutex)
        kernel.CloseHandle(mutex)


if __name__ == "__main__":
    sys.exit(main())
