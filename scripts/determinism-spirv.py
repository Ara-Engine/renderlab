"""Extract actual vkCreateShaderModule pCode from a LunarG API dump.

py -3 scripts/determinism-spirv.py E:/.../api.txt E:/.../spirv
Writes one file per unique binary and every creation's entry points/hash/length.
"""
from pathlib import Path
import hashlib
import json
import re
import struct
import sys

source, output = map(Path, sys.argv[1:3])
output.mkdir(parents=True, exist_ok=True)
creations = []
for line in source.open():
    if not line.lstrip().startswith("[ 0x07230203,"):
        continue
    words = [int(h, 16) for h in re.findall(r"0x[0-9a-fA-F]{8}", line)]
    data = struct.pack(f"<{len(words)}I", *words)
    entries = []
    at = 5
    contractions = 0
    while at < len(words):
        count, opcode = words[at] >> 16, words[at] & 65535
        assert count and at + count <= len(words)
        if opcode == 15:  # OpEntryPoint: execution model, ID, then NUL-terminated name.
            entries.append(struct.pack(f"<{count - 3}I", *words[at + 3:at + count]).split(b"\0")[0].decode())
        if opcode == 71 and count == 3 and words[at + 2] == 42:
            contractions += 1  # OpDecorate ... NoContraction.
        at += count
    sha = hashlib.sha256(data).hexdigest()
    (output / f"{sha}.spv").write_bytes(data)
    creations.append(dict(entries=entries, sha256=sha, bytes=len(data), no_contraction=contractions))
(output / "creations.json").write_text(json.dumps(creations, indent=2))
print(f"{len(creations)} creations, {len(set(c['sha256'] for c in creations))} unique binaries")
