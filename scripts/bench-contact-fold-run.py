"""S1's E:-local launcher; reuse the shared atomic lease and live 4/2 GiB guard."""
import datetime
import ctypes
import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = Path("E:/renderlab-work/contact-fold")
SCHEDULE = Path("E:/renderlab-work/gpu-schedule.md")
spec = importlib.util.spec_from_file_location("film_runner", ROOT / "scripts/m6-film-run.py")
runner = importlib.util.module_from_spec(spec); spec.loader.exec_module(runner)
runner.OUT = OUT
runner.OWNER = "contact-fold-bench"
runner.PREFLIGHT_MEMORY_MBYTES = 4096
runner.RUNTIME_MEMORY_MBYTES = 2048
save_original = runner.save
claimed = False
state_original = runner.state
foreground_before = None


def foreground():
    user=ctypes.WinDLL("user32");user.GetForegroundWindow.restype=ctypes.c_void_p
    handle=user.GetForegroundWindow();pid=ctypes.c_ulong()
    user.GetWindowThreadProcessId(ctypes.c_void_p(handle),ctypes.byref(pid))
    return dict(handle=handle,pid=pid.value)


def state(owned=()):
    value=state_original(owned)
    # The generic guard identifies games by their Epic Games install path.
    # These exact UE engine helpers are not games or interactive renderers.
    helpers={"crashreportclienteditor.exe","crashreportclient.exe","shadercompileworker.exe","unrealtraceserver.exe","zenserver.exe"}
    value["engine_helpers"]=[p for p in value["games"] if p["Name"].lower() in helpers and "\\ue_5.7\\engine\\" in (p.get("ExecutablePath") or "").lower()]
    value["games"]=[p for p in value["games"] if p not in value["engine_helpers"]]
    return value


def save(path, value):
    global claimed,foreground_before
    if path == runner.LOCK / "owner.json":
        claimed = True
        end = datetime.datetime.fromisoformat(value["utc"]) + datetime.timedelta(minutes=value["expected_minutes"])
        with SCHEDULE.open("a",encoding="utf-8") as f:
            f.write(f"\n[{runner.OWNER} reservation] owner={runner.OWNER} end={end.isoformat()} label={value['label']}\n")
    if path.name.endswith("-preflight.json"):
        foreground_before=foreground();value["foreground_read_only"]=foreground_before
    if isinstance(value,dict) and "command" in value:
        value["foreground_before"]=foreground_before;value["foreground_after"]=foreground()
    save_original(path,value)


runner.save = save
runner.state = state
if __name__ == "__main__":
    try: raise SystemExit(runner.main())
    finally:
        if claimed:
            with SCHEDULE.open("a",encoding="utf-8") as f:
                f.write(f"\ncontact-fold-bench window released {runner.utc()} (no owned GPU/build job remains).\n")
