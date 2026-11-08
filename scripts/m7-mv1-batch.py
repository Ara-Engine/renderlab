"""One guarded lease. Load each quarry path once; retain only images/JSON/logs."""
import json
import importlib.util
import os
from pathlib import Path
import subprocess
import sys

binary, root, template = sys.argv[1:4]
root = Path(root)
root.mkdir(parents=True, exist_ok=True)
spec = importlib.util.spec_from_file_location("mv1_run", Path(__file__).with_name("m7-mv1-run.py"))
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)
u = guard.g1.ue()
commands = [("lib", [binary, "--test-threads=1"], {})]
commands.append(("sk1", [binary, "gpu_skinned_streams_history_and_paths", "--ignored", "--nocapture", "--test-threads=1"], {}))
commands.append(("mq1", [binary, "gpu_mq1_moving_placement", "--ignored", "--nocapture", "--test-threads=1"], {}))
commands.append(("static", [binary, "gpu_mv1_mq1_static_identity", "--ignored", "--nocapture", "--test-threads=1"],
    {"RENDERLAB_MQ1_RESOLVE": str(guard.OUT / "mq1-622db4d.wgsl")}))
m = json.loads(Path(template).read_text(encoding="utf-8-sig"))
for path in ["nonrt", "rt"]:
    manifest = root / f"{path}.json"
    manifest.write_text(json.dumps(dict(m, path=path, output=str(root/path)), indent=2))
    commands.append((path,[binary,"gpu_object_motion","--ignored","--nocapture","--test-threads=1"],
        {"RENDERLAB_PRIMARY_MANIFEST":str(manifest)}))
if len(sys.argv)>4:
    example = sys.argv[4]
    for path in ["nonrt", "rt"]:
        commands.append(("example-"+path,[example,"--path",path,"--out",str(root/("example-"+path)),
            "--frames","48","--every","8","--size","960x540"],{}))
progress = root / "completed.json"
done = json.loads(progress.read_text()) if progress.exists() else []
for name, command, extra in commands:
    if name in done:
        continue
    # Recheck after n8's preflight and before every simulation. A memory wait or a
    # new root order never authorizes launching a later step under an old decision.
    if not guard.turn_ready(u) or guard.available_mb() < 8192:
        print("MV1 yielding before", name, "(latest order or <8 GiB available)", flush=True)
        raise SystemExit(3)
    print("MV1 starting",name,flush=True)
    with (root/f"{name}.log").open("w") as log:
        result = subprocess.run(command,env=dict(os.environ,**extra),stdout=log,stderr=subprocess.STDOUT)
    print("MV1 finished",name,"exit",result.returncode,flush=True)
    if result.returncode:
        raise SystemExit(result.returncode)
    done.append(name)
    progress.write_text(json.dumps(done, indent=2))
