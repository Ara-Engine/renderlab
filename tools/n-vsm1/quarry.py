"""N-VSM1's quarry work as one batch for one 15-minute `--quarry-lease` (root, 2026-11-11: the
PC rarely has the 14 GiB the lease needs, so one window has to be enough). Binaries must be
built first (host.py build); nothing here compiles.

    py -3 -B tools/n-vsm1/quarry.py LABEL DEFAULT_BIN DISK_BIN     # e.g. q-y4 m0 d13
    py -3 -B tools/n-vsm1/quarry.py after LABEL                    # CPU, after the lease

Steps, in the order they would be dropped from the end if the lease ran short:
  1. Yardstick timing with the per-stage pass log, two visits each: the default lookup and Disk.
     Frame 0 of each process is the first frame of a brand-new view (no page exists yet).
  2. Capture frames of both, for the picture, error and flicker scores.
  3. Unreal software Lumen at Epic: the capture, for the side-by-side.
  4. One Unreal ProfileGPU of an early frame, for the brand-new-view cost on that side.
  5. Map and Hybrid timing, one visit each (the brief's step 1).
`after` decodes the Unreal dumps, prints the timing table with Epic ratios and scores error and
flicker (tools/p-yard/score.py's method).
"""
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
WORK = Path("D:/Coding/renderlab-work/n-vsm1")
PY = ["py", "-3", "-B"]
PASSES = "passes+RENDERLAB_PASS_LOG_DETAIL=1"


def runs(label):
    return [f"{label}-{n}-nonrt" for n in ("disk-1", "surface-1", "disk-2", "surface-2", "map-1", "hybrid-1")]


def lease(label, default, disk):
    base = f"bin={default}/p_yard_capture"
    new = f"bin={disk}/p_yard_capture+RENDERLAB_DIAG=vsm-lookup=disk"
    yard = [*PY, str(HERE / "yard.py"), "run"]
    steps = [
        ["timing=" + yard[0], *yard[1:],
         f"{label}-disk-1:timing:nonrt:{PASSES}+{new}", f"{label}-surface-1:timing:nonrt:{PASSES}+{base}",
         f"{label}-disk-2:timing:nonrt:{PASSES}+{new}", f"{label}-surface-2:timing:nonrt:{PASSES}+{base}"],
        ["capture=" + yard[0], *yard[1:],
         f"{label}-diskcap:capture:nonrt:{new}", f"{label}-surfacecap:capture:nonrt:{base}"],
        ["ue-capture=py", "-3", "-B", str(HERE.parent / "p-yard/ue.py"), "capture", "quarry", "ue-sw-e"],
        ["ue-first=py", "-3", "-B", str(HERE.parent / "p-yard/ue.py"), "profile", "quarry", "ue-sw-e", "--frame", "2"],
        ["lookups=" + yard[0], *yard[1:],
         f"{label}-map-1:timing:nonrt:{PASSES}+{base}+RENDERLAB_DIAG=vsm-lookup=map",
         f"{label}-hybrid-1:timing:nonrt:{PASSES}+{base}+RENDERLAB_DIAG=vsm-lookup=hybrid;vsm-march-samples=6"],
    ]
    command = [*PY, str(HERE / "steps.py"), str(WORK / "runs" / label)]
    for i, step in enumerate(steps):
        command += ([";;"] if i else []) + step
    return subprocess.call([*PY, str(HERE / "host.py"), "lease", label, "15", "--", *command])


def after(label):
    subprocess.call([*PY, str(HERE.parent / "p-yard/ue.py"), "decode", "quarry"])
    done = [r for r in runs(label) if (WORK / "runs" / r / "engine.json").exists()]
    subprocess.call([*PY, str(HERE / "yard.py"), "timing", str(WORK / f"{label}-timing.json"), *done])
    subprocess.call([*PY, str(HERE / "quality.py"), f"{label}-diskcap-nonrt", f"{label}-surfacecap-nonrt"])


if __name__ == "__main__":
    if sys.argv[1] == "after":
        after(sys.argv[2])
    else:
        if not sys.argv[1].startswith("q-"):
            raise SystemExit("the label must start with q- (a quarry lease)")
        raise SystemExit(lease(*sys.argv[1:4]))
