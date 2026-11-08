"""Measure a declared edit/build/relaunch command through its first-image marker.

Commands are JSON arrays, never shell text. Edits may target only a measurement
copy beneath D:/Coding/renderlab-work/ara1/staged; Ara worktrees stay read-only.
An adapter edit is an adapter-edit proxy, not a renderer incremental-build result.
"""
import argparse
import json
from pathlib import Path
import subprocess
import time

from capture import ara_process_check, digest, identity, require_lease
from lease import check_order

STAGED=Path("D:/Coding/renderlab-work/ara1/staged").resolve()


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("recipe",type=Path,help="engine, source, kind, scope, commands, ready, out, optional staged_edit")
    a=p.parse_args();recipe=json.loads(a.recipe.read_text());out=Path(recipe["out"])
    order,_=require_lease()
    ready=Path(recipe["ready"])
    if ready.exists() or out.exists():raise ValueError("use fresh ready/output paths")
    source=identity(Path(recipe["source"]))
    edit=Path(recipe["staged_edit"]).resolve() if recipe.get("staged_edit") else None
    if edit and (not edit.is_relative_to(STAGED) or edit.suffix not in (".rs",".wgsl",".slang",".slangh")):
        raise ValueError("only staged measurement copies may be edited")
    original=edit.read_bytes() if edit else None
    out.mkdir(parents=True)
    result=dict(recipe=recipe,recipe_sha256=digest(a.recipe),source=source,steps=[],measurement="process start / staged edit to a fresh GPU readback marker; excludes PNG encoding and scanout")
    start=time.perf_counter()
    try:
        if edit:edit.write_bytes(original+b"\n// ARA1 development-loop measurement edit\n")
        for index,command in enumerate(recipe["commands"]):
            check_order(order)
            if recipe["engine"]=="ara":ara_process_check()
            if not isinstance(command,list) or not command or not all(isinstance(v,str) for v in command):raise ValueError("command must be a nonempty string array")
            began=time.perf_counter()
            with (out/f"step-{index}.log").open("w") as log:
                child=subprocess.Popen(command,stdout=log,stderr=subprocess.STDOUT,creationflags=getattr(subprocess,"CREATE_NO_WINDOW",0))
                observed=None
                while child.poll() is None:
                    if observed is None and ready.exists():observed=(time.perf_counter()-start)*1000
                    time.sleep(.01)
            if observed is None and ready.exists():observed=(time.perf_counter()-start)*1000
            result["steps"].append(dict(command=command,exit_code=child.returncode,seconds=time.perf_counter()-began,ready_observed_ms=observed))
            if child.returncode:raise RuntimeError("development-loop command failed")
        result["edit_to_ready_ms"]=next((s["ready_observed_ms"] for s in result["steps"] if s["ready_observed_ms"] is not None),None)
        if result["edit_to_ready_ms"] is None:raise RuntimeError("command completed without first-image marker")
    finally:
        if edit:edit.write_bytes(original)
        result["source_status_unchanged"]=source==identity(Path(recipe["source"]))
        (out/"result.json").write_text(json.dumps(result,indent=2)+"\n")
    if not result["source_status_unchanged"]:raise RuntimeError("source changed while measuring")


if __name__=="__main__":main()
