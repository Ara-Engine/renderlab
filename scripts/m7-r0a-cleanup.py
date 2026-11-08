"""Bounded cleanup of R0a-owned, summarized measurement data."""
import argparse
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--final", action="store_true")
    parser.add_argument("--release-target", action="store_true")
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location("r0a", Path(__file__).with_name("m7-r0a.py"))
    r = importlib.util.module_from_spec(spec); spec.loader.exec_module(r)
    r.cpu_note("hash/prune only own superseded data"+(" and verified Cargo target" if args.final else "")); r.no_quiet()
    root = r.OUT.resolve()
    if root!=Path("E:/renderlab-work/m7-r0a").resolve():
        raise ValueError("unexpected cleanup root")
    ledger_path = root/"cleanup-ledger.json"
    ledger = json.loads(ledger_path.read_text()) if ledger_path.exists() else {}

    def remove_file(path, reason):
        if not path.resolve().is_relative_to(root):
            raise ValueError("cleanup escaped own root")
        ledger[str(path.relative_to(root))] = dict(bytes=path.stat().st_size,sha256=r.sha(path),reason=reason)
        r.save(ledger_path,ledger)
        path.unlink()

    old = root/"ue-package-b"
    if old.exists():
        identity = json.loads((root/"cook/c/identity.json").read_text())
        for name,digest in identity["content"].items():
            if r.sha(root/"ue-package-c"/name)!=digest:
                raise ValueError("package C is not retained intact")
        for path in old.rglob("*"):
            if path.is_file():
                ledger[str(path.relative_to(root))] = dict(bytes=path.stat().st_size,sha256=r.sha(path),reason="superseded B; corrected C verified")
        r.save(ledger_path,ledger)
        if old.resolve()!=Path("E:/renderlab-work/m7-r0a/ue-package-b").resolve():
            raise ValueError("unexpected obsolete package path")
        shutil.rmtree(old)
    for name in ("capture/b", "motion/capture/b", "asprobe", "reference/m-ordered-before-overlap", "motion/capture-before-overlap", "motion/reference-before-overlap", "current/b-motion-before-overlap"):
        for path in (root/name).rglob("*.npz"):
            if path.stat().st_size>5*1024**2:
                # These are superseded matrices or repeated AS diagnostic dumps;
                # their JSON statistics, hash proofs and rendered PNGs survive.
                remove_file(path,"superseded capture or summarized repeated-AS diagnostic")
    if args.final:
        query = r"@(Get-CimInstance Win32_Process | Where-Object { $_.Name -eq 'python.exe' -and $_.CommandLine -match 'm7-r0a\.py' } | Select-Object ProcessId,CommandLine) | ConvertTo-Json -Compress"
        controllers = subprocess.run(["powershell", "-NoProfile", "-Command", query],
            capture_output=True,text=True,check=True,timeout=30,creationflags=subprocess.CREATE_NO_WINDOW)
        if json.loads(controllers.stdout or "[]"):
            raise ValueError("final cleanup requires every owned controller to exit, including queued jobs")
        if (live := r.owner_now()) and live.get("owner")=="m7-r0a":
            raise ValueError("final cleanup requires all owned GPU controllers to finish")
        static = json.loads((root/"scores/c-static.json").read_text())
        motion = json.loads((root/"scores/c-motion.json").read_text())
        audit = json.loads((root/"qa/c/visual-audit.json").read_text())
        if not static["complete"] or len(static["rows"])!=130 or not motion["complete"] or len(motion["rows"])!=10 or not audit["complete"]:
            raise ValueError("final score/inspection coverage incomplete")
        for name in ("reference", "asprobe", "compatibility"):
            r.cleanup_raw(root/name)
        producers = {Path(value["source_archive"]).resolve()
            for value in json.loads((root/"reference/final/manifest.json").read_text())["cases"].values()}
        for path in (root/"reference").rglob("*.npz"):
            canonical_static = path.is_relative_to(root/"reference/final") or path.resolve() in producers
            canonical_motion = path.is_relative_to(root/"reference/m-ordered") and path.parent.name=="4096" and path.parent.parent.name.endswith("-zero")
            if not canonical_static and not canonical_motion and path.stat().st_size>5*1024**2:
                remove_file(path,"summarized diagnostic or producer archive; canonical reference inputs retained separately")
        for name in ("capture/c", "motion/capture/c", "current"):
            for path in (root/name).rglob("*.npz"):
                if path.stat().st_size>5*1024**2:
                    remove_file(path,"scored and inspected final capture; retained summaries/PNGs/reference inputs")
    if args.final or args.release_target:
        checks = json.loads((root/"checks.json").read_text())
        if checks["cargo_exit"] or checks["fmt_exit"] or any(r.sha(r.REPO/name)!=digest for name,digest in checks["rust_sources"].items()):
            raise ValueError("Rust checks no longer match source")
        target = root/"target"
        if target.exists():
            if target.resolve()!=Path("E:/renderlab-work/m7-r0a/target").resolve():
                raise ValueError("unexpected Cargo target path")
            ledger["target/"] = dict(bytes=sum(p.stat().st_size for p in target.rglob("*") if p.is_file()),reason="owned Cargo target; checks and pinned executables retained")
            shutil.rmtree(target)
        r.cleanup_raw(root/"compatibility")
    r.save(ledger_path,ledger)
    print(json.dumps(dict(deleted_entries=len(ledger),free_gib=shutil.disk_usage(root).free/2**30)))


if __name__=="__main__":
    main()
