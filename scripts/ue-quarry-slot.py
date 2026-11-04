"""Atomic shared GPU lease. Evidence and the private project stay outside Git."""
import argparse
import datetime as dt
import json
from pathlib import Path
import subprocess
import uuid

LOCK = Path("D:/Coding/renderlab-work/gpu-slot.lock")
PRIVATE = Path("D:/Coding/renderlab-work/ue-m5")
OWNER = "ue-matched"


def utc():
    return dt.datetime.now(dt.timezone.utc)


def processes():
    command = ("@(Get-CimInstance Win32_Process | Where-Object { $_.Name -match "
               "'(?i)UnrealEditor|UnrealGame|ShaderCompileWorker|UnrealLightmass|renderlab|fresh-tests|m6-shadows|sel2shadow|primary.*probe|win64-shipping|valorant' -or "
               "($_.ExecutablePath -match '(?i)\\\\(renderlab-work|renderlab)\\\\.*\\.exe$' -and $_.Name -notmatch '(?i)^(cargo|rustc|rust-analyzer|python|py|powershell|pwsh)\\.exe$') } "
               "| Select-Object Name,ProcessId,ExecutablePath) | ConvertTo-Json -Compress")
    output = subprocess.check_output(["powershell", "-NoProfile", "-Command", command], text=True,
                                     creationflags=subprocess.CREATE_NO_WINDOW).strip()
    value = json.loads(output) if output else []
    return value if isinstance(value, list) else [value]


def read_lease():
    value = json.loads((LOCK / "owner.json").read_text(encoding="utf-8-sig"))
    if value.get("owner") != OWNER:
        raise RuntimeError("GPU slot belongs to another worker")
    age = (utc() - dt.datetime.fromisoformat(value["utc"].replace("Z", "+00:00"))).total_seconds()
    remaining = min(45, value["expected_minutes"]) * 60 - age
    if remaining <= 0:
        raise RuntimeError("GPU lease expired; release before another launch")
    return value, remaining


def history(event, record):
    directory = PRIVATE / "Quarry/slot-history"
    directory.mkdir(parents=True, exist_ok=True)
    name = utc().strftime("%Y%m%dT%H%M%S.%fZ") + "-" + event + ".json"
    (directory / name).write_text(json.dumps(record, indent=2) + "\n")


def remove_lock(expected):
    # Only remove the one known metadata file; unexpected contents require inspection.
    contents = list(LOCK.iterdir())
    if contents != [LOCK / "owner.json"]:
        raise RuntimeError("unexpected GPU lock contents; refusing removal")
    claimed = LOCK / ("owner.reaping-" + uuid.uuid4().hex + ".json")
    (LOCK / "owner.json").rename(claimed)  # Only one stale reaper can claim the metadata.
    if json.loads(claimed.read_text(encoding="utf-8-sig")) != expected:
        claimed.rename(LOCK / "owner.json")
        raise RuntimeError("GPU lease changed; refusing removal")
    claimed.unlink()
    LOCK.rmdir()


def acquire(minutes):
    if not 1 <= minutes <= 45:
        raise ValueError("GPU windows must be 1–45 minutes")
    try:
        LOCK.mkdir()
    except FileExistsError:
        try:
            existing = json.loads((LOCK / "owner.json").read_text(encoding="utf-8-sig"))
        except (FileNotFoundError, json.JSONDecodeError):
            print("GPU slot held; owner metadata is being published or released")
            return 2
        age = (utc() - dt.datetime.fromisoformat(existing["utc"].replace("Z", "+00:00"))).total_seconds()
        live = processes()
        if age <= 3600 or live:
            print(json.dumps(dict(status="held", lease=existing, live=live), indent=2))
            return 2
        history("stale", dict(lease=existing, age_seconds=age, live=live))
        remove_lock(existing)
        LOCK.mkdir()  # Competing acquisition still fails atomically.
    lease = dict(owner=OWNER, utc=utc().isoformat(), expected_minutes=minutes, token=uuid.uuid4().hex)
    (LOCK / "owner.json").write_text(json.dumps(lease, indent=2) + "\n")
    history("acquire", lease)
    print(json.dumps(lease, indent=2))
    return 0


def release():
    lease = json.loads((LOCK / "owner.json").read_text(encoding="utf-8-sig"))
    if lease.get("owner") != OWNER:
        raise RuntimeError("cannot release another worker's GPU slot")
    live = processes()
    if any("unreal" in p["Name"].lower() or "shadercompile" in p["Name"].lower() for p in live):
        raise RuntimeError("owned Unreal work must finish before release")
    record = dict(owner=OWNER, released_utc=utc().isoformat(), acquired=lease, live_processes=live)
    history("release", record)
    remove_lock(lease)
    (PRIVATE / "gpu-released.json").write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps(record, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("acquire", "release", "status"))
    parser.add_argument("--minutes", type=int, default=45)
    args = parser.parse_args()
    if args.action == "acquire":
        raise SystemExit(acquire(args.minutes))
    if args.action == "release":
        release()
    else:
        print(json.dumps(read_lease(), indent=2))
