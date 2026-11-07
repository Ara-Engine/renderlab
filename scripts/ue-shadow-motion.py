"""M6 close: Unreal 5.7.4's shadow-pass GPU cost per frame under our quarry harness's motions.

Reuses the matched e2e static command lines (packaged Development UnrealGame.exe, D3D12 SM6,
-RenderOffscreen, hidden startup, no window; docs/quarry-end-to-end-comparison.md) and N8h's
launcher (docs/non-rt-n8h.md). Only the log path, r.RayTracing (vsm-nort) and the CSV-frame
commands change. Each process runs 160 CSV frames: 0-63 still at pose 0, then from frame 64
the workload's motion, one step a frame, through the measured block 96-159:

- static: nothing moves.
- camera: eye and target translate 1 mm a frame along source X (UE +0.1 cm Y), as the no-RT
  quarry harness's move phase (`move_frames`, src/visibility_buffer_tests.rs).
- sun: the sun turns 0.5 mrad a frame about source Y (UE yaw), as the harness's turn phase.
- caster: placement 0 (Quarry_00) translates 1 mm a frame along source X, as `move_batch` 0.
  It needs a movable component; `--probe` reports mobility and the actor names first.

ProfileGPU trees at PROFILE_FRAMES price VSM's stages separately (invalidation, page marking,
page allocation, raster, merge, projection); those frames and the next are excluded from the
CSV statistics. Run inside a GPU lease:

    N8_OWNER=m6-close N8_OUT=E:/renderlab-work/m6-close py -3 -B scripts/n8-run.py \
        --label ue-motion --minutes 15 --fast-check -- py -3 -B scripts/ue-shadow-motion.py run s1
    py -3 -B scripts/ue-shadow-motion.py summarize s1      # CPU only, from saved logs and CSVs
"""
import csv
import json
import math
import re
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path

LAUNCHES = Path("D:/Coding/renderlab-work/ue-m5/Quarry/launches")
PACKAGE = Path("D:/Coding/renderlab-work/ue-m5/Quarry/package/Windows")
SAVED = PACKAGE / "M5Calibration/Saved"
OUT = Path("E:/renderlab-work/m6-close/ue")
# Unreal's exec resolves relative paths only (probe-p3); the e2e files sit six levels up.
EXEC_ROOT = Path("D:/Coding/renderlab-work/m6-close-ue")
EXEC_PREFIX = "../../../../../../m6-close-ue"
FLAGS = subprocess.CREATE_NO_WINDOW
MOTION_START, BLOCK, FRAMES = 64, range(96, 160), 160
PROFILE_FRAMES = (100, 116, 132, 148)
EYE = (-620.0, 520.0, 340.0)  # pose 0 in UE cm: 100 * (-Z, X, Y) of eye [5.2, 3.4, 6.2]
SUN = (-57.994616792, -53.130102354)  # pitch, yaw (degrees) of the registered travel
CONFIGS = {  # label: (matched e2e source launch, r.RayTracing)
    "vsm-nort": ("e2e-matched-s3-01-vsm-static", False),
    "vsm-rt": ("e2e-matched-s3-01-vsm-static", True),
    "hrt": ("e2e-matched-s3-02-hrt-static", True),
}
WORKLOADS = ("static", "camera", "sun", "caster")


def step_command(workload, k, caster_actor):
    """The console command placing the workload's moving object k steps from the pose."""
    if workload == "camera":
        x, y, z = EYE
        return f'ke CameraActor K2_SetActorLocation "(X={x:g},Y={y + 0.1 * k:.4f},Z={z:g})" false () false'
    if workload == "sun":
        pitch, yaw = SUN
        return f'ke DirectionalLight K2_SetActorRotation "(Pitch={pitch},Yaw={yaw + math.degrees(0.0005 * k):.9f},Roll=0)" false'
    if workload == "caster":
        return f'ke {caster_actor} K2_SetActorLocation "(X=0,Y={0.1 * k:.4f},Z=0)" false () false'
    raise ValueError(workload)


def csv_commands(workload, session, caster_actor, mobility):
    commands = []
    if workload == "caster" and mobility:
        commands.append((1, f"exec {EXEC_PREFIX}/{session}/caster-mobility.txt"))
    if workload != "static":
        folder = EXEC_ROOT / session / workload
        folder.mkdir(parents=True, exist_ok=True)
        for f in range(MOTION_START, FRAMES):
            k = f - MOTION_START + 1
            (folder / f"{k:03d}.txt").write_text(step_command(workload, k, caster_actor) + "\n", encoding="ascii")
            commands.append((f, f"exec {EXEC_PREFIX}/{session}/{workload}/{k:03d}.txt"))
    commands += [(f, "ProfileGPU") for f in PROFILE_FRAMES]
    return sorted(commands, key=lambda c: c[0])


