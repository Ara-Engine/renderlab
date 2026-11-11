"""U-SPECK2 host discipline: tools/p-relit/host.py under this unit's owner, work folder and Cargo
target (schedule lines, root-order checks, the atomic build slot, GPU leases through
scripts/n8-run.py with its memory watchdog).

    py -3 -B tools/speck2/host.py build NAME [TREE]   # release examples of TREE into bin/NAME
                                                      # (NAME lib-...: the lib test binary)
    py -3 -B tools/speck2/host.py build2 NAME [TREE]  # the same through the second build slot
                                                      # (root, 2026-11-11: beside a held first slot,
                                                      # >= 12 GiB free at its start, stops under 6 GiB,
                                                      # no stillkit mutex, never beside a GPU lease)
    py -3 -B tools/speck2/host.py lease LABEL MIN -- COMMAND...
    py -3 -B tools/speck2/host.py lease-picture LABEL MIN -- COMMAND...   # see beside_packaging
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
WORK = Path("D:/Coding/renderlab-work/unit-speck2")


def load():
    spec = importlib.util.spec_from_file_location("speck2_host", HERE.parent / "p-relit/host.py")
    host = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(host)
    host.OWNER, host.WORK, host.TARGET = "u-speck2", WORK, WORK / "target"
    host.EXAMPLES = ("game_scene", "reflection_matched")
    # Root's away-limits (2026-11-11): builds up to -j 8.
    source = inspect.getsource(host.build).replace('"-j", "2"', '"-j", "8"').replace('"CARGO_BUILD_JOBS": "2"', '"CARGO_BUILD_JOBS": "8"').replace("-j 2 {r.utc()}", "-j 8 {r.utc()}")
    exec(source, host.__dict__)
    return host


SLOT2 = Path("D:/Coding/renderlab-work/build-slot-2.lock")


def build2(host, name, tree=None):
    """host.build's release-example or lib-test build through the second build slot."""
    r = host.runner()
    tree = Path(tree) if tree else host.REPO
    lib = name.startswith("lib")
    command = (["cargo", "test", "--locked", "-j", "8", "--lib", "--no-run", "--message-format=json-render-diagnostics"] if lib else
               ["cargo", "build", "--release", "--locked", "-j", "8"] + [a for e in host.EXAMPLES for a in ("--example", e)])
    (WORK / "tmp").mkdir(parents=True, exist_ok=True)
    host.post(f"[u-speck2 request] owner=u-speck2 {name} build through build-slot-2: one Cargo command, -j 8; >=12 GiB start, stops <6 GiB; no GPU lease held {r.utc()}")
    last = None
    while True:
        why = [w for w, bad in (("gpu slot held", r.LOCK.exists()), ("slot 2 held", SLOT2.exists()),
                                (f"available {host.free_gib():.1f} GiB", host.free_gib() < 12.0),
                                ("hold", "HOLD" in host.order() and "LIFTED" not in host.order())) if bad]
        if not why:
            try:
                SLOT2.mkdir()
                break
            except FileExistsError:
                continue
        if why != last:
            print(time.strftime("%H:%M:%S"), "build2 waiting:", why, flush=True)
            last = why
        time.sleep(.5)
    (SLOT2 / "owner.json").write_text(json.dumps(dict(owner="u-speck2", pid=os.getpid(), utc=r.utc(), label=name)))
    start = time.monotonic()
    code, stopped = 1, False
    log_path = WORK / f"build-{name}.log"
    try:
        with log_path.open("w") as log:
            code, stopped = host.run_guarded(command, floor_gib=6.0, cwd=tree, stdout=log, stderr=sp.STDOUT,
                env=os.environ | {"CARGO_TARGET_DIR": str(host.TARGET), "CARGO_BUILD_JOBS": "8", "TMP": str(WORK / "tmp"), "TEMP": str(WORK / "tmp")})
        if code == 0:
            commit = sp.check_output(["git", "rev-parse", "HEAD"], cwd=tree, text=True).strip()
            dirty = sp.check_output(["git", "status", "--porcelain", "--", "src", "shaders", "crates", "examples", "support", "Cargo.toml", "Cargo.lock"],
                                    cwd=tree, text=True).strip()
            out = WORK / "bin" / name
            out.mkdir(parents=True, exist_ok=True)
            digests = {}
            if lib:
                exe = [json.loads(l)["executable"] for l in log_path.read_text(errors="replace").splitlines()
                       if l.startswith("{") and json.loads(l).get("executable")][-1]
                shutil.copy2(exe, out / "renderlab-lib.exe")
            else:
                for e in host.EXAMPLES:
                    shutil.copy2(host.TARGET / f"release/examples/{e}.exe", out / f"{e}.exe")
                    with (out / f"{e}.exe").open("rb") as f:
                        digests[e] = hashlib.file_digest(f, "sha256").hexdigest()
            (out / "build.json").write_text(json.dumps(dict(base=commit, source_dirty=bool(dirty), sha256=digests,
                seconds=time.monotonic() - start, command=command, profile="test" if lib else "release"), indent=2))
    finally:
        (SLOT2 / "owner.json").unlink(missing_ok=True)
        SLOT2.rmdir()
        host.post(f"u-speck2 {name} build window released {r.utc()} (build-slot-2, exit {code}{', stopped below 6 GiB' if stopped else ''}, {time.monotonic() - start:.0f} s; no owned build remains).")
    print(chr(10).join(l for l in log_path.read_text(errors="replace").splitlines() if not l.startswith("{"))[-3000:])
    return code


def beside_packaging(host):
    """Root ruling, 2026-11-11T20:47Z: while build-slot.lock is held by ara-integration's
    preview-rehearsal2 packaging step and no compiler runs (cargo.exe alone, the parent of
    `ara-dev preview`), a picture-only lease may run beside it. For that one reason only the
    lease's admission drops the build-slot and cargo.exe refusals; no timing comes from it."""
    refuse = host.ready

    def ready(r, label):
        why = refuse(r, label)
        if label == "build":
            return why
        try:
            owner = json.loads((host.BUILD_SLOT / "owner.json").read_text())
        except (OSError, ValueError):
            return why
        packaging = owner.get("owner") == "ara-integration" and owner.get("label") == "preview-rehearsal2"
        cargo_only = all(c["name"].lower() == "cargo.exe" for c in r.resource_sample()["compilers"])
        if packaging and cargo_only:
            why = [w for w in why if w not in ("build slot held", "foreign compiler")]
        return why

    host.ready = ready


if __name__ == "__main__":
    host = load()
    if sys.argv[1] == "build2":
        raise SystemExit(build2(host, *sys.argv[2:4]))
    if sys.argv[1] == "lease-picture":
        beside_packaging(host)
        i = sys.argv.index("--")
        raise SystemExit(host.lease(sys.argv[2], float(sys.argv[3]), sys.argv[i + 1:]))
    if sys.argv[1] == "build":
        raise SystemExit(host.build(*sys.argv[2:4]))
    if sys.argv[1] == "lease":
        i = sys.argv.index("--")
        raise SystemExit(host.lease(sys.argv[2], float(sys.argv[3]), sys.argv[i + 1:]))
    if sys.argv[1] == "ready":
        print(f"{host.free_gib():.1f} GiB free", host.ready(host.runner(), sys.argv[2] if len(sys.argv) > 2 else "check"))
