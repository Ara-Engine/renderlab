"""R0b CPU checks, guarded headless GPU probe, and reference scoring.

Shares the established U0b process/game/resource guards. Every GPU child stays
inside this blocking controller's owned lease; no queued work survives exit.
"""
import argparse
import datetime as dt
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

REPO = Path(__file__).resolve().parent.parent
OUT = Path("E:/renderlab-work/m7-r0b")
spec = importlib.util.spec_from_file_location("r0a", REPO / "scripts/m7-r0a.py")
r = importlib.util.module_from_spec(spec)
spec.loader.exec_module(r)


def note(action):
    with r.SCHEDULE.open("a", encoding="utf-8") as f:
        f.write(f"\n[m7-r0b CPU-only] owner=m7-r0b action={action}; no GPU/window; two Cargo jobs. {dt.datetime.now(dt.timezone.utc).isoformat()}\n")


def wait_cpu():
    while r.active_quiet(r.schedule_text(), r.owner_now()):
        print("Waiting outside foreign QUIET window", flush=True)
        time.sleep(5)


def checks(args):
    wait_cpu()
    note("library tests and fmt" if args.full else "targeted CPU shader validation")
    command = ["cargo", "test", "-p", "renderlab", "--lib", "-j", "2", "--target-dir", str(OUT / "target"), "--message-format=json"]
    if not args.full:
        command.append("reference::reflection_probe::probe_shader_and_program_sources_validate")
    command += ["--", "--test-threads=2"]
    start = time.perf_counter()
    with (OUT / "checks.jsonl").open("w") as log, (OUT / "checks.stderr.log").open("w") as err:
        p = subprocess.Popen(command, cwd=REPO, stdout=log, stderr=err, creationflags=subprocess.CREATE_NO_WINDOW)
        try:
            while p.poll() is None:
                if r.active_quiet(r.schedule_text(), r.owner_now()) or r.available_gib() < 2:
                    raise RuntimeError("owned CPU check stopped for foreign QUIET/resource guard")
                time.sleep(.5)
        finally:
            if p.poll() is None:
                subprocess.run(["taskkill", "/PID", str(p.pid), "/T", "/F"], capture_output=True, creationflags=subprocess.CREATE_NO_WINDOW)
                p.wait()
    print((OUT / "checks.stderr.log").read_text()[-10000:])
    lines = (OUT / "checks.jsonl").read_text().splitlines()
    for line in lines:
        if line.startswith("{"):
            message = json.loads(line)
            if message.get("reason") == "compiler-message":
                print(message["message"].get("rendered", ""))
    print("\n".join(line for line in lines if not line.startswith("{"))[-8000:])
    if p.returncode:
        raise SystemExit(p.returncode)
    artifacts = [json.loads(line) for line in lines if line.startswith("{")]
    binary = next(x["executable"] for x in artifacts if x.get("reason") == "compiler-artifact" and x.get("executable") and x["target"]["name"] == "renderlab")
    (OUT / "bin").mkdir(exist_ok=True)
    shutil.copyfile(binary, OUT / "bin/probe-tests.exe")
    wait_cpu()
    fmt = subprocess.run(["cargo", "fmt", "--all", "--", "--check"], cwd=REPO, capture_output=True, text=True, creationflags=subprocess.CREATE_NO_WINDOW)
    r.save(OUT / ("checks-full.json" if args.full else "checks-shader.json"), dict(cargo_exit=p.returncode, fmt_exit=fmt.returncode,
        fmt_output=fmt.stdout+fmt.stderr, seconds=time.perf_counter()-start, binary_sha256=r.sha(OUT / "bin/probe-tests.exe"), command=command))
    print(fmt.stdout+fmt.stderr)
    raise SystemExit(fmt.returncode)


