"""One guarded lease. Load each quarry path once; retain only images/JSON/logs."""
import json
import os
from pathlib import Path
import subprocess
import sys

binary, root, template = sys.argv[1:4]
root = Path(root)
root.mkdir(parents=True, exist_ok=True)
commands = [("lib", [binary, "--test-threads=1"], {})]
commands.append(("sk1", [binary, "gpu_skinned_streams_history_and_paths", "--ignored", "--nocapture", "--test-threads=1"], {}))
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
            "--frames","24","--every","8","--size","960x540"],{}))
for name, command, extra in commands:
    print("MV1 starting",name,flush=True)
    with (root/f"{name}.log").open("w") as log:
        result = subprocess.run(command,env=dict(os.environ,**extra),stdout=log,stderr=subprocess.STDOUT)
    print("MV1 finished",name,"exit",result.returncode,flush=True)
    if result.returncode:
        raise SystemExit(result.returncode)
