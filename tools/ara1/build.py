"""Build external measurement adapters, never modifying an Ara checkout.

All targets, locks, profiles and caches are placed beneath --root. Two Cargo jobs.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
import tomllib

REPO = Path(__file__).resolve().parents[2]
DEFAULT_ROOT = Path("D:/Coding/renderlab-work/ara1")


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args], text=True).strip()


def build_ara(source, root, label, check=False, release=False, jobs=2):
    project = root / ("adapter-" + label)
    project.mkdir(parents=True, exist_ok=True)
    deps = {name: source / "crates" / name for name in ("ara-render", "ara-gpu", "ara-render-fidelity")}
    deps["renderlab-asset"] = REPO / "crates/asset"
    manifest = ['[package]', 'name = "ara1-ara"', 'version = "0.0.0"', 'edition = "2024"',
                '[workspace]', '[[bin]]', 'name = "ara1-ara"',
                'path = ' + json.dumps((REPO / "tools/ara1/ara.rs").as_posix()), '[dependencies]']
    manifest.extend(name + ' = { path = ' + json.dumps(path.as_posix()) + ' }' for name, path in deps.items())
    manifest += ['anyhow = "1"', 'serde = { version = "1", features = ["derive"] }', 'serde_json = "1"',
                 'glam = { version = "0.30", features = ["serde", "bytemuck"] }',
                 'sha2 = "0.10"', 'bytemuck = "1"', 'image = { version = "0.25", default-features = false, features = ["png", "hdr", "jpeg"] }',
                 'gltf = { version = "1.4", features = ["import"] }',
                 '[profile.dev]', 'debug = 0', '[profile.release]', 'debug = 0']
    (project / "Cargo.toml").write_text("\n".join(manifest) + "\n")
    cfg = 'println!("cargo::rustc-cfg=ara1_v3");' if label=="ara-v3" else ''
    (project / "build.rs").write_text('fn main() { println!("cargo::rustc-check-cfg=cfg(ara1_v3)"); ' + cfg + ' }\n')
    (project / "sample.rs").write_text('#[allow(dead_code)]\n#[path = ' + json.dumps((source / "crates/ara-render/examples/common/mod.rs").as_posix()) + ']\nmod common;\n')
    assets = project / "examples/assets"
    if not assets.exists():
        assets.parent.mkdir(exist_ok=True)
        # A directory junction reads original sample media without copying or writing it.
        subprocess.run(["powershell", "-NoProfile", "-Command",
                        "New-Item -ItemType Junction -Path $env:ARA1_LINK_PATH -Target $env:ARA1_LINK_TARGET | Out-Null"],
                       env=os.environ | {"ARA1_LINK_PATH":str(assets), "ARA1_LINK_TARGET":str(source / "crates/ara-render/examples/assets")}, check=True)
    env = os.environ | {"CARGO_TARGET_DIR": str(root / ("target-" + label)), "CARGO_BUILD_JOBS": str(jobs),
                        "ARA_CONFIG_DIR": str(root / "profiles" / label), "ARA_CRASH_DIR": str(root / "crashes" / label)}
    channel = tomllib.loads((source / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    command = ["cargo", "+" + channel, "check" if check else "build", "--manifest-path", str(project / "Cargo.toml"), "-j", str(jobs)]
    if release:command.append("--release")
    return run_build(command, env, source, root, label)


def run_build(command, env, source, root, label):
    begin = time.perf_counter()
    before = git(source, "status", "--porcelain", "--untracked-files=no")
    verb="check" if "check" in command else "build"
    log = root / (verb + "-" + label + ".log")
    with log.open("w", encoding="utf-8") as stream:
        result = subprocess.run(command, cwd=REPO, env=env, stdout=stream, stderr=subprocess.STDOUT,
                                creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    record = dict(command=command, source=str(source), source_sha=git(source, "rev-parse", "HEAD"),
                  source_dirty_tracked=bool(before), seconds=time.perf_counter()-begin, exit_code=result.returncode,
                  target=env["CARGO_TARGET_DIR"], cargo_jobs=int(env["CARGO_BUILD_JOBS"]),
                  adapter_sources={str(p.relative_to(REPO)):hashlib.sha256(p.read_bytes()).hexdigest() for p in
                      sorted([*(REPO/"tools/ara1").glob("*.rs"),*(REPO/"examples").glob("ara1_*.rs")])},
                  tracked_status_unchanged=before == git(source, "status", "--porcelain", "--untracked-files=no"))
    (root / (verb + "-" + label + ".json")).write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps(record), flush=True)
    if result.returncode:
        print(log.read_text(encoding="utf-8")[-12000:], flush=True)
    if not record["tracked_status_unchanged"]:
        raise RuntimeError("source status changed during build; inspect before proceeding")
    return result.returncode


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("engine", choices=["renderlab", "ara-main", "ara-v3"])
    p.add_argument("--root", type=Path, default=DEFAULT_ROOT)
    p.add_argument("--ara-source", type=Path)
    p.add_argument("--check", action="store_true")
    p.add_argument("--release", action="store_true")
    p.add_argument("--jobs", type=int, choices=[1,2], default=2)
    a = p.parse_args()
    a.root.mkdir(parents=True, exist_ok=True)
    if a.engine == "renderlab":
        command = ["cargo", "check" if a.check else "build", "--example", "ara1_capture", "--example", "ara1_prepare", "-j", str(a.jobs)]
        if a.release:command.append("--release")
        return run_build(command, os.environ | {"CARGO_TARGET_DIR": str(a.root / "target-renderlab"), "CARGO_BUILD_JOBS": str(a.jobs)}, REPO, a.root, "renderlab")
    source = a.ara_source or (a.root / "ara-main" if a.engine == "ara-main" else Path("C:/Users/snowy/.t3/worktrees/snengine/t3-9c4a1f92"))
    return build_ara(source.resolve(), a.root, a.engine, a.check, a.release, a.jobs)


if __name__ == "__main__":
    raise SystemExit(main())
