"""P-RTS paired A/B captures on the frozen ARA1 scenes, run inside tools/p-rts/lease.py.

    ab.py OUT --binary base=EXE --binary cand=EXE --quality SCENE... --timing SCENE... [--reps N]

Each binary is the unchanged ARA1 Renderlab adapter (examples/ara1_capture.rs) built
from one tree. Quality scenes run once per binary; timing scenes alternate binaries for
N repetitions (fresh process each), so first-cut frames are sampled across processes.
Release runs drop inherited Vulkan validation layers, as tools/ara1/capture.py does.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parents[2]
SCENES = Path("D:/Coding/renderlab-work/ara1/phase2-scenes")
sys.path.insert(0, str(REPO / "tools/ara1"))
from lease import check_order  # noqa: E402

def check_running(expected):
    """A running P-RTS lease continues under a newer order that says it does not stop
    running leases; any other change stops it (tools/ara1/lease.py's check)."""
    from lease import order_state
    state = order_state()
    if state["sha256"] != expected and "Running leases are not stopped" in state["line"]:
        os.environ["P_RTS_EXPECTED_ORDER"] = state["sha256"]
        return state
    return check_order(expected)



def digest(path):
    with open(path, "rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


def run(binary, scene, out, extra=()):
    env = os.environ.copy()
    env.update(extra)
    removed = {k: env.pop(k) for k in ("VK_INSTANCE_LAYERS", "VK_LAYER_ENABLES") if k in env}
    out.mkdir(parents=True)
    begin = time.perf_counter()
    with (out / "process.log").open("w", encoding="utf-8") as log:
        code = subprocess.run([str(binary), str(scene), "rt", str(out)], cwd=REPO, env=env,
                              stdout=log, stderr=subprocess.STDOUT,
                              creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0)).returncode
    record = dict(binary=str(binary), binary_sha256=digest(binary), extra_env=dict(extra), scene=str(scene), scene_sha256=digest(scene),
                  exit_code=code, seconds=time.perf_counter() - begin, removed_layers=removed,
                  images={f.name: digest(f) for f in sorted(out.glob("frame-*.png"))})
    (out / "process.json").write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps(dict(out=str(out), exit_code=code, seconds=round(record["seconds"], 2))), flush=True)
    if code:
        print((out / "process.log").read_text()[-4000:], flush=True)
    return code


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("out", type=Path)
    p.add_argument("--binary", action="append", required=True)
    p.add_argument("--quality", nargs="*", default=[])
    p.add_argument("--timing", nargs="*", default=[])
    p.add_argument("--reps", type=int, default=3)
    p.add_argument("--test-exe", type=Path, help="first run these lib tests (CPU naga checks); stop on failure")
    p.add_argument("--test-filter", action="append", default=[])
    a = p.parse_args()
    # LABEL=EXE or LABEL=EXE@KEY=V;KEY=V (screen-only environment for that variant).
    binaries, extras = {}, {}
    for b in a.binary:
        label, rest = b.split("=", 1)
        exe, _, env = rest.partition("@")
        binaries[label] = exe
        extras[label] = dict(kv.split("=", 1) for kv in env.split(";") if kv)
    expected = os.environ.get("P_RTS_EXPECTED_ORDER")
    if not expected:
        raise SystemExit("run through tools/p-rts/lease.py")
    for name in a.test_filter:
        result = subprocess.run([str(a.test_exe), name, "--test-threads=1"], cwd=REPO, capture_output=True,
                                text=True, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        print(result.stdout[-6000:], result.stderr[-6000:], flush=True)
        print(json.dumps(dict(test=name, exit_code=result.returncode)), flush=True)
        if result.returncode:
            return result.returncode
    # Quality scene i, then timing scene i (alternating binaries per repetition), so a
    # lease cut short still holds complete A/B pairs. Leftovers of the longer list follow.
    steps = []
    for i in range(max(len(a.quality), len(a.timing))):
        if i < len(a.quality):
            steps += [(a.quality[i], label, 0) for label in binaries]
        if i < len(a.timing):
            steps += [(a.timing[i], label, r) for r in range(a.reps)
                      for label in (list(binaries) if r % 2 == 0 else list(binaries)[::-1])]
    failures = 0
    for scene, label, rep in steps:
        expected = check_running(expected)["sha256"]
        # A name is a frozen ARA1 scene; a .json path is a P-RTS supplemental scene.
        path = Path(scene) if scene.endswith(".json") else SCENES / f"{scene}.json"
        # The adapter keeps its pipeline cache beside OUT: one per binary, across scenes.
        failures += run(Path(binaries[label]), path, a.out / label / f"{path.stem}-{rep}", extras[label]) != 0
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