def command(source, log, ray_tracing, commands, extra_exec=()):
    args = json.loads((LAUNCHES / source / "process.json").read_text())["command"]
    out = []
    for a in args:
        if a.startswith("-abslog="):
            a = f"-abslog={log}"
        elif a.startswith("-csvExecCmds="):
            a = "-csvExecCmds=" + ",".join(f"{f}:{c}" for f, c in commands)
        elif a == "-ini:Engine:[/Script/Engine.RendererSettings]:r.RayTracing=True":
            a = f"-ini:Engine:[/Script/Engine.RendererSettings]:r.RayTracing={ray_tracing}"
        elif a.startswith("-ExecCmds=") and extra_exec:
            body = a[len("-ExecCmds="):]
            assert body.endswith(",csvprofile frames=160")
            a = "-ExecCmds=" + body[: -len(",csvprofile frames=160")] + "," + ",".join(extra_exec) + ",csvprofile frames=160"
        out.append(a)
    assert "-RenderOffscreen" in out and any(a.startswith("-csvExecCmds=") for a in out)
    return out


def gpu_idle():
    p = subprocess.run(["nvidia-smi", "--query-gpu=utilization.gpu,memory.used,clocks.gr,temperature.gpu,power.draw",
                        "--format=csv,noheader"], capture_output=True, text=True, creationflags=FLAGS, timeout=20)
    return p.stdout.strip()


def launch(root, args):
    root.mkdir(parents=True, exist_ok=False)
    before = {p for p in SAVED.rglob("*") if p.is_file()}
    startup = subprocess.STARTUPINFO()
    startup.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = 0
    idle = gpu_idle()
    start = time.perf_counter()
    with (root / "stdout.log").open("w", encoding="utf-8") as log:
        proc = subprocess.Popen(args, stdout=log, stderr=subprocess.STDOUT, creationflags=FLAGS,
                                cwd=str(Path(args[0]).parent), startupinfo=startup)
        try:
            code = proc.wait(timeout=240)
        except subprocess.TimeoutExpired:
            subprocess.run(["taskkill", "/PID", str(proc.pid), "/T", "/F"], capture_output=True, creationflags=FLAGS)
            code = proc.wait()
    elapsed = time.perf_counter() - start
    new = sorted(p for p in SAVED.rglob("*") if p.is_file() and p not in before)
    for p in new:
        if p.suffix == ".csv":
            shutil.copyfile(p, root / p.name)
    record = dict(command=args, exit_code=code, elapsed_seconds=elapsed, gpu_before_launch=idle, gpu_after=gpu_idle(),
                  saved=[str(p) for p in new], scope="whole process; NOT GPU frame time")
    (root / "process.json").write_text(json.dumps(record, indent=2) + "\n")
    print(root.name, "exit", code, f"{elapsed:.1f} s", "idle GPU before:", idle, flush=True)
    return code


def probe(session):
    """Mobility and runtime names of the quarry's mesh components, and whether ke can make one movable."""
    root = OUT / session / "probe"
    folder = EXEC_ROOT / session
    folder.mkdir(parents=True, exist_ok=True)
    (folder / "probe-a.txt").write_text("getall StaticMeshComponent Mobility\ngetall StaticMeshComponent StaticMesh\n", encoding="ascii")
    (folder / "probe-b.txt").write_text("ke StaticMeshComponent SetMobility Movable\n", encoding="ascii")
    (folder / "probe-c.txt").write_text("getall StaticMeshComponent Mobility\n", encoding="ascii")
    commands = [(4, f"exec {EXEC_PREFIX}/{session}/probe-a.txt"), (8, f"exec {EXEC_PREFIX}/{session}/probe-b.txt"),
                (12, f"exec {EXEC_PREFIX}/{session}/probe-c.txt")]
    source, rt = CONFIGS["vsm-nort"]
    args = command(source, root / "ue.log", rt, commands)
    args = [a.replace("csvprofile frames=160", "csvprofile frames=20") for a in args]
    return launch(root, args)


