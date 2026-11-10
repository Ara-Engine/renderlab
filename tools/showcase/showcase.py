"""Ara's Showcase scene as a standing Renderlab test scene (P-SHOW, docs/p-show.md).

Showcase is drawn by Ara's player through Ara's Renderlab bridge (`ara-renderlab`), with
Renderlab patched to this checkout. No Ara code or content is copied into this repository:
Ara's sample assets are fetched by Ara's own `ara-samples` from their pinned public sources
(Poly Haven CC0 and Khronos glTF sample models, see Ara's samples/THIRD_PARTY_ASSETS.md).

    py -3 -I tools/showcase/showcase.py prepare --ara D:/Coding/projects/desktop/snengine --rev <sha>
    py -3 -I tools/showcase/showcase.py build          # wrap in the shared build slot
    py -3 -I tools/showcase/showcase.py fetch          # Ara's sample fetch and kits (once)
    py -3 -I tools/showcase/showcase.py capture rt still-rt -- --stability 60   # inside a GPU lease
    py -3 -I tools/showcase/showcase.py summary

Everything (worktree, Cargo target, captures) lives under --root (default
D:/Coding/renderlab-work/showcase). `capture` prints the per-frame GPU span that Ara's
`capture --stability` reports for the bridge (Renderlab's `GpuSample::render_span`).

P-SHOW2 (docs/p-show2.md): `capture` also sets RENDERLAB_PASS_LOG, Renderlab's named per-stage
GPU timeline (src/pass_timeline.rs), and adds the medians over the measured frames (the last
`--stability N` or `--video N` frames) to the capture JSON as `renderlab_passes`. `-e K=V`
passes Renderlab measurement switches, e.g. `-e RENDERLAB_ABLATE=reflections`.
"""
import argparse
import json
import os
from pathlib import Path
import re
import statistics
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parents[2]
DEFAULT_ROOT = Path("D:/Coding/renderlab-work/showcase")
# Ara's measurement of PR #4 locked exposure at this EV100 for every renderer.
EV100 = "10.17"
RENDERERS = {"rt": "renderlab-rt", "nonrt": "renderlab-nonrt", "ara": "ara"}


def run(cmd, cwd=None, env=None):
    print("+", " ".join(map(str, cmd)), flush=True)
    return subprocess.call([str(c) for c in cmd], cwd=cwd, env=env)


def prepare(args):
    tree = args.root / "ara"
    if not tree.exists():
        if run(["git", "-C", args.ara, "worktree", "add", "--detach", tree, args.rev]):
            return 1
    manifest = tree / "crates/ara-renderlab/Cargo.toml"
    text = manifest.read_text(encoding="utf-8")
    patched = re.sub(r"^renderlab = \{[^\n]*\}$",
                     "renderlab = { path = " + json.dumps(REPO.as_posix()) + " }", text, flags=re.M)
    if patched == text and REPO.as_posix() not in text:
        print("ara-renderlab's renderlab dependency line was not found", file=sys.stderr)
        return 1
    manifest.write_bytes(patched.encode("utf-8"))
    print(f"Ara worktree {tree} draws with Renderlab at {REPO}")
    return 0


def cargo_env(args):
    return os.environ | {"CARGO_TARGET_DIR": str(args.root / "target"), "CARGO_BUILD_JOBS": "2"}


def build(args):
    # perf: Ara's optimized iteration profile (release opt-level, no debug assertions).
    return run(["cargo", "build", "--profile", args.profile, "-p", "ara", "-p", "ara-player",
                "-p", "ara-import-worker", "-p", "ara-samples"], cwd=args.root / "ara", env=cargo_env(args))


def binary(args, name):
    return args.root / "target" / args.profile / (name + ".exe")


def fetch(args):
    tree = args.root / "ara"
    for step in (["fetch", "showcase"], ["kits", "showcase"]):
        if run([binary(args, "ara-samples"), *step], cwd=tree):
            return 1
    return 0


