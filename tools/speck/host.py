"""U-SPECK host discipline: tools/p-relit/host.py under this unit's owner, work folder and Cargo
target (schedule lines, root-order checks, the atomic build slot, GPU leases through
scripts/n8-run.py with its memory watchdog).

    py -3 -B tools/speck/host.py build NAME [TREE]   # release examples of TREE into bin/NAME
                                                     # (NAME lib-...: the lib test binary; clippy: lint)
    py -3 -B tools/speck/host.py build2 NAME         # the same in the second build slot (below)
    py -3 -B tools/speck/host.py lease LABEL MIN -- COMMAND...   # LABEL q-...: a quarry lease (14 GiB)
"""
import hashlib
import importlib.util
import inspect
import json
import os
from pathlib import Path
import shutil
import subprocess as sp
import sys
import time

HERE = Path(__file__).resolve().parent
WORK = Path("D:/Coding/renderlab-work/unit-speck")
SLOT2 = Path("D:/Coding/renderlab-work/build-slot-2.lock")
START_GIB = float(os.environ.get("SPECK_SLOT2_GIB", "12"))


def load():
    spec = importlib.util.spec_from_file_location("speck_host", HERE.parent / "p-relit/host.py")
    host = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(host)
    host.OWNER, host.WORK, host.TARGET = "u-speck", WORK, WORK / "target"
    host.EXAMPLES = ("game_scene", "reflection_matched", "p_yard_capture")
    # The lease's watchdog stops a run when the machine falls under 8 GiB free, and a
    # game-scene process takes about 2.5 GiB: start only with that much room above the floor.
    host.LEASE_MIB = 11000
    # Root's away-limits (2026-11-11): builds up to -j 8.
    source = inspect.getsource(host.build).replace('"-j", "2"', '"-j", "8"').replace('"CARGO_BUILD_JOBS": "2"', '"CARGO_BUILD_JOBS": "8"').replace("-j 2 {r.utc()}", "-j 8 {r.utc()}")
    exec(source, host.__dict__)
    # Root's ruling (2026-11-11 20:50 UTC): a picture or test lease may run while
    # ara-integration's packaging step holds build-slot.lock with no compiler running
    # (SPECK_LEASE_BESIDE_PACKAGING=1): only the held slot is ignored, a running compiler still
    # makes the lease wait. Timing leases keep the full check.
    if os.environ.get("SPECK_LEASE_BESIDE_PACKAGING") == "1":
        ready = host.ready
        host.ready = lambda r, label: [w for w in ready(r, label) if label == "build" or w != "build slot held"]
    return host


def build2(host, name):
    """Root's away-limits (2026-11-11): a second build beside the one holding build-slot.lock. It
    takes build-slot-2.lock, needs 12 GiB free at its start (10 GiB while the first slot's holder
    runs no compiler: root's ruling of 20:50 UTC, START_GIB), takes no stillkit mutex and
    ignores the other build's compilers, never overlaps a GPU lease, and stops below 6 GiB free."""
    r = host.runner()
    lib = name.startswith("lib")
    command = (["cargo", "test", "--locked", "-j", "8", "--lib", "--no-run", "--message-format=json-render-diagnostics"] if lib else
               ["cargo", "build", "--release", "--locked", "-j", "8"] + [a for e in host.EXAMPLES for a in ("--example", e)])
    (WORK / "tmp").mkdir(parents=True, exist_ok=True)
    host.post(f"[u-speck request] owner=u-speck {name} build in the second slot (root's away-limits): >={START_GIB:.0f} GiB start, stop <6 GiB, no GPU lease, -j 8 {r.utc()}")
    while True:
        if host.free_gib() >= START_GIB and not r.LOCK.exists() and not ("HOLD" in host.order() and "LIFTED" not in host.order()):
            try:
                SLOT2.mkdir()
                break
            except FileExistsError:
                pass
        time.sleep(1)
    (SLOT2 / "owner.json").write_text(json.dumps(dict(owner="u-speck", pid=os.getpid(), utc=r.utc(), label=name)))
    start = time.monotonic()
    code, stopped = 1, False
    log_path = WORK / f"build-{name}.log"
    try:
        with log_path.open("w") as log:
            code, stopped = host.run_guarded(command, floor_gib=6.0, cwd=host.REPO, stdout=log, stderr=sp.STDOUT,
                                             env=os.environ | {"CARGO_TARGET_DIR": str(host.TARGET), "CARGO_BUILD_JOBS": "8",
                                                               "TMP": str(WORK / "tmp"), "TEMP": str(WORK / "tmp")})
        if code == 0:
            out = WORK / "bin" / name
            out.mkdir(parents=True, exist_ok=True)
            digests = {}
            if lib:
                # The test binary's path is in Cargo's JSON messages.
                exe = [json.loads(l)["executable"] for l in log_path.read_text(errors="replace").splitlines()
                       if l.startswith("{") and json.loads(l).get("executable")][-1]
                shutil.copy2(exe, out / "renderlab-lib.exe")
            else:
                for e in host.EXAMPLES:
                    shutil.copy2(host.TARGET / f"release/examples/{e}.exe", out / f"{e}.exe")
                    with (out / f"{e}.exe").open("rb") as f:
                        digests[e] = hashlib.file_digest(f, "sha256").hexdigest()
            commit = sp.check_output(["git", "rev-parse", "HEAD"], cwd=host.REPO, text=True).strip()
            dirty = sp.check_output(["git", "status", "--porcelain", "--", "src", "shaders", "crates", "examples", "support", "Cargo.toml", "Cargo.lock"],
                                    cwd=host.REPO, text=True).strip()
            (out / "build.json").write_text(json.dumps(dict(base=commit, source_dirty=bool(dirty), sha256=digests,
                seconds=time.monotonic() - start, command=command, profile="test" if lib else "release"), indent=2))
    finally:
        (SLOT2 / "owner.json").unlink()
        SLOT2.rmdir()
        host.post(f"u-speck {name} build window released {r.utc()} (second slot; build exit {code}{', stopped below 6 GiB' if stopped else ''}, {time.monotonic() - start:.0f} s; no owned build remains).")
    print("\n".join(l for l in log_path.read_text(errors="replace").splitlines() if not l.startswith("{"))[-3000:])
    return code


if __name__ == "__main__":
    host = load()
    if sys.argv[1] == "build":
        raise SystemExit(host.build(*sys.argv[2:4]))
    if sys.argv[1] == "build2":
        raise SystemExit(build2(host, sys.argv[2]))
    if sys.argv[1] == "lease":
        i = sys.argv.index("--")
        raise SystemExit(host.lease(sys.argv[2], float(sys.argv[3]), sys.argv[i + 1:]))
    if sys.argv[1] == "ready":
        print(f"{host.free_gib():.1f} GiB free", host.ready(host.runner(), sys.argv[2] if len(sys.argv) > 2 else "check"))
