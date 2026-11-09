"""P-FLAT lease runner (docs/p-flat.md): RD1's runner (scripts/m7-rd1-run.py) under owner p-flat,
with root-order gates for the M8 owner names. Polls in-process (50 ms) until the shared GPU
lock is free, no QUIET window or other lease is in force, no game hold applies, at least 8 GiB
is available and the latest [root order] awaits no release line, then takes one lease with
R1's guard (shared lock, reservation/release lines, game, RAM and foreign-renderer guards,
2 GiB runtime floor) and runs one guarded command. Evidence goes under
D:/Coding/renderlab-work/p-flat.

    py -3 -B scripts/p-flat-run.py LABEL MINUTES GIVE_UP_MINUTES -- powershell -NoProfile -File batch.ps1
"""
import importlib.util
import os
import re
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
OUT = Path("D:/Coding/renderlab-work/p-flat")
SCHEDULE = Path("E:/renderlab-work/gpu-schedule.md")
OWNER = "p-flat"
# Root's 11:00:47 order: 7 GiB for short non-quarry leases (full-quarry runs keep 8 GiB).
ADMISSION_GIB = float(os.environ.get("RENDERLAB_PFLAT_ADMISSION_GIB", "8"))


def read_schedule():
    while True:
        try:
            return SCHEDULE.read_text(encoding="utf-8", errors="replace")
        except PermissionError:
            time.sleep(0.2)


def root_hold(text):
    """What the latest [root order] still awaits, or None to proceed. Conservatively, every
    other owner the order names (p-*, m7-p-*) needs a later owner line, one that starts with
    the bare owner name (not a bracketed request quoting the phrase) and says released. An
    order naming p-flat as next proceeds at once."""
    # A hold relayed outside the schedule: RENDERLAB_PFLAT_AFTER_LINE="EXACT LINE TEXT" waits
    # until a schedule line starts with that text.
    literal = os.environ.get("RENDERLAB_PFLAT_AFTER_LINE")
    if literal and not re.search(rf"^{re.escape(literal)}", text, re.M):
        return literal
    # The slot root gave: RENDERLAB_PFLAT_AFTER_RE="REGEX@LINE" proceeds once an owner line
    # after schedule line LINE matches REGEX (anchored at its start), and holds on any newer
    # [root order], which may move the slot.
    explicit = os.environ.get("RENDERLAB_PFLAT_AFTER_RE")
    if explicit:
        pattern, _, line = explicit.rpartition("@")
        lines = text.splitlines()[int(line):]
        if any(l.startswith("[root order]") for l in lines):
            return "a newer [root order]"
        return None if re.search(pattern, "\n".join(lines), re.M) else pattern
    orders = list(re.finditer(r"^\[root order\]", text, re.M))
    if not orders:
        return None
    order, _, after = text[orders[-1].start():].partition("\n")
    if re.search(rf"\b{OWNER}\b[^.;:]*\bnext\b|\bnext\b[^.;]*\b{OWNER}\b|Queue[^:.;]*:\s*(?:m7-)?{OWNER}\b", order):
        return None
    names = re.findall(r"line starting '([^']+)'", order)
    names += re.findall(r"\b(?:m7-)?p-[a-z0-9]+\b", order)
    for owner in dict.fromkeys(n for n in names if n not in (OWNER, f"m7-{OWNER}")):
        if not re.search(rf"^{re.escape(owner)}\b[^\n]*released", after, re.M):
            return owner
    return None


def available_gib():
    """Available physical memory (GlobalMemoryStatusEx)."""
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
    r1.OUT, r1.OWNER = OUT, OWNER
    g = r1.guard()
    # Other units' Cargo build scripts are CPU builds, not renderers (as RD1).
    ignored = {"build-script-build.exe"}

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
    g.schedule_line(f"[{OWNER} request] owner={OWNER} asks for one <={minutes} min lease \"{label}\" "
                    f"(courtyard flat-surface speckle bisect: game_scene and matched/path-traced screens, "
                    f"both paths; prebuilt binaries, no cargo; headless); takes the lock at the next free "
                    f"moment after reading the latest root order. {g.utc().isoformat()}")
    deadline = time.time() + wait * 60
    while time.time() < deadline:
        if root_hold(read_schedule()) is None and available_gib() >= ADMISSION_GIB and not r1.LOCK.exists() \
                and not r1.schedule_read_retry(runner.quiet_active, g, any_lease=True) \
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
