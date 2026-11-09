"""Run a P-NRT GPU plan inside tools/p-nrt/lease.py. Never run it outside a lease.

    py -3 -B tools/p-nrt/plan.py PLAN.json

PLAN.json is a list of steps, run in order; a failing step is recorded and the next
one still runs:
  {"kind": "trace", "binary": EXE, "scene": JSON, "out": DIR, "submits": 1200, "shader_profiler": true}
      Nsight GPU Trace (N8f/N8g recipe: Nsight-only implicit-layer dir, no --platform,
      200 ms / 4 submits), then the target is killed (it keeps running after ngfx).
  {"kind": "capture", "binary": EXE, "scene": JSON, "out": DIR, "mode": "nonrt"}
      ARA1 capture adapter without validation layers: engine.json timing and PNGs.
  {"kind": "build", "tree": DIR, "target": DIR, "out": LOG}
      Release ara1_capture build; only inside our own lease (build scripts trip guards).
  {"kind": "copy", "source": EXE, "out": EXE, "marker": TEXT}
      Snapshot a built exe; fails unless the embedded shader text `marker` is in it.
  {"kind": "exec", "command": [...], "env": {...}, "out": LOG}
      A test-binary step (Q-walk, RF fixtures, lib suite) without validation layers.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

NGFX = "C:/Program Files/NVIDIA Corporation/Nsight Graphics 2026.4.0/host/windows-desktop-nomad-x64/ngfx.exe"
LAYERS = "E:/renderlab-work/n8f/layers"
FLAGS = subprocess.CREATE_NO_WINDOW
REPO = Path(__file__).resolve().parents[2]


def digest(path):
    with open(path, "rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


def release_env():
    env = os.environ.copy()
    for key in ("VK_INSTANCE_LAYERS", "VK_LAYER_ENABLES"):
        env.pop(key, None)
    return env


def kill(name):
    subprocess.run(["taskkill", "/IM", name, "/F", "/T"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, creationflags=FLAGS)


def trace(step):
    out = Path(step["out"])
    out.mkdir(parents=True, exist_ok=False)
    binary = Path(step["binary"])
    env = release_env()
    env["VK_IMPLICIT_LAYER_PATH"] = LAYERS
    capture = out / "capture"
    args = f'"{step["scene"]}" {step.get("mode", "nonrt")} "{capture}"'
    command = [NGFX, "--activity", "GPU Trace Profiler", "--output-dir", str(out), "--exe", str(binary), "--dir", str(REPO),
               "--args", args, "--env", f"WGPU_DEBUG=1; RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK=I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK;",
               "--start-after-submits", str(step.get("submits", 1200)), "--limit-to-submits", str(step.get("limit", 4)),
               "--max-duration-ms", "200", "--set-gpu-clocks", "unaltered", "--collect-screenshot", "0",
               "--architecture", "Ada", "--metric-set-name", "Top-Level Triage"]
    if step.get("shader_profiler", True):
        command.append("--real-time-shader-profiler")
    start = time.monotonic()
    with (out / "ngfx.log").open("w", encoding="utf-8") as log:
        result = subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT, creationflags=FLAGS, timeout=step.get("timeout", 240))
    kill(binary.name)
    time.sleep(1)
    files = [str(f.relative_to(out)) for f in out.rglob("*") if f.is_file()]
    return dict(exit_code=result.returncode, seconds=time.monotonic() - start, files=files)


def capture(step):
    out = Path(step["out"])
    out.parent.mkdir(parents=True, exist_ok=True)
    start = time.monotonic()
    with (out.parent / f"{out.name}.log").open("w", encoding="utf-8") as log:
        result = subprocess.run([step["binary"], step["scene"], step.get("mode", "nonrt"), str(out)], cwd=REPO, env=release_env(),
                                stdout=log, stderr=subprocess.STDOUT, creationflags=FLAGS, timeout=step.get("timeout", 300))
    images = {f.name: digest(f) for f in sorted(out.glob("frame-*.png"))} if out.exists() else {}
    return dict(exit_code=result.returncode, seconds=time.monotonic() - start, images=images)


def build(step):
    # Inside our own lease only: other units' guards count our build scripts as renderers.
    env = os.environ | {"CARGO_TARGET_DIR": step["target"], "CARGO_BUILD_JOBS": "2"}
    start = time.monotonic()
    with open(step["out"], "w", encoding="utf-8") as log:
        result = subprocess.run(["cargo", "build", "--release", "--example", "ara1_capture", "-j", "2"], cwd=step["tree"], env=env,
                                stdout=log, stderr=subprocess.STDOUT, creationflags=FLAGS, timeout=step.get("timeout", 600))
    return dict(exit_code=result.returncode, seconds=time.monotonic() - start)


def copy(step):
    # Snapshot a just-built exe; `marker` must occur in it (an embedded shader line), which
    # guards against a stale binary from a shared target.
    data = Path(step["source"]).read_bytes()
    found = step["marker"].encode() in data if step.get("marker") else None
    if step.get("absent") and step["absent"].encode() in data:
        found = False
    Path(step["out"]).parent.mkdir(parents=True, exist_ok=True)
    Path(step["out"]).write_bytes(data)
    return dict(exit_code=0 if found is not False else 1, marker_found=found, sha256=digest(step["out"]))


def execute(step):
    # A test-binary step (Q-walk, RF fixtures, the lib suite): `command` with `env` added and
    # validation layers removed, output to `out`.
    env = release_env() | step.get("env", {})
    start = time.monotonic()
    Path(step["out"]).parent.mkdir(parents=True, exist_ok=True)
    with open(step["out"], "w", encoding="utf-8") as log:
        result = subprocess.run(step["command"], cwd=REPO, env=env, stdout=log, stderr=subprocess.STDOUT,
                                creationflags=FLAGS, timeout=step.get("timeout", 300))
    return dict(exit_code=result.returncode, seconds=time.monotonic() - start)


def wait(step):
    # Block (bounded) until a file another process writes exists, e.g. a candidate exe that
    # tools/p-nrt/build.sh snapshots outside the lease.
    start = time.monotonic()
    while not Path(step["path"]).exists():
        if time.monotonic() - start > step.get("timeout", 300):
            return dict(exit_code=1, seconds=time.monotonic() - start)
        time.sleep(1)
    return dict(exit_code=0, seconds=time.monotonic() - start)


def main():
    plan_path = Path(sys.argv[1])
    plan = json.loads(plan_path.read_text())
    results = []
    failed = False
    for step in plan:
        record = dict(step=step, utc=time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()))
        try:
            if "binary" in step:
                record["binary_sha256"] = digest(step["binary"])
            record |= {"trace": trace, "capture": capture, "build": build, "copy": copy, "exec": execute,
                       "wait": wait}[step["kind"]](step)
        except Exception as e:  # recorded; the next step still runs
            record["error"] = repr(e)
        failed |= record.get("exit_code") != 0
        results.append(record)
        print(json.dumps({k: record.get(k) for k in ("utc", "exit_code", "seconds", "error")} | {"out": step.get("out", step.get("path"))}), flush=True)
        plan_path.with_suffix(".results.json").write_text(json.dumps(results, indent=1) + "\n")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
