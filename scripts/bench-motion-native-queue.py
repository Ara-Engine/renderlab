"""Reserve the native suite; retry shared-memory deferrals every two minutes."""
import datetime
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

ROOT=Path("E:/renderlab-work/motion-benches")
REPO=Path(__file__).resolve().parents[1]


def main():
    for bench in ("S2","S3"):
        root=ROOT/bench;env=os.environ.copy();env["RENDERLAB_MOTION_ROOT"]=str(root)
        process=root/"unreal-render-process.json"
        if (root/"unreal/complete.json").exists() and process.exists():
            prior=json.loads(process.read_text())
            if prior["exit"]==0 and not prior["errors"]:
                print("Native bench already complete",bench,flush=True);continue
        scene=json.loads((root/"fixture/scene.json").read_text())
        attempt=1
        while True:
            label=f"unreal-native-{attempt:02}"
            if (root/"runs"/(label+".json")).exists() or (root/"runs"/(label+"-deferred.json")).exists():attempt+=1;continue
            command=["py","-3","-B","scripts/bench-motion-run.py","--label",label,"--minutes","45","--ue-startup","--","py","-3","-B","scripts/bench-motion-unreal-run.py","render"]
            code=subprocess.run(command,cwd=REPO,env=env,creationflags=subprocess.CREATE_NO_WINDOW).returncode
            if code==0:break
            record=root/"runs"/(label+".json")
            data=json.loads(record.read_text()) if record.exists() else {}
            memory_failure=any("memory" in text for text in data.get("failures",[]))
            if code not in (1,3) or (code==1 and not memory_failure):raise SystemExit(code)
            if memory_failure:
                archive=root/"unreal-attempts"/label
                archive.mkdir(parents=True,exist_ok=True)
                for name in ("unreal-render.log","unreal-render-stdout.log","unreal-launch.json"):
                    source=root/name
                    if source.exists():shutil.copyfile(source,archive/source.name)
                for track in scene["tracks"]:
                    for radius in scene["radii"]:
                        case=root/"unreal"/f"{track['name']}-r{radius:g}"
                        for mode in ("VSM","HRT1","HRT256"):
                            directory=case/mode
                            if directory.exists() and not (directory/"config-complete.json").exists():
                                for failed in directory.glob("stage-*"):
                                    if (failed/"complete.done").exists():continue
                                    target=archive/case.name/mode/failed.name
                                    assert failed.resolve().is_relative_to(root.resolve()) and target.resolve().is_relative_to(root.resolve())
                                    target.parent.mkdir(parents=True,exist_ok=True);shutil.move(str(failed),str(target))
                        depth=case/"stage-depth"
                        if depth.exists() and not (depth/"complete.done").exists():
                            target=archive/case.name/"stage-depth"
                            assert depth.resolve().is_relative_to(root.resolve()) and target.resolve().is_relative_to(root.resolve())
                            target.parent.mkdir(parents=True,exist_ok=True);shutil.move(str(depth),str(target))
                # Failed raw captures are not accepted evidence. Preserve their
                # file/size inventory and metadata, then bound disk use.
                if archive.exists():
                    paths=list(archive.rglob("*.bin"));rows=[dict(path=str(p.relative_to(archive)),bytes=p.stat().st_size) for p in paths]
                    (archive/"aborted-raw.json").write_text(json.dumps(dict(resources=rows,accepted=False),indent=2))
                    for path in paths:path.unlink()
            print(json.dumps(dict(bench=bench,label=label,exit=code,retry_in_seconds=120,utc=datetime.datetime.now(datetime.timezone.utc).isoformat())),flush=True)
            attempt+=1;time.sleep(120)


if __name__=="__main__":main()
