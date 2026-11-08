"""Fetch the redistributable SK1 walk fixture, with pinned attribution and content."""
import hashlib
from pathlib import Path
import sys
import urllib.request

revision = "8f492390ddc7a2ae221a1369ef560589cf8c70fb"
base = f"https://raw.githubusercontent.com/KhronosGroup/glTF-Sample-Assets/{revision}/Models/Fox/"
out = Path(sys.argv[1] if len(sys.argv) > 1 else "D:/Coding/renderlab-work/m7-sk1")
out.mkdir(parents=True, exist_ok=True)
data = urllib.request.urlopen(base + "glTF-Binary/Fox.glb", timeout=60).read()
assert hashlib.sha256(data).hexdigest() == "d97044e701822bac5a62696459b27d7b375aada5de8574ed4362edbba94771f7"
(out / "Fox.glb").write_bytes(data)
(out / "Fox-attribution.md").write_bytes(urllib.request.urlopen(base + "README.md", timeout=60).read())
print(out / "Fox.glb")
