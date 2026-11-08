"""M7-RQ1 lease runner (docs/m7-rq1.md): MQ1's runner under owner m7-rq1. Polls in-process
(50 ms) until the shared GPU lock is free, no QUIET window or lease is in force, no game hold
applies and the latest [root order] awaits no release line, then takes one lease with R1's guard
(scripts/m7-r1-run.py: the shared lock, schedule reservation/release lines, game, RAM and
foreign-renderer guards) and runs one guarded command, usually a PowerShell batch of game_scene
runs and test binaries. Evidence and lease records go under D:/Coding/renderlab-work/rq1.

    py -3 -B scripts/m7-rq1-run.py LABEL MINUTES GIVE_UP_MINUTES -- powershell -NoProfile -File batch.ps1
"""
import importlib.util
import os
import re
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
OUT = Path("D:/Coding/renderlab-work/rq1")
SCHEDULE = Path("E:/renderlab-work/gpu-schedule.md")


def root_hold():
    """The owner or label whose release line the latest [root order] still awaits, or None to
    proceed. An order that holds others names owners (m7-*) and root labels (`root LABEL`);
    each needs a later line naming it with "released". An order naming m7-rq1 next proceeds."""
    while True:
        try:
            text = SCHEDULE.read_text(encoding="utf-8", errors="replace")
            break
        except PermissionError:
            time.sleep(0.2)
    # A hold relayed outside the schedule: RENDERLAB_RQ1_AFTER=OWNER@LINE waits for OWNER's
    # "released" line after schedule line LINE.
    if os.environ.get("RENDERLAB_RQ1_AFTER"):
        owner, _, line = os.environ["RENDERLAB_RQ1_AFTER"].partition("@")
        later = "\n".join(text.splitlines()[int(line or 0):])
        if not re.search(rf"^{re.escape(owner)}\b[^\n]*released", later, re.M):
            return owner
    # RENDERLAB_RQ1_AFTER_TEXT="OWNER TEXT@LINE" waits for a line after schedule line LINE that
    # starts with OWNER and contains TEXT (quotes in other owners' lines do not count).
    if os.environ.get("RENDERLAB_RQ1_AFTER_TEXT"):
        literal, _, line = os.environ["RENDERLAB_RQ1_AFTER_TEXT"].rpartition("@")
        owner, _, rest = literal.partition(" ")
        later = "\n".join(text.splitlines()[int(line or 0):])
        if not re.search(rf"^\W*{re.escape(owner)}\b[^\n]*{re.escape(rest)}", later, re.M):
            return literal
    orders = list(re.finditer(r"^\[root order\]", text, re.M))
    if not orders:
        return None
    start = orders[-1].start()
    order, _, after = text[start:].partition("\n")
    if re.search(r"m7-rq1[^.;]*\bnext\b|\bnext\b[^.;]*m7-rq1", order):
        return None
    if "hold" not in order:
        return None
    names = [n for n in re.findall(r"\bm7-[a-z0-9-]*[a-z0-9]", order) if n != "m7-rq1"]
    names += re.findall(r"\broot ([a-z][\w-]*-[\w-]+)", order)
    for name in names:
        if not re.search(rf"^[^\n]*\b{re.escape(name)}\b[^\n]*released", after, re.M):
            return name
    return None


def available_gib():
    """Available physical memory (GlobalMemoryStatusEx): leases start only with headroom,
    because the shared guard stops them below 2 GiB while other units compile."""
    import ctypes

    class Status(ctypes.Structure):
        _fields_ = [("length", ctypes.c_ulong), ("load", ctypes.c_ulong)] + [
            (name, ctypes.c_ulonglong) for name in
            ("total", "available", "page_total", "page_free", "virtual_total", "virtual_free", "extended")]
    status = Status()
    status.length = ctypes.sizeof(Status)
    ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status))
    return status.available / 2**30


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), REPO / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main():
    label, minutes, wait = sys.argv[1], int(sys.argv[2]), float(sys.argv[3])
    command = sys.argv[sys.argv.index("--") + 1:]
    r1 = load("m7-r1-run")
    r1.OUT, r1.OWNER = OUT, "m7-rq1"
    g = r1.guard()
    # Other units' Cargo build scripts (build-script-build.exe under renderlab-work) are CPU
    # builds, not renderers; the generic guard's path rule stops leases on them. Functional
    # (image-only, untimed) leases with RENDERLAB_RQ1_FUNCTIONAL=1 also ignore the Ara
    # integration unit's ara1-ara.exe, which runs outside the lease protocol: foreign GPU work
    # cannot change RQ1's images, only its timings.
    ignored = {"build-script-build.exe"}
    if os.environ.get("RENDERLAB_RQ1_FUNCTIONAL") == "1":
        ignored.add("ara1-ara.exe")

    def without_build_scripts(source):
        def state(*a, **k):
            value = source(*a, **k)
            value["foreign_renderers"] = [p for p in value.get("foreign_renderers", [])
                                          if p["Name"].lower() not in ignored]
            return value
        return state
    g.E2E.state = without_build_scripts(g.E2E.state)
    if getattr(g, "MONITOR_STATE", None):
        g.MONITOR_STATE = without_build_scripts(g.MONITOR_STATE)
    runner = load("m7-g1-run")
    env = dict(os.environ, RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
    g.schedule_line(f"[m7-rq1 request] owner=m7-rq1 asks for one <={minutes} min lease \"{label}\" "
                    f"(game_scene reflection screens, references and RF captures, both paths; headless); "
                    f"takes the lock at the next free moment. {g.utc().isoformat()}")
    deadline = time.time() + wait * 60
    while time.time() < deadline:
        if root_hold() is None and available_gib() >= 6.0 and not r1.LOCK.exists() and not r1.schedule_read_retry(runner.quiet_active, g, any_lease=True) \
                and r1.schedule_read_retry(runner.hold_state, g) != "hold":
            try:
                with g.lease(label, minutes) as owner:
                    out = OUT / "lease" / label
                    g.guarded(command, out, timeout=max(60, int(g.remaining_seconds(owner)) - 20), cwd=REPO, env=env)
                    print(f"{label} done; log {out / 'stdout.log'}", flush=True)
                return 0
            except SystemExit as e:
                if e.code != 3:
                    raise
                print(f"{label} deferred; polling again", flush=True)
            except RuntimeError as e:
                print(f"{label} failed: {e}", flush=True)
                return 1
        time.sleep(0.05)
    print("lease wait timed out", flush=True)
    return 3


if __name__ == "__main__":
    sys.exit(main())
