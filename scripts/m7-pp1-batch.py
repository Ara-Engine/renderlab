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
template_data = json.loads(Path(template).read_text(encoding="utf-8-sig"))
scales = template_data.get("render_scales", [template_data.get("render_scale", "native")])
for scale in scales:
    for path in template_data.get("paths", ["rt", "nonrt"]):
        if path == "raster" and scale != "native":
            continue
        m = dict(template_data)
        output = root / scale / path if "render_scales" in m else root / path
        m.update(path=path, output=str(output), render_scale=scale,
                 fixture_only=m.get("fixture_only", False) or scale != "native" or path == "raster")
        manifest = root / f"{scale}-{path}.json"
        manifest.write_text(json.dumps(m, indent=2))
        commands.append((f"{scale}-{path}", [binary, "gpu_post_sweep", "--ignored", "--nocapture", "--test-threads=1"],
                         {"RENDERLAB_PRIMARY_MANIFEST": str(manifest)}))
for name, command, extra in commands:
    print("PP1 starting", name, flush=True)
    with (root / f"{name}.log").open("w") as log:
        result = subprocess.run(command, env=dict(os.environ, **extra), stdout=log, stderr=subprocess.STDOUT)
    print("PP1 finished", name, "exit", result.returncode, flush=True)
    if result.returncode:
        raise SystemExit(result.returncode)
