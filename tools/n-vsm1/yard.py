"""N-VSM1 on the yardstick scenes: tools/p-flicker's run.py and timing.py over this unit's work
folder and binaries (bin/NAME/p_yard_capture.exe).

    py -3 -B tools/n-vsm1/yard.py run STEP...          # inside a lease; STEP as tools/p-flicker/run.py,
                                                       # with bin=NAME/p_yard_capture; RENDERLAB_DIAG=
                                                       # vsm-lookup=map selects a lookup (';' between keys)
    py -3 -B tools/n-vsm1/yard.py timing OUT.json RUN...   # span and sun-pass p50 per state, Epic ratios
"""
import importlib.util
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
WORK = Path("D:/Coding/renderlab-work/n-vsm1")
STATES = ("still", "moving", "cut", "recover")


def flicker_tool(name):
    sys.path.insert(0, str(HERE.parent / "p-flicker"))
    spec = importlib.util.spec_from_file_location("nvsm1_" + name, HERE.parent / "p-flicker" / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.WORK = WORK
    return module


def sun(passes):
    """The sun pass's stages (`sun_shadow`, and its `sun_shadow.STAGE` parts under
    RENDERLAB_PASS_LOG_DETAIL), summed."""
    return sum(v for k, v in passes.items() if k.split(".")[0] == "sun_shadow")


def timing(out, runs):
    t = flicker_tool("timing")
    result = {run: t.summary(run) for run in runs}
    epic = {k: {s: v["p50"] for s, v in e.items()} for k, e in t.EPIC.items()}
    Path(out).write_text(json.dumps(dict(runs=result, epic=epic), indent=1))
    for run, states in result.items():
        e = epic["ue-hw-e" if run.endswith("-rt") else "ue-sw-e"]
        print(run)
        for s in STATES:
            if s not in states or "gpu" not in states[s]:
                continue
            p = states[s].get("passes", {})
            parts = " ".join(f"{k.split('.', 1)[1]} {v:.3f}" for k, v in sorted(p.items()) if k.startswith("sun_shadow."))
            print(f"  {s:8} frame {states[s]['gpu']:6.2f} ms ({states[s]['gpu'] / e[s]:.2f}x Epic's {e[s]:.2f})"
                  f" | sun {sun(p):6.3f} ms {parts}")


if __name__ == "__main__":
    mode = sys.argv[1]
    if mode == "run":
        run = flicker_tool("run")
        codes = [run.run(s) for s in sys.argv[2:]]
        raise SystemExit(max(codes) if codes else 0)
    if mode == "timing":
        timing(sys.argv[2], sys.argv[3:])