def capture(args):
    out = args.root / "captures"
    out.mkdir(parents=True, exist_ok=True)
    env = os.environ | {
        "ARA_RENDERER": RENDERERS[args.path],
        "ARA_CONFIG_DIR": str(args.root / "profile/config"),
        "ARA_CRASH_DIR": str(args.root / "profile/crashes"),
        # Ara's PR #4 numbers were taken with wgpu's debug-build validation off.
        "WGPU_VALIDATION": "0",
        "WGPU_DEBUG": "0",
        "RENDERLAB_PASS_LOG": str(out / f"{args.name}.passes.jsonl"),
    } | dict(kv.split("=", 1) for kv in args.env)
    extra = args.extra or ["--stability", "60"]
    started = time.monotonic()
    with (out / f"{args.name}.log").open("w", encoding="utf-8") as log:
        code = subprocess.call([str(binary(args, "ara")), "capture", "--camera", "Camera",
                                "--path", str(out / f"{args.name}.png"), "--width", "1920",
                                "--height", "1080", "--ev100", EV100, *extra],
                               cwd=args.root / "ara/samples/showcase", env=env,
                               stdout=log, stderr=subprocess.STDOUT)
    record_passes(out / f"{args.name}.json", out / f"{args.name}.passes.jsonl", measured_frames(extra), args.env)
    print(f"{args.name}: exit {code} after {time.monotonic() - started:.0f} s; {spans(out / f'{args.name}.json')}")
    print("   ", passes_line(out / f"{args.name}.json"))
    return code


def measured_frames(extra):
    """The frames `ara capture` measures: the last --stability N, or the --video N frames."""
    for flag in ("--stability", "--video"):
        if flag in extra:
            return int(extra[extra.index(flag) + 1])
    return 60


def record_passes(capture, log, frames, env):
    """Medians of the timeline's last `frames` frames, into the capture JSON."""
    try:
        lines = [json.loads(l) for l in log.read_text(encoding="utf-8").splitlines() if l.strip()]
    except (OSError, ValueError):
        return
    lines = lines[-frames:]
    if not lines:
        return
    names = list(dict.fromkeys(n for l in lines for n in l["passes"]))
    passes = {n: round(statistics.median(l["passes"].get(n, 0.0) for l in lines), 4) for n in names}
    record = dict(frames=len(lines), span=round(statistics.median(l["span"] for l in lines), 4),
                  passes=passes, env=env)
    try:
        data = json.loads(capture.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        data = {}
    data["renderlab_passes"] = record
    capture.write_text(json.dumps(data, indent=1), encoding="utf-8")


def passes_line(path):
    try:
        r = json.loads(path.read_text(encoding="utf-8"))["renderlab_passes"]
    except (OSError, ValueError, KeyError):
        return "no pass timeline"
    body = " ".join(f"{k}={v:.2f}" for k, v in r["passes"].items() if v >= 0.05)
    return f"timeline span={r['span']:.2f} ({r['frames']} frames): {body}"


def spans(path):
    try:
        timings = json.loads(path.read_text(encoding="utf-8"))["timings"]
    except (OSError, ValueError, KeyError):
        return "no timings"
    external = {t["pass"]: t["milliseconds"] for t in timings if t["pass"].startswith("External.")}
    if external:
        return " ".join(f"{k.removeprefix('External.')}={v:.2f}" for k, v in external.items())
    return f"Ara passes sum {sum(t['milliseconds'] for t in timings):.2f} ms"


def summary(args):
    for path in sorted((args.root / "captures").glob("*.json")):
        if not path.name.endswith(".legend.json"):
            print(f"{path.stem:32} {spans(path)}")
            print(f"{'':32} {passes_line(path)}")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--root", type=Path, default=DEFAULT_ROOT)
    parser.add_argument("--profile", default="perf", help="Ara's Cargo profile: perf, or release to share an sccache")
    commands = parser.add_subparsers(dest="command", required=True)
    p = commands.add_parser("prepare")
    p.add_argument("--ara", type=Path, required=True, help="an Ara (snengine) clone")
    p.add_argument("--rev", required=True, help="Ara commit with the Renderlab bridge")
    commands.add_parser("build")
    commands.add_parser("fetch")
    c = commands.add_parser("capture")
    c.add_argument("path", choices=sorted(RENDERERS))
    c.add_argument("name")
    c.add_argument("-e", "--env", action="append", default=[], help="K=V for Renderlab measurement switches")
    c.add_argument("extra", nargs=argparse.REMAINDER, help="after --: more `ara capture` arguments")
    commands.add_parser("summary")
    args = parser.parse_args()
    if getattr(args, "extra", None) and args.extra[0] == "--":
        args.extra = args.extra[1:]
    return {"prepare": prepare, "build": build, "fetch": fetch, "capture": capture, "summary": summary}[args.command](args)


if __name__ == "__main__":
    raise SystemExit(main())