def caster_target(session):
    """(actor name, needs SetMobility) for Quarry_00, from the probe log."""
    text = (OUT / session / "probe" / "ue.log").read_text(encoding="utf-8", errors="replace")
    mesh = re.findall(r"\d+\) StaticMeshComponent (\S+)\.StaticMeshComponent0\.StaticMesh = (.*)$", text, re.M)
    actor = next((path.rsplit(".", 1)[-1].rsplit(":", 1)[-1] for path, m in mesh if "Quarry_00.Quarry_00" in m), None)
    mobility = re.findall(r"\d+\) StaticMeshComponent (\S+)\.StaticMeshComponent0\.Mobility = (\S+)", text)
    return actor, mobility


def run(session, blocks=2, workloads=WORKLOADS, configs=tuple(CONFIGS)):
    status = 0
    if not (OUT / session / "probe" / "ue.log").exists():
        status |= probe(session)
    actor, mobility = caster_target(session) if "caster" in workloads else (None, [])
    # Readbacks before (first) and after (last) the probe's `ke ... SetMobility Movable`.
    movable = [m for p, m in mobility if p.endswith("." + (actor or "?")) or p.endswith(":" + (actor or "?"))]
    needs_mobility = not movable or "Movable" not in movable[0]
    if "caster" in workloads:
        print("caster actor", actor, "mobility", movable, flush=True)
        if actor is None or (needs_mobility and (not movable or "Movable" not in movable[-1])):
            print("caster dropped: no Quarry_00 actor, or ke cannot make it movable", flush=True)
            workloads = tuple(w for w in workloads if w != "caster")
        elif needs_mobility:
            (EXEC_ROOT / session / "caster-mobility.txt").write_text(
                "ke StaticMeshComponent SetMobility Movable\n", encoding="ascii")
    order = [(c, w) for w in workloads for c in configs]
    n = 0
    for block in range(blocks):
        for config, workload in (order if block % 2 == 0 else order[::-1]):
            source, rt = CONFIGS[config]
            label = f"{n:02d}-{config}-{workload}"
            root = OUT / session / label
            commands = csv_commands(workload, session, actor, needs_mobility)
            status |= launch(root, command(source, root / "ue.log", rt, commands))
            n += 1
    return status


TREE = re.compile(r"┃\s*(\d+) │\s*(\d+) │.*?│\s*([\d.]+) ms ┃\s*(\d+) │\s*(\d+) │.*?│\s*([\d.]+) ms ┃(\s*)(.*?)\s*┃\s*$")
STAGES = {  # stage: (node name, required ancestor or None); inclusive ms
    "invalidation": ("Shadow.Virtual.ProcessInvalidationsGPU", None),
    "mark_pages": ("VirtualShadowMapMarkPages", None),
    "page_allocation": ("FVirtualShadowMapArray::BuildPageAllocation", None),
    "raster_nanite": ("RenderVirtualShadowMaps(Nanite)", None),
    "raster_non_nanite": ("RenderVirtualShadowMaps(NonNanite)", None),
    "merge_static": ("FVirtualShadowMapArray::MergeStaticPhysicalPages", None),
    "projection": ("ShadowProjectionOnOpaque", None),
    # Hardware RT shadows: the light's shadow batch (rays plus denoiser), and the RT scene.
    "rt_shadow_batch": ("ShadowBatch", None),
    "rt_denoiser": ("DenoiseShadowVisibilityMasks", None),
    "rt_scene": ("RayTracingScene", None),
    "rt_nanite_streaming": ("NaniteRayTracing::UpdateStreaming", None),
}


def trees(log):
    """ProfileGPU trees in a ue.log: a list of [(depth, name, exclusive ms, inclusive ms)]."""
    out, current = [], None
    for line in log.read_text(encoding="utf-8", errors="replace").splitlines():
        if "LogRHI: Display:" not in line:
            if current:
                out.append(current)
                current = None
            continue
        body = line.split("LogRHI: Display:", 1)[1]
        m = TREE.search(body)
        if not m:
            if current and "┗" in body:
                out.append(current)
                current = None
            continue
        depth = (len(m.group(7)) - 1) // 3
        if m.group(8) == "<root>":
            if current:
                out.append(current)
            current = []
        if current is not None:
            current.append((depth, m.group(8), float(m.group(3)), float(m.group(6))))
    if current:
        out.append(current)
    return out


