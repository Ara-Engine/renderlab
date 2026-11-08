"""One guarded lease. Load each quarry path once; retain only images/JSON/logs."""
import json
import importlib.util
import datetime as dt
import os
from pathlib import Path
import subprocess
import sys
import time

binary, root, template = sys.argv[1:4]
root = Path(root)
root.mkdir(parents=True, exist_ok=True)
spec = importlib.util.spec_from_file_location("mv1_run", Path(__file__).with_name("m7-mv1-run.py"))
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)
u = guard.g1.ue()
def ready():
    return (guard.turn_ready(u) and not guard.g1.quiet_active(u, any_lease=False)
            and guard.g1.hold_state(u) != "hold")

commands = [("lib", [binary, "--test-threads=1"], {})]
commands.append(("mq1", [binary, "gpu_mq1_moving_placement", "--ignored", "--nocapture", "--test-threads=1"], {}))
commands.append(("static", [binary, "gpu_mv1_mq1_static_identity", "--ignored", "--nocapture", "--test-threads=1"],
    {"RENDERLAB_MQ1_RESOLVE": str(guard.OUT / "mq1-622db4d.wgsl")}))
m = json.loads(Path(template).read_text(encoding="utf-8-sig"))
minimum = max(guard.ADMISSION_MB, 0 if m.get("fixture_only") is True else 8192)
for path in ["nonrt", "rt"]:
    manifest = root / f"{path}.json"
    manifest.write_text(json.dumps(dict(m, path=path, output=str(root/path)), indent=2))
    commands.append((path,[binary,"gpu_object_motion","--ignored","--nocapture","--test-threads=1"],
        {"RENDERLAB_PRIMARY_MANIFEST":str(manifest)}))
if len(sys.argv)>4:
    example = sys.argv[4]
    for path in ["nonrt", "rt"]:
        commands.append(("example-"+path,[example,"--path",path,"--out",str(root/("example-"+path)),
            "--frames","48","--every","8","--size","960x540",
            "--pipeline-cache",str(guard.OUT/"pipeline-cache")],{}))
commands.append(("sk1", [binary, "gpu_skinned_streams_history_and_paths", "--ignored", "--nocapture", "--test-threads=1"], {}))
progress = root / "completed.json"
done = json.loads(progress.read_text()) if progress.exists() else []
failures = root / "failed.json"
failed = json.loads(failures.read_text()) if failures.exists() else []
for name, command, extra in commands:
    if name in done or name in failed:
        continue
    # Recheck after n8's preflight and before every simulation. A memory wait or a
    # new root order never authorizes launching a later step under an old decision.
    memory = guard.available_mb()
    if memory < minimum and ready():
        print("MV1 waiting <=45 s for", minimum, "MiB before", name, "(currently", memory, "MiB)", flush=True)
        deadline = time.monotonic() + 45
        while memory < minimum and time.monotonic() < deadline and ready():
            time.sleep(1)
            memory = guard.available_mb()
    if not ready() or memory < minimum:
        print("MV1 yielding before", name, "(order/hold or", memory, "MiB available)", flush=True)
        raise SystemExit(3)
    starts = root / "starts.json"
    rows = json.loads(starts.read_text()) if starts.exists() else []
    rows.append(dict(name=name, available_mib=memory, admission_mib=minimum, utc=dt.datetime.now(dt.timezone.utc).isoformat()))
    starts.write_text(json.dumps(rows, indent=2))
    print("MV1 starting",name,"with",memory,"MiB available",flush=True)
    with (root/f"{name}.log").open("w") as log:
        env = dict(os.environ, RENDERLAB_PRIMARY_MANIFEST=template)
        env.update(extra)
        result = subprocess.run(command,env=env,stdout=log,stderr=subprocess.STDOUT)
    print("MV1 finished",name,"exit",result.returncode,flush=True)
    if result.returncode:
        failed.append(name)
        failures.write_text(json.dumps(failed, indent=2))
    else:
        done.append(name)
        progress.write_text(json.dumps(done, indent=2))
if failed:
    raise SystemExit("MV1 failed steps: " + ", ".join(failed))
