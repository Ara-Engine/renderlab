"""U-IDS merge gate, run inside one GPU lease (tools/u-ids/host.py lease ... -- py -3 -B this STEP...).

Steps, in the order given:
  tests     the lib's id tests on both paths (bin/lib-merge.exe), log and "test result" lines
  identity  game_scene with ids off (the default), the branch against main, both paths:
            every frame's RGBA8 hash compared (bin/game-ids.exe vs bin/game-main.exe)
  timing    1080p, both paths: main's binary first and last as the reference bracket, then the
            branch with ids off and on, interleaved; GPU span per frame and the "ids" pass
            from the pass timeline

Writes D:/Coding/renderlab-work/unit-ids/gate/<step>.json and prints a summary per step.
"""
import json
import os
from pathlib import Path
import statistics
import subprocess as sp
import sys

WORK = Path("D:/Coding/renderlab-work/unit-ids")
BIN = WORK / "bin"
OUT = WORK / "gate"
ACK = {"RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK": "I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK"}


def run(command, env=None, log=None):
    print(">", " ".join(map(str, command)), flush=True)
    with (open(log, "w") if log else sp.DEVNULL) as f:
        code = sp.run(list(map(str, command)), env=os.environ | ACK | (env or {}), stdout=f, stderr=sp.STDOUT).returncode
    print("  exit", code, flush=True)
    return code


def tests():
    log = OUT / "tests.log"
    code = run([BIN / "lib-merge.exe", "id_output", "object_ids", "--test-threads=1"], log=log)
    lines = [l.strip() for l in log.read_text(errors="replace").splitlines()
             if l.startswith("test result") or " corner pixels " in l or "panicked" in l]
    return {"exit": code, "lines": lines}


def game(binary, path, out, frames, every, size, env=None, timing=0):
    out.mkdir(parents=True, exist_ok=True)
    command = [BIN / binary, "--path", path, "--out", out, "--frames", frames, "--every", every,
               "--size", size, "--no-pipeline-cache"]
    if timing:
        command += ["--timing-frames", timing]
    code = run(command, env=env, log=out / "stdout.log")
    record = json.loads((out / "run.json").read_text()) if (out / "run.json").exists() else None
    return code, record


def identity():
    result = {}
    for path in ["nonrt", "rt"]:
        hashes = {}
        for side, binary in [("main", "game-main.exe"), ("ids", "game-ids.exe")]:
            code, record = game(binary, path, OUT / f"identity-{path}-{side}", 12, 1, "640x360")
            hashes[side] = [f["rgba8_sha256"] for f in record["frames"]] if record else None
            result[f"{path}-{side}-exit"] = code
        same = hashes["main"] is not None and hashes["main"] == hashes["ids"]
        differ = None if hashes["main"] is None or hashes["ids"] is None else sum(
            a != b for a, b in zip(hashes["main"], hashes["ids"]))
        result[path] = {"identical": same, "frames": len(hashes["main"] or []), "differing_frames": differ,
                        "first_main": (hashes["main"] or [None])[0], "first_ids": (hashes["ids"] or [None])[0]}
    return result


def spans(record):
    values = [s for s in (record or {}).get("gpu_render_span_ms", []) if s]
    return statistics.median(values[len(values) // 3:]) if values else None


def pass_median(log, name):
    values = []
    if log.exists():
        for line in log.read_text(errors="replace").splitlines():
            try:
                passes = json.loads(line).get("passes", {})
            except ValueError:
                continue
            if name in passes:
                values.append(passes[name])
    return (statistics.median(values[len(values) // 3:]), len(values)) if values else (None, 0)


def timing():
    result = {"runs": []}
    plan = [("reference-start", "game-main.exe", {})]
    for path in ["nonrt", "rt"]:
        for k in range(2):
            plan += [(f"{path}-off-{k}", "game-ids.exe", {}), (f"{path}-on-{k}", "game-ids.exe", {"RENDERLAB_IDS": "1"})]
    plan += [("reference-end", "game-main.exe", {})]
    for label, binary, env in plan:
        path = "nonrt" if label.startswith(("reference", "nonrt")) else "rt"
        out = OUT / f"timing-{label}"
        log = out / "passes.jsonl"
        out.mkdir(parents=True, exist_ok=True)
        if log.exists():
            log.unlink()
        code, record = game(binary, path, out, 48, 48, "1920x1080",
                            env=env | {"RENDERLAB_PASS_LOG": str(log)}, timing=0)
        ids, n = pass_median(log, "ids")
        result["runs"].append({"label": label, "path": path, "exit": code, "gpu_span_ms": spans(record),
                               "ids_pass_ms": ids, "ids_frames": n})
        print(" ", result["runs"][-1], flush=True)
    return result


if __name__ == "__main__":
    OUT.mkdir(parents=True, exist_ok=True)
    for step in sys.argv[1:]:
        value = {"tests": tests, "identity": identity, "timing": timing}[step]()
        (OUT / f"{step}.json").write_text(json.dumps(value, indent=2))
        print(step, json.dumps(value, indent=1), flush=True)
