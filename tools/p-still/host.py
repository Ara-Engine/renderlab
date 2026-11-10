"""P-STILL host discipline (copied from tools/p-yard/host.py): schedule lines, root-order checks, the atomic build slot and
GPU leases through scripts/n8-run.py (<=15 min). Builds also take the stillkit mutex, only once admitted.

    py -3 -B tools/p-yard/host.py build              # release adapter, one Cargo command
    py -3 -B tools/p-yard/host.py lease LABEL MIN -- COMMAND...
"""
import importlib.util
import inspect
import json
import os
from pathlib import Path
import shutil
import subprocess as sp
import sys
import time
import uuid

REPO = Path(__file__).resolve().parents[2]
OWNER = "p-still"
WORK = Path("D:/Coding/renderlab-work/p-still")
SCHEDULE = Path("E:/renderlab-work/gpu-schedule.md")
BUILD_SLOT = Path("D:/Coding/renderlab-work/build-slot.lock")
TARGET = WORK / "target"
BINARY = WORK / "bin/p_yard_capture.exe"


def free_gib():
    """Available physical memory (GlobalMemoryStatusEx: no process snapshot, cheap under pressure)."""
    import ctypes

    class Status(ctypes.Structure):
        _fields_ = [("length", ctypes.c_ulong), ("load", ctypes.c_ulong)] + [
            (n, ctypes.c_ulonglong) for n in ("total", "avail", "page_total", "page_avail", "virt_total", "virt_avail", "ext")]
    st = Status()
    st.length = ctypes.sizeof(Status)
    ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(st))
    return st.avail / 2**30


def run_guarded(command, floor_gib=5.0, **kwargs):
    """Run COMMAND; stop its process tree if available RAM falls below FLOOR_GIB (root budget
    2026-11-10). Returns (exit code, stopped)."""
    proc = sp.Popen(command, **kwargs)
    while proc.poll() is None:
        if free_gib() < floor_gib:
            sp.run(["taskkill", "/T", "/F", "/PID", str(proc.pid)], stdout=sp.DEVNULL, stderr=sp.DEVNULL)
            proc.wait()
            return proc.returncode, True
        time.sleep(1)
    return proc.returncode, False


def post(line):
    line = f"{free_gib():.1f} GiB free | " + line  # root budget 2026-11-10: every line starts with free RAM
    with SCHEDULE.open("a", encoding="utf-8") as f:
        f.write("\n" + line + "\n")
    print(line, flush=True)


def order():
    latest = None
    with SCHEDULE.open(encoding="utf-8", errors="replace") as lines:
        for line in lines:
            if line.startswith("[root order]"):
                latest = line.strip()
    return latest or ""


def fresh(line, minutes=60):
    """A request is stale after an hour with no release: its runner died (the 15:14 memory
    emergency left ara-vk's 15:08 request open with no process alive)."""
    import datetime, re
    m = re.findall(r"20\d\d-\d\d-\d\dT\d\d:\d\d:\d\d", line)
    if not m:
        return True
    at = datetime.datetime.fromisoformat(m[-1]).replace(tzinfo=datetime.timezone.utc)
    return datetime.datetime.now(datetime.timezone.utc) - at < datetime.timedelta(minutes=minutes)


def foreign_requests():
    """Ara work goes first: an Ara request after its last release blocks us."""
    pending = {}
    with SCHEDULE.open(encoding="utf-8", errors="replace") as lines:
        for line in lines:
            for owner in ("ara-pr-gate", "ara-integration", "ara-vk", "ara-wscene", "p-show2"):
                if line.startswith(f"[{owner} request]") or line.startswith(f"[{owner} reservation]"):
                    pending[owner] = fresh(line)
                elif line.startswith(f"{owner} ") and " window released" in line:
                    pending[owner] = False
    return [o for o, p in pending.items() if p]


def runner():
    os.environ.update(N8_OWNER=OWNER, N8_OUT=str(WORK / "lease"))
    spec = importlib.util.spec_from_file_location("pstill_n8", REPO / "scripts/n8-run.py")
    n8 = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(n8)
    r = n8.s1.runner
    # As p-retime: audit every compiler, not rustc only.
    source = inspect.getsource(r.resource_sample)
    source = source.replace("count = 0", "count = 0; compilers = []")
    source = source.replace('count += entry.name.lower() == "rustc.exe"',
        'count += entry.name.lower() == "rustc.exe"\n            if entry.name.lower() in '
        '{"cargo.exe","rustc.exe","link.exe","lld-link.exe","cl.exe","clang-cl.exe","clippy-driver.exe"}: '
        'compilers.append({"pid": entry.pid, "name": entry.name})')
    source = source.replace("rustc_count=count)", "rustc_count=count, compilers=compilers)")
    scope = dict(r.__dict__)
    exec(source, scope)
    r.resource_sample = scope["resource_sample"]
    return r


# Root order 2026-11-10T15:02: builds and short leases start at 6 GiB available; this unit's
# full-quarry leases keep 7 GiB (quarry runs tripped the 2 GiB runtime guard below that).
# Root budget 2026-11-10 (sole unit): start builds and leases at 10 GiB, stop below 5 GiB.
BUILD_MIB = 10240
LEASE_MIB = 10240


