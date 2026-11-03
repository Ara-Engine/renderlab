"""Preserve compiled identities and run the bounded quarry preparation comparison."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
EVIDENCE = ROOT / ".refs/quarry-preparation"
OUTPUT = ROOT / "measurements/quarry-preparation"


def digest(path):
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def sources():
    paths = [ROOT / "Cargo.toml", ROOT / "Cargo.lock"]
    paths += [p for folder in ("src", "examples", "crates/asset", "support", "shaders")
              for p in (ROOT / folder).rglob("*") if p.is_file()]
    return {str(p.relative_to(ROOT)): digest(p) for p in sorted(paths)}


def build(label, release, profile):
    binary = EVIDENCE / "bin" / f"{label}.exe"
    if binary.exists():
        raise FileExistsError(binary)
    binary.parent.mkdir(parents=True, exist_ok=True)
    before = sources()
    command = [shutil.which("cargo"), "build", "--locked", "-p", "renderlab", "--example",
               "virtual-stream", "--no-default-features", "-j", "2"]
    if release:
        command.append("--release")
    if profile:
        command += ["--features", "selection-profile"]
    with (EVIDENCE / f"build-{label}.log").open("wb") as log:
        subprocess.run(command, cwd=ROOT, stdout=log, stderr=log, check=True)
    if sources() != before:
        raise RuntimeError("sources changed during build")
    shutil.copy2(ROOT / f"target/{'release' if release else 'debug'}/examples/virtual-stream.exe", binary)
    record = {"command": command, "binary_sha256": digest(binary), "sources": before,
              "release": release, "profile": profile}
    binary.with_suffix(".json").write_text(json.dumps(record, indent=2) + "\n")
    print(f"built {label} {record['binary_sha256']}", flush=True)


def warm():
    manifest = ROOT / "measurements/quarry-scan/quarry-virtual.manifest"
    cache = ROOT / "measurements/quarry-scan/cache/virtual-v1"
    package = cache / "packages" / manifest.read_bytes()[40:72].hex()
    count = 0
    for path in [package, *sorted((cache / "pages").rglob("*"))]:
        if path.is_file():
            with path.open("rb") as file:
                while block := file.read(4 << 20):
                    count += len(block)
    return count


def run(label, binary, validate, extra):
    identity = json.loads((EVIDENCE / "bin" / f"{binary}.json").read_text())
    executable = EVIDENCE / "bin" / f"{binary}.exe"
    if digest(executable) != identity["binary_sha256"]:
        raise RuntimeError("preserved binary changed")
    output = OUTPUT / label
    if output.exists():
        raise FileExistsError(output)
    warm_bytes = warm()
    # Replay content, cameras, budgets and quality come from the original process record.
    original = json.loads((ROOT / ".refs/quarry-scan/stream-4.process.json").read_text())["command"]
    arguments = []
    i = 1
    while i < len(original):
        arg = original[i]
        if arg in ("--capture", "--output"):
            i += 2
        elif arg in ("--validate", "--verify-cover"):
            i += 1
        else:
            arguments.append(arg)
            i += 1
    if validate:
        arguments.append("--validate")
    arguments += extra + ["--output", str(output)]
    measurement = ("Offscreen quarry, explicit warm filesystem caches, equal tasklist/nvidia-smi observer; "
                   f"validation={validate}, profiling={identity['profile']}; extra={extra}")
    (EVIDENCE / f"{label}.compiled.json").write_text(json.dumps({
        "binary": binary, "identity": identity, "warm_read_bytes": warm_bytes,
        "content": {str(p.relative_to(ROOT)): digest(p) for p in [
            ROOT / "measurements/quarry-scan/quarry-virtual.manifest",
            ROOT / "measurements/quarry-scan/cameras.json", ROOT / "measurements/quarry-scan/limits.json"]},
        "label": measurement}, indent=2) + "\n")
    subprocess.run([sys.executable, str(ROOT / "scripts/scan-execute.py"), "--gpu", "--timeout", "300",
                    "--measurement-label", measurement, str(EVIDENCE / label), "--", str(executable),
                    *arguments], cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
    if validate and "gpu_validation_summary errors=0 missing_layer=0 active_layer=1" not in (
            EVIDENCE / f"{label}.stderr.log").read_text():
        raise RuntimeError("missing clean active Khronos validation")
    print(f"completed {label}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("build")
    p.add_argument("label")
    p.add_argument("--release", action="store_true")
    p.add_argument("--profile", action="store_true")
    p = sub.add_parser("run")
    p.add_argument("label")
    p.add_argument("binary")
    p.add_argument("--validate", action="store_true")
    p.add_argument("extra", nargs=argparse.REMAINDER)
    p = sub.add_parser("campaign")
    p.add_argument("tag")
    sub.add_parser("quality")
    args = parser.parse_args()
    if args.command == "build":
        build(args.label, args.release, args.profile)
    elif args.command == "campaign":
        for profile, validate in [("dev", True), ("release", False)]:
            for repeat in range(1, 6):
                for side in ("baseline", "fixed"):
                    run(f"{args.tag}-{profile}-{repeat}-{side}", f"{side}-{profile}", validate, [])
    elif args.command == "quality":
        for pose in (0, 4, 8, 12):
            for side in ("baseline", "fixed"):
                run(f"held-{pose}-{side}", f"{side}-release", True,
                    ["--start", str(pose), "--frames", "1", "--hold-pose", "--capture", str(pose), "--verify-cover"])
        for side in ("baseline", "fixed"):
            run(f"moving-4-{side}", f"{side}-release", True,
                ["--capture", ",".join(map(str, range(16))), "--verify-cover"])
        run("moving-1-fixed", "fixed-dev", True,
            ["--samples", "1", "--capture", "0,4,8,12", "--verify-cover"])
    else:
        extra = args.extra[1:] if args.extra[:1] == ["--"] else args.extra
        # REMAINDER also accepts --validate after the binary.
        validate = args.validate or "--validate" in extra
        run(args.label, args.binary, validate, [a for a in extra if a not in ("--validate", "--")])


if __name__ == "__main__":
    main()
