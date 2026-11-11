"""P-RELIT lease plans for scripts/p-flat-batch.py (docs/p-relit.md): courtyard runs of the
prebuilt binaries in D:/Coding/renderlab-work/p-relit/bin/NAME (host.py build NAME).

    py -3 -B tools/p-relit/plan.py flights|hold|cases|timing|tests NAME...   # writes plans/KIND.json

flights  the dusk flight with the real fire and Fox, 48 frames, every frame written, both paths,
         for each NAME, plus the first NAME's no-history control (TR1 off).
hold     the camera held at its first pose while the scene animates (`--diag hold=1`), with the
         flicker on and steady, for each NAME, plus the first NAME's no-history control.
cases    the scene events at frame 24 (fire off, fire on, fire circling, bulb emission step,
         camera cut), for each NAME, plus the first NAME's no-history control.
timing   1080p, 150 frames of the flight with no images written and the per-pass GPU log
         (RENDERLAB_PASS_LOG), flicker on and steady, for each NAME, both paths.
tests    NAME's lib test binary (host.py build lib-NAME): the GPU acceptance tests, then the
         lib suite.
"""
import json
from pathlib import Path
import sys

WORK = Path("D:/Coding/renderlab-work/p-relit")
FOX = "D:/Coding/renderlab-work/m7-sk1/Fox.glb"
PATHS = ("rt", "nonrt")
CASES = {"fire-off": "fire=off", "fire-on": "fire=on", "fire-move": "fire=move", "glow-step": "glow=step", "cut": "cut=1"}
MANIFEST = "D:/Coding/renderlab-work/mq1/primary-manifest.json"


def game(name, run, path, args, env=None):
    out = dict(name=f"{name}-{path}-{run}", binary=name, path=path, args=args)
    if env:
        out["env"] = env
    return out


def diag(*keys):
    return [a for k in keys for a in ("--diag", k)]


def runs(kind, names):
    out = []
    first = names[0]
    if kind == "flights":
        for path in PATHS:
            out.append(game(first, "flight-control", path, ["--every", "1", "--no-temporal"]))
            out += [game(n, "flight", path, ["--every", "1"]) for n in names]
    elif kind == "hold":
        for path in PATHS:
            out.append(game(first, "hold-control", path, ["--every", "1", "--no-temporal", *diag("hold=1")]))
            for n in names:
                out.append(game(n, "hold", path, ["--every", "1", *diag("hold=1")]))
                out.append(game(n, "hold-steady", path, ["--every", "1", *diag("hold=1", "flicker=0")]))
    elif kind == "cases":
        for path in PATHS:
            for case, key in CASES.items():
                out.append(game(first, case + "-control", path, ["--every", "1", "--frames", "40", "--no-temporal", *diag(key)]))
                out += [game(n, case, path, ["--every", "1", "--frames", "40", *diag(key)]) for n in names]
    elif kind == "timing":
        for path in PATHS:
            for n in names:
                for run, keys in (("time", ()), ("time-steady", ("flicker=0",))):
                    out.append(game(n, run, path, ["--size", "1920x1080", "--frames", "150", "--every", "1000", *diag(*keys)],
                                    env={"RENDERLAB_PASS_LOG": "{out}/passes.jsonl"}))
    elif kind == "tests":
        for n in names:
            lib = f"lib-{n}"
            gpu = lambda test, env=None: dict(name=f"{n}-{test.split('::')[-1]}", binary=lib,
                                              args=[test, "--ignored", "--nocapture", "--test-threads=1"], env=env or {})
            out += [
                gpu("gpu_mq1_moving_placement", {"RENDERLAB_PRIMARY_MANIFEST": MANIFEST}),
                gpu("gpu_mv1_mq1_static_identity", {"RENDERLAB_PRIMARY_MANIFEST": MANIFEST,
                                                    "RENDERLAB_MQ1_RESOLVE": "D:/Coding/renderlab-work/m7-mv1/mq1-622db4d.wgsl"}),
                gpu("gpu_skinned_streams_history_and_paths"),
                dict(name=f"{n}-lib", binary=lib, args=["--test-threads=2"]),
            ]
    else:
        raise SystemExit(__doc__)
    return out


def main():
    kind, names = sys.argv[1], sys.argv[2:]
    binaries, common = {}, {}
    for n in names:
        binaries[n] = str(WORK / "bin" / n / "game_scene.exe")
        binaries[f"lib-{n}"] = str(WORK / "bin" / f"lib-{n}" / "renderlab-lib.exe")
        # One pipeline cache per binary: their shaders differ.
        common[n] = ["--fox", FOX, "--frames", "48", "--pipeline-cache", str(WORK / "pcache" / n)]
    plan = dict(binaries=binaries, common=common, runs=runs(kind, names), env={
        "RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK": "I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
        "VK_IMPLICIT_LAYER_PATH": "D:/Coding/renderlab-work/m7-mv1/empty-implicit",
        "TMP": str(WORK / "tmp"), "TEMP": str(WORK / "tmp")})
    if kind != "tests":
        plan["env"]["VK_INSTANCE_LAYERS"] = ""  # timings and images without the validation layer
    out = WORK / "plans" / f"{kind}-{'-'.join(names)}.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(plan, indent=1), encoding="utf-8")
    print(out, len(plan["runs"]), "runs")


if __name__ == "__main__":
    main()