def ready(r, label):
    sample = r.resource_sample()
    why = []
    if "p-still" not in order():
        why.append("root order does not list p-still")
    if "HOLD" in order() and "LIFTED" not in order():
        why.append("hold")
    if r.LOCK.exists():
        why.append("gpu slot held")
    if BUILD_SLOT.exists():
        why.append("build slot held")
    if sample["compilers"]:
        why.append("foreign compiler")
    if sample["available_mbytes"] < (BUILD_MIB if label == "build" else LEASE_MIB):
        why.append(f"available {sample['available_mbytes']:.0f} MiB")
    if shutil.disk_usage("D:/").free < 5e9:
        why.append("D: below 5 GB")
    if foreign_requests():
        why.append("pending " + ",".join(foreign_requests()))
    return why


def wait_ready(r, label, minutes=90):
    deadline = time.monotonic() + minutes * 60
    last = None
    while True:
        why = ready(r, label)
        if not why:
            return
        if why != last:
            print(time.strftime("%H:%M:%S"), label, "waiting:", why, flush=True)
            last = why
        if time.monotonic() > deadline:
            raise SystemExit("admission expired: " + repr(why))
        time.sleep(.25)


def build():
    r = runner()
    command = ["cargo", "build", "--release", "--locked", "-j", "2", "--example", "p_yard_capture", "--example", "ara1_capture"]
    WORK.mkdir(parents=True, exist_ok=True)
    post(f"[{OWNER} request] owner={OWNER} adapter: one Cargo command, atomic build slot + stillkit mutex; >=10 GiB start, stop <5 GiB/5 GB; no foreign compiler; -j 2 {r.utc()}")
    import ctypes
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.CreateMutexW.restype = ctypes.c_void_p
    kernel.WaitForSingleObject.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
    kernel.ReleaseMutex.argtypes = [ctypes.c_void_p]
    mutex = kernel.CreateMutexW(None, False, "Global\\stillkit-cargo-one-at-a-time")
    while True:
        # Admission first, so the mutex is never held while waiting on other owners.
        wait_ready(r, "build")
        if kernel.WaitForSingleObject(mutex, 0) not in (0, 0x80):  # signalled or abandoned
            time.sleep(.25)
            continue
        if ready(r, "build"):
            kernel.ReleaseMutex(mutex)
            continue
        try:
            BUILD_SLOT.mkdir()
            break
        except FileExistsError:
            kernel.ReleaseMutex(mutex)
            time.sleep(.25)
    token = uuid.uuid4().hex
    (BUILD_SLOT / "owner.json").write_text(json.dumps(dict(owner=OWNER, token=token, pid=os.getpid(), utc=r.utc(), command=command)))
    start = time.monotonic()
    code, stopped = 1, False
    try:
        with (WORK / "build.log").open("w") as log:
            code, stopped = run_guarded(command, cwd=REPO, stdout=log, stderr=sp.STDOUT,
                          env=os.environ | {"CARGO_TARGET_DIR": str(TARGET), "CARGO_BUILD_JOBS": "2", "TMP": str(WORK / "tmp"), "TEMP": str(WORK / "tmp")})
        if code == 0:
            BINARY.parent.mkdir(exist_ok=True)
            shutil.copy2(TARGET / "release/examples/p_yard_capture.exe", BINARY)
            shutil.copy2(TARGET / "release/examples/ara1_capture.exe", BINARY.parent / "ara1_capture.exe")
            commit = sp.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip()
            dirty = sp.check_output(["git", "status", "--porcelain", "--", "src", "shaders", "crates", "Cargo.toml", "Cargo.lock"],
                                    cwd=REPO, text=True).strip()
            with BINARY.open("rb") as f:
                import hashlib
                digest = hashlib.file_digest(f, "sha256").hexdigest()
            (WORK / "build.json").write_text(json.dumps(dict(base=commit, renderer_source_dirty=bool(dirty), sha256=digest,
                seconds=time.monotonic() - start, command=command, profile="release"), indent=2))
    finally:
        if json.loads((BUILD_SLOT / "owner.json").read_text())["token"] == token:
            (BUILD_SLOT / "owner.json").unlink()
            BUILD_SLOT.rmdir()
        kernel.ReleaseMutex(mutex)
        post(f"{OWNER} adapter window released {r.utc()} (build exit {code}{', stopped below 5 GiB' if stopped else ''}, {time.monotonic() - start:.0f} s; no owned build remains).")
    print((WORK / "build.log").read_text(errors="replace")[-3000:])
    return code


def lease(label, minutes, command):
    """Wait for admission, then run COMMAND under scripts/n8-run.py's guarded lease."""
    assert 0 < minutes <= 15
    r = runner()
    requeues = 0
    while True:
        wait_ready(r, label)
        env = os.environ | {"N8_OWNER": OWNER, "N8_OUT": str(WORK / "lease")}
        code = sp.run(["py", "-3", "-B", str(REPO / "scripts/n8-run.py"), "--label", label, "--minutes", str(minutes),
                       "--fast-check", "--", *command], cwd=REPO, env=env).returncode
        # run.py stops its own capture process below 5 GiB and exits 75; n8-run's finally has
        # released the lease by then. Requeue behind admission; finished steps are skipped.
        if code == 75 and requeues < 2:
            requeues += 1
            post(f"{OWNER} {label} window released {r.utc()} (stopped by its owner: available RAM below 5 GiB; requeued behind 10 GiB admission).")
            label = label + "q"
            continue
        if code != 3:
            return code
        label = label + "r"  # a deferral leaves the label's log; retry under a new one
        time.sleep(2)


if __name__ == "__main__":
    if sys.argv[1] == "build":
        raise SystemExit(build())
    if sys.argv[1] == "lease":
        i = sys.argv.index("--")
        raise SystemExit(lease(sys.argv[2], float(sys.argv[3]), sys.argv[i + 1:]))
    if sys.argv[1] == "ready":
        print(ready(runner(), "check"))
