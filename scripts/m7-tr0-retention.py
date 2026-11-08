"""CPU-only accounting of TR0's hash-recorded deletions; never deletes files.

Count a path/content identity once, even when a compacted archive carries the
same ledger again. Provider-owned raw data is excluded from TR0's accounting.
"""
import hashlib
import json
from pathlib import Path

ROOT = Path("E:/renderlab-work/m7-tr0").resolve()
records = {}
ledgers = {}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def add(value, ledger):
    path = Path(value["path"]).resolve()
    if ROOT not in path.parents:
        raise ValueError("deletion ledger names a foreign path: " + str(path))
    # A transferred input can share bytes with another file; each distinct
    # path occupied disk and is counted. Copied ledgers are not counted twice.
    identity = (str(path).casefold(), value["sha256"])
    if identity in records and records[identity]["bytes"] != value["bytes"]:
        raise ValueError("conflicting deletion record")
    records[identity] = dict(value, path=str(path))
    ledgers[str(ledger)] = sha(ledger)


names = {"deleted-raw.json", "pruned-raw.json", "removed-transferred-inputs.json",
         "deleted-generated-scenes.json", "deleted-geometry.json", "deleted-staging.json"}
for run in (ROOT / "runs").iterdir():
    if not run.is_dir():
        continue
    for path in run.glob("*.json"):
        if path.name in names:
            for value in json.loads(path.read_text()):
                add(value, path)
        elif path.name == "rejected-reference-summary.json":
            for value in json.loads(path.read_text()).get("deleted_raw", []):
                add(value, path)
    for path in (run / "data").glob("*.json"):
        data = json.loads(path.read_text())
        if isinstance(data, dict) and isinstance(data.get("deleted_raw"), dict):
            for name, value in data["deleted_raw"].items():
                add(dict(value, path=str(path.parent / name)), path)

for folder in (ROOT / "s2g/geometry", ROOT / "s2g/physical"):
    for ledger in folder.rglob("deleted-raw.json"):
        for value in json.loads(ledger.read_text(encoding="utf-8")):
            add(value, ledger)
result = dict(bytes=sum(x["bytes"] for x in records.values()), files=len(records),
    ledgers=[dict(path=path, sha256=identity) for path, identity in sorted(ledgers.items())],
    note="Unique owned path/content deletions: native/raw reference dumps, generated scene copies, transferred archives and redundant pinned executables. Imported provider deletion records excluded. Remaining pending inputs are not counted.",
    script_sha256=sha(Path(__file__)))
(ROOT / "retention-summary.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
print("TR0 reclaimed bytes", result["bytes"], "files", result["files"], "ledgers", len(ledgers))