def run(args):
    u = r.ue()
    u.OUT = OUT
    u.OWNER = "m7-r0b"
    inherited_abort = u.ABORT_REQUESTED
    def abort():
        reason = inherited_abort()
        if reason:
            return reason
        if r.active_quiet(r.schedule_text(), r.owner_now()):
            return "foreign QUIET window began"
        return None
    u.ABORT_REQUESTED = abort
    label = args.label
    u.schedule_line(f'[m7-r0b request] owner=m7-r0b asks for one <=10 min lease "{label}" (headless correctness probe; timings functional only) {dt.datetime.now(dt.timezone.utc).isoformat()}')
    while True:
        wait_cpu()
        if r.game_state() != "released" or r.LOCK.exists():
            time.sleep(.5)
            continue
        try:
            with u.lease(label, 10) as owner:
                output = OUT / "runs" / label
                env = dict(os.environ, RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
                    RENDERLAB_M7_REFERENCE_OUT=str(output), RENDERLAB_R0B_SCENES=";".join(str(r.OUT / "scenes" / x) for x in args.fixtures),
                    VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation")
                env.pop("VK_INSTANCE_LAYERS", None)
                r.save(output / "recipe.json", dict(lease=owner, binary_sha256=r.sha(OUT / "bin/probe-tests.exe"),
                    environment={k:v for k,v in env.items() if k.startswith("RENDERLAB_")}, timing_eligible_for_claim=False))
                for test in args.test.split(","):
                    u.guarded([OUT / "bin/probe-tests.exe", "reference::reflection_probe::"+test, "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                        output / ("process-"+test), timeout=min(570,int(u.remaining_seconds(owner))-10), cwd=REPO, env=env)
            return
        except SystemExit as e:
            if e.code != 3:
                raise
            time.sleep(5)


def score(args):
    import numpy as np
    from PIL import Image
    wait_cpu()
    note("score retained RF images against R0a archives and floors")
    floor = json.loads((r.OUT / "scores/c-static.json").read_text())
    manifest = json.loads((r.OUT / "reference/final/manifest.json").read_text())
    codes = r.load("m7-u0b-score").codes
    rows = []
    run = OUT / "runs" / args.label
    keys = ["rgb_relative_rms", "luminance_relative_l1", "absolute_energy_bias",
            "display_rms_-14", "p99_absolute_display_error_-14", "display_rms_-10", "p99_absolute_display_error_-10"]
    for fixture in args.fixtures:
        scene_path = r.OUT / "scenes" / fixture / "scene.json"
        scene = json.loads(scene_path.read_text())
        for pose in scene["poses"]:
            case = fixture + "-" + pose["name"]
            reference = r.OUT / "reference/final" / manifest["cases"][case]["archive"]
            if r.sha(reference) != manifest["cases"][case]["archive_sha256"]:
                raise ValueError("reference identity changed: " + case)
            with np.load(reference) as ref:
                h0, h1 = ref["L0"].astype("f4"), ref["L1"].astype("f4")
                populations = {k[4:]: ref[k] for k in ref.files if k.startswith("pop_")}
            for rays in (4,8):
                name = case + f"-k{rays}"
                meta = json.loads((run / (name + ".json")).read_text())
                if meta["extra"]["scene_sha256"] != r.sha(scene_path):
                    raise ValueError("probe scene hash mismatch")
                archive = run / (name + ".npz")
                raw = run / (name + ".f32")
                if raw.exists():
                    candidate = np.fromfile(raw, dtype="<f4").reshape(meta["height"],meta["width"],3)
                    np.savez_compressed(archive, L=candidate)
                else:
                    with np.load(archive) as image:
                        candidate = image["L"]
                if candidate.shape != h0.shape or not np.isfinite(candidate).all():
                    raise ValueError("invalid candidate")
                scores = {}
                for pop,mask in populations.items():
                    wait_cpu()
                    scores[pop] = r.measures(candidate,h0,h1,mask)
                scores["complete-frame"] = r.measures(candidate,h0,h1,np.ones(h0.shape[:2],dtype=bool))
                features = r.feature_measures(candidate,(h0+h1)/2,populations)
                comparisons = {}
                for config in ("hw-hit-epic", "hw-sc-epic", "sw-mesh-epic"):
                    f = next(x for x in floor["rows"] if x["fixture"]==fixture and x["pose"]==pose["name"] and x["config"]==config)
                    if f["reference_archive_sha256"] != r.sha(reference):
                        raise ValueError("floor used a different reference: "+case)
                    checks = {pop:{key:scores[pop][key]<=values[key] for key in keys if key in values}
                        for pop,values in f["measures"].items() if pop in scores}
                    feature_checks = {}
                    for feature,values in f["features"].items():
                        measured = features.get(feature,{})
                        for key in ("footprint_energy_ratio","chromatic_energy_ratio","centroid_error_pixels"):
                            if values.get(key) is not None and measured.get(key) is not None:
                                error = (lambda x:abs(x-1)) if key.endswith("ratio") else (lambda x:x)
                                feature_checks[feature+"/"+key] = error(measured[key])<=error(values[key])
                        for key in ("candidate_missing_edges","median_extra_pixels","p95_extra_pixels"):
                            a,b=measured.get("edge_spread",{}).get(key),values.get("edge_spread",{}).get(key)
                            if a is not None and b is not None:
                                feature_checks[feature+"/"+key] = abs(a)<=abs(b)
                    comparisons[config] = dict(passes=all(v for checks in checks.values() for v in checks.values()),
                        failed=[f"{pop}/{key}" for pop,checks in checks.items() for key,passed in checks.items() if not passed],
                        passed=sum(v for checks in checks.values() for v in checks.values()), tested=sum(len(v) for v in checks.values()),
                        feature_checks=feature_checks,features_passed=sum(feature_checks.values()),features_tested=len(feature_checks))
                    comparisons[config]["passes"] &= all(feature_checks.values())
                row = dict(fixture=fixture,pose=pose["name"],rays=rays,measures=scores,
                    features=features,comparisons=comparisons,
                    scene_sha256=r.sha(scene_path),reference_archive_sha256=r.sha(reference),candidate_archive_sha256=r.sha(archive),
                    gpu_stage_ms=meta["gpu_stage_ms"],same_resource_replay_bit_identical=meta["same_resource_replay_bit_identical"],
                    scope="static only, all registered populations, no fitted gain or pixel rejection; RF10 HDR provisional")
                r.save(run/(name+"-scores.json"),row)
                ref=(h0+h1)/2
                sheet=np.concatenate([codes(ref,-14),codes(candidate,-14),np.minimum(255,4*np.abs(codes(candidate,-14)-codes(ref,-14)))],axis=1)
                Image.fromarray(np.clip(sheet,0,255).astype("u1")).save(run/(name+"-comparison.png"))
                # The compact candidate archive supports CPU re-scoring. Only the
                # disposable staging f32 is pruned; canonical references are read-only.
                if raw.exists():
                    raw.unlink()
                rows.append(row)
                print(name,comparisons["hw-hit-epic"]["passed"],"/",comparisons["hw-hit-epic"]["tested"],flush=True)
    r.save(OUT/(args.label+"-scores.json"),dict(rows=rows,complete=len(rows)==26,reference_manifest_sha256=r.sha(r.OUT/"reference/final/manifest.json")))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("command", choices=["checks", "run", "score"])
    p.add_argument("--full", action="store_true")
    p.add_argument("--label", default="pilot")
    p.add_argument("--test", default="gpu_r0b_fixtures")
    p.add_argument("--fixtures", nargs="+", default=[f"RF{i}" for i in range(1,11)])
    args = p.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    dict(checks=checks,run=run,score=score)[args.command](args)


if __name__ == "__main__":
    main()