def stage_costs(tree):
    """Inclusive ms per VSM/RT shadow stage in one tree; `shadow_depths` is the top-level ShadowDepths node."""
    costs = {k: 0.0 for k in STAGES}
    costs["shadow_depths_top"] = 0.0
    names = {v[0]: k for k, v in STAGES.items()}
    stack = []
    other = {}
    for depth, name, excl, incl in tree:
        del stack[depth:]
        parents = list(stack)
        stack.append(name)
        if name in names:
            costs[names[name]] += incl
        elif name == "ShadowDepths" and "VirtualShadowMapMarkPages" not in parents:
            costs["shadow_depths_top"] += incl
        elif re.search(r"RayTrac.*Shadow|Shadow.*RayTrac|Denoise.*Shadow|ShadowDenois|SMRT|VirtualShadow|Shadow\.Virtual", name) \
                and not any(a in names or a in other for a in parents):
            other[name] = other.get(name, 0.0) + incl
    costs["other_shadow_nodes"] = other
    return costs


def read_csv(path):
    rows = list(csv.reader(path.open(encoding="utf-8", errors="replace")))
    header = [r for r in rows if r and r[0] == "EVENTS"][-1]
    data = [r for r in rows if r and r[0] != "EVENTS" and len(r) >= len(header) - 8]
    return header, data


def summarize(session):
    root = OUT / session
    cells = {}
    for run_dir in sorted(p for p in root.iterdir() if p.is_dir() and p.name[:2].isdigit()):
        _, config, workload = run_dir.name.split("-", 1)[0], *run_dir.name.split("-", 1)[1].rsplit("-", 1)
        csvs = sorted(run_dir.glob("Profile*.csv"))
        if not csvs:
            print(run_dir.name, "no CSV")
            continue
        header, data = read_csv(csvs[-1])
        skip = {f for p in PROFILE_FRAMES for f in (p, p + 1)}
        frames = [f for f in BLOCK if f < len(data) and f not in skip]
        col = {h: i for i, h in enumerate(header)}

        def med(name):
            vals = [float(data[f][col[name]]) for f in frames if name in col and data[f][col[name]] not in ("", None)]
            return statistics.median(vals) if vals else None

        gpu_cols = [h for h in header if h.startswith("GPU/")]
        row = dict(frames=len(frames), gpu_time=med("GPUTime"),
                   shadow_depths=med("GPU/ShadowDepths"), shadow_projection=med("GPU/ShadowProjection"),
                   local_used_mb=med("GPUMem/LocalUsedMB"),
                   gpu_columns={h: med(h) for h in gpu_cols if (med(h) or 0) > 0.005})
        log = run_dir / "ue.log"
        text = log.read_text(encoding="utf-8", errors="replace")
        moved = len(re.findall(r"Called 'K2_Set\w+ .*?\((?:1 succeeded|call succeeded)\)", text))
        row["moves_succeeded"] = moved
        tr = [stage_costs(t) for t in trees(log) if len(t) > 30]
        row["profile_trees"] = len(tr)
        row["stages_median"] = {k: statistics.median(t[k] for t in tr) for k in tr[0] if k != "other_shadow_nodes"} if tr else {}
        row["other_shadow_nodes"] = [t["other_shadow_nodes"] for t in tr]
        if row["shadow_depths"] is not None and row["shadow_projection"] is not None:
            row["shadow_csv_sum"] = row["shadow_depths"] + row["shadow_projection"]
        st = row["stages_median"]
        # Hardware RT shadows: the light's shadow batch (rays + denoiser) plus the RT scene and
        # Nanite RT streaming that only the RT shadow path needs here.
        row["rt_shadow_total"] = sum(st.get(k, 0.0) for k in ("rt_shadow_batch", "rt_scene", "rt_nanite_streaming"))
        cells.setdefault(f"{config}/{workload}", []).append(dict(run=run_dir.name, **row))
    (root / "summary.json").write_text(json.dumps(cells, indent=2) + "\n")
    for key, runs in cells.items():
        for r in runs:
            st = r["stages_median"]
            print(f"{key:18s} {r['run']:24s} GPU {r['gpu_time']:.3f} depths {r['shadow_depths'] or 0:.3f} proj {r['shadow_projection'] or 0:.3f} "
                  f"moves {r['moves_succeeded']:3d} | " + " ".join(f"{k}={v:.3f}" for k, v in st.items()))
    return 0


if __name__ == "__main__":
    action, session = sys.argv[1], sys.argv[2]
    if action == "run":
        workloads = tuple(sys.argv[3].split(",")) if len(sys.argv) > 3 else WORKLOADS
        blocks = int(sys.argv[4]) if len(sys.argv) > 4 else 2
        raise SystemExit(run(session, blocks, workloads))
    if action == "probe":
        raise SystemExit(probe(session))
    raise SystemExit(summarize(session))
