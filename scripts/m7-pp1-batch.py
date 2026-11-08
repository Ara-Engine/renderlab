"""One PP1 lease: lib tests, then one asset load per path for every effect."""
import json
import os
from pathlib import Path
import subprocess
import sys

binary, root, template = sys.argv[1:]
root = Path(root)
root.mkdir(parents=True, exist_ok=True)
commands = [("lib", [binary, "--test-threads=1"], {})]
for path in ("rt", "nonrt"):
    m = json.loads(Path(template).read_text(encoding="utf-8-sig"))
    m.update(path=path, output=str(root / path), fixture_only=False)
    manifest = root / f"{path}.json"
    manifest.write_text(json.dumps(m, indent=2))
    commands.append((path, [binary, "gpu_post_sweep", "--ignored", "--nocapture", "--test-threads=1"],
                     {"RENDERLAB_PRIMARY_MANIFEST": str(manifest)}))
for name, command, extra in commands:
    print("PP1 starting", name, flush=True)
    with (root / f"{name}.log").open("w") as log:
        result = subprocess.run(command, env=dict(os.environ, **extra), stdout=log, stderr=subprocess.STDOUT)
    print("PP1 finished", name, "exit", result.returncode, flush=True)
    if result.returncode:
        raise SystemExit(result.returncode)
