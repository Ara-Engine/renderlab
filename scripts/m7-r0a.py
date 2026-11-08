"""R0a fixture/reference controller. Every GPU operation owns a blocking U0b lease.

CPU work declares itself in the shared schedule; builds/tests never enter a foreign
QUIET window. GPU correctness work is allowed under foreign CPU load, with the
existing native process inventory and 4/2-GiB RAM plus 5/2-GiB disk guards.
"""
import argparse
import contextlib
import copy
import ctypes
import datetime as dt
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import re
import shutil
import subprocess
import time

import numpy as np

REPO = Path(__file__).resolve().parent.parent
OUT = Path("E:/renderlab-work/m7-r0a")
SCHEDULE = Path("E:/renderlab-work/gpu-schedule.md")
LOCK = Path("D:/Coding/renderlab-work/gpu-slot.lock")
PINNED = Path("E:/renderlab-work/m7-u0b/bin/reference-tests.exe")
REFERENCE_BINARY = OUT/"bin/r0a-reference-tests.exe"
MOTION_SCORING_VERSION = 1


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), REPO/"scripts"/(name+".py"))
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module


def save(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2, allow_nan=False)+"\n", encoding="utf-8")


def sha(path):
    with path.open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


def active_quiet(text, owner=None):
    """A QUIET declaration lasts until its matching owner's labelled release.

    Expired reservation times alone never release ownership. Also consult the live
    owner token: a different active owner cannot release the declared window.
    """
    active = {}
    for line in text.splitlines():
        match = re.match(r"\[([^\]]+) QUIET (?:window|active)\].*?label=([^ ;]+)", line)
        if match:
            active[(match[1].lower(), match[2])] = line
        released = re.match(r"([^ ]+) (?:QUIET |functional )?([^ ]+) window released ", line, re.IGNORECASE)
        if released:
            owner_name = released[1].lower()
            active.pop((owner_name, released[2]), None)
            if released[2].startswith("study-"):
                active.pop((owner_name, released[2][6:]), None)
            prefix = owner_name.removeprefix("m7-")+"-"
            if released[2].startswith(prefix):
                active.pop((owner_name, released[2][len(prefix):]), None)
    if owner and "quiet" in owner.get("label", "").lower():
        active[(owner["owner"], owner["label"])] = owner
    return active


def owner_now():
    try:
        return json.loads((LOCK/"owner.json").read_text())
    except (FileNotFoundError, json.JSONDecodeError):
        return dict(owner="publishing", label="unresolved") if LOCK.exists() else None


def cpu_note(action):
    with SCHEDULE.open("a", encoding="utf-8") as stream:
        stream.write(f"\n[m7-r0a CPU-only] owner=m7-r0a action={action}; no GPU/window; two Cargo jobs if compiling. {dt.datetime.now(dt.timezone.utc).isoformat()}\n")


def schedule_text():
    """Retry a transient Windows writer-sharing conflict, then remain fail-closed."""
    for attempt in range(11):
        try:
            return SCHEDULE.read_text(encoding="utf-8")
        except PermissionError:
            if attempt==10:
                raise
            time.sleep(.02)


def game_state():
    state = "released"
    for line in schedule_text().splitlines():
        if line.startswith("[GAME HOLD RELEASED]"):
            state = "released"
        elif line.startswith("[GAME HOLD RELAXED]"):
            state = "relaxed"
        elif line.startswith("[GAME HOLD]"):
            state = "hold"
    return state


def available_gib():
    class Memory(ctypes.Structure):
        _fields_ = [("length", ctypes.c_ulong), ("load", ctypes.c_ulong)]+[
            (name, ctypes.c_ulonglong) for name in ("total_physical", "available_physical", "total_page", "available_page",
                "total_virtual", "available_virtual", "extended_virtual")]
    value = Memory(); value.length = ctypes.sizeof(value)
    if not ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(value)):
        raise ctypes.WinError()
    return value.available_physical/2**30


def no_quiet():
    quiet = active_quiet(schedule_text(), owner_now())
    if quiet:
        raise RuntimeError("foreign QUIET window: "+str(quiet))


def ue():
    base = load("m7-u0b")
    base.game_state = game_state
    # Build a fresh controller, keeping U0b's native process/memory guards, but
    # store our readiness, lease history and captures only under R0a.
    base.OUT = OUT
    u = base.ue(allow_game=False)
    u.OUT = OUT; u.OWNER = "m7-r0a"
    inherited_tool = u.engine_tool
    def engine_tool(process):
        if inherited_tool(process):
            return True
        path = (process.get("ExecutablePath") or "").lower().replace("/", "\\")
        # G1c announced these default-feature CPU cook tests at 08:11:22Z;
        # its optional GPU environment example is a separate executable.
        if path.startswith("e:\\renderlab-work\\m7-g1c\\target\\debug\\deps\\") and re.fullmatch(
            r"(?:renderlab_cook|virtual_cooking)-[0-9a-f]+\.exe", process["Name"].lower()):
            return True
        # Cargo-generated build scripts are compiler work, including generic-array
        # in the foreign fix1 tree. They are not ignored renderer/GPU test binaries.
        return process["Name"].lower()=="build-script-build.exe" and bool(re.search(
            r"\\target\\(?:debug|release)\\build\\[a-z0-9_-]+-[0-9a-f]+\\build-script-build\.exe$", path))
    u.engine_tool = engine_tool
    inherited = u.lease

    @contextlib.contextmanager
    def lease(label, minutes):
        next_notice = 0
        while (OUT/"cpu-gap").exists() or active_quiet(schedule_text(), owner_now()):
            if time.monotonic()>=next_notice:
                print("Waiting outside CPU gap / foreign QUIET window", flush=True)
                next_notice = time.monotonic()+60
            time.sleep(5)
        with inherited(label, minutes) as owner:
            no_quiet()
            yield owner
        # Give already queued owners one poll interval before the next batch.
        time.sleep(.75)
    u.lease = lease
    return u


def checks(args):
    """Own two-job Cargo target; stop only our child if a foreign QUIET starts."""
    cpu_note("requested CPU-only nonignored library tests / no-run build and fmt; two threads/jobs, separate pinned reference executable; no Rust edits during command")
    gap = OUT/"cpu-gap"
    gap.mkdir()
    proc = None
    try:
        while (live := owner_now()) is not None and live.get("owner")=="m7-r0a" and live.get("label", "").startswith("current-"):
            time.sleep(.5)
        no_quiet()
        if available_gib()<4 or shutil.disk_usage(OUT).free<5*2**30:
            raise RuntimeError("CPU check/build start requires 4 GiB RAM and 5 GiB disk")
        target = OUT/"target"
        command = ["cargo", "test", "-p", "renderlab", "--lib", "-j", "2", "--target-dir", str(target), "--message-format=json"]
        if args.command=="build":
            command.append("--no-run")
        else:
            command += ["--", "--test-threads=2"]
        start = time.perf_counter()
        with (OUT/"build.jsonl").open("w") as log, (OUT/"build.stderr.log").open("w") as err:
            proc = subprocess.Popen(command, cwd=REPO, stdout=log, stderr=err, creationflags=subprocess.CREATE_NO_WINDOW)
            save(OUT/"cpu-process.json", dict(pid=proc.pid, command=command))
            while proc.poll() is None:
                live = owner_now()
                if available_gib()<2 or shutil.disk_usage(OUT).free<2*2**30:
                    raise RuntimeError("CPU check/build resource guard")
                if active_quiet(schedule_text(), live):
                    subprocess.run(["taskkill", "/PID", str(proc.pid), "/T", "/F"], capture_output=True, creationflags=subprocess.CREATE_NO_WINDOW)
                    proc.wait(); raise RuntimeError("stopped owned CPU build for a GPU lease or foreign QUIET window")
                time.sleep(.5)
        if proc.returncode:
            raise RuntimeError((OUT/"build.stderr.log").read_text()[-5000:])
        artifacts = [json.loads(l) for l in (OUT/"build.jsonl").read_text().splitlines() if l.startswith("{")]
        executable = next(x["executable"] for x in artifacts if x.get("reason")=="compiler-artifact" and x.get("executable") and x["target"]["name"]=="renderlab")
        destination = OUT/"bin/r0a-tests.exe"; destination.parent.mkdir(exist_ok=True)
        shutil.copyfile(executable, destination)
        if not REFERENCE_BINARY.exists():
            shutil.copyfile(destination, REFERENCE_BINARY)
        no_quiet()
        fmt = subprocess.run(["cargo", "fmt", "--all", "--", "--check"], cwd=REPO, capture_output=True, text=True, creationflags=subprocess.CREATE_NO_WINDOW)
        save(OUT/("build.json" if args.command=="build" else "checks.json"), dict(cargo_exit=proc.returncode, fmt_exit=fmt.returncode, fmt_output=fmt.stdout+fmt.stderr,
            cargo_mode="no-run" if args.command=="build" else "library tests",
            seconds=time.perf_counter()-start, binary_sha256=sha(destination), command=command,
            rust_sources={str(p.relative_to(REPO)):sha(p) for p in (REPO/"src").rglob("*.rs")}))
        if fmt.returncode:
            raise RuntimeError(fmt.stdout+fmt.stderr)
    finally:
        if proc is not None and proc.poll() is None:
            subprocess.run(["taskkill", "/PID", str(proc.pid), "/T", "/F"], capture_output=True, creationflags=subprocess.CREATE_NO_WINDOW)
            proc.wait()
        gap.rmdir()


def cleanup_raw(root):
    deleted = {}
    for path in root.rglob("*"):
        if path.is_file() and path.suffix in (".f32", ".bin", ".rgba16f") and path.stat().st_size>5*1024**2:
            deleted[str(path.relative_to(root))] = dict(bytes=path.stat().st_size, sha256=sha(path))
            path.unlink()
    if deleted:
        previous = root/"deleted-raw.json"
        save(previous, (json.loads(previous.read_text()) if previous.exists() else {}) | deleted)


def prune_pilots(args):
    """Only the superseded R0a package; preserve proof and the corrected B archive."""
    cpu_note("hash and prune own superseded package A, final B package retained"); no_quiet()
    old = (OUT/"ue-package").resolve()
    if old!=Path("E:/renderlab-work/m7-r0a/ue-package").resolve() or old.parent!=OUT.resolve():
        raise ValueError("unexpected obsolete package path")
    if not old.exists():
        return
    identity = json.loads((OUT/"cook/b/identity.json").read_text())
    for name, digest in identity["content"].items():
        if sha(OUT/"ue-package-b"/name)!=digest:
            raise ValueError("corrected B content is not retained")
    ledger = {str(p.relative_to(old)):dict(bytes=p.stat().st_size, sha256=sha(p)) for p in old.rglob("*") if p.is_file()}
    save(OUT/"superseded-package-a-deleted.json", dict(files=ledger, reason="corrected B package retained; A pilot captures excluded"))
    shutil.rmtree(old)


def prune_superseded_arrays(args):
    cpu_note("prune summarized superseded R0a arrays, retain PNGs/JSON/hashes"); no_quiet()
    # Final d/m references and b captures are deliberately outside this list.
    roots = [OUT/"reference"/x for x in ("pilot", "b", "c")]
    roots += [OUT/"capture"/x for x in ("pilot", "a")]
    roots += [OUT/"as-probe/rf5-build"]
    ledger = {}
    for root in roots:
        if not root.resolve().is_relative_to(OUT.resolve()):
            raise ValueError("unexpected archive path")
        for path in root.rglob("*.npz"):
            if path.stat().st_size>5*1024**2:
                ledger[str(path.relative_to(OUT))] = dict(bytes=path.stat().st_size, sha256=sha(path))
                path.unlink()
    save(OUT/"superseded-arrays-deleted.json", ledger)


def compact_reference(root, name, scene):
    score = load("m7-u0b-score")
    meta = json.loads((root/(name+".json")).read_text())
    if meta["invalid_pixels"]:
        raise ValueError("nonfinite reference")
    h, w = meta["height"], meta["width"]
    arrays = {f"L{k}": np.fromfile(root/f"{name}-half{k}.f32", "<f4").reshape(h, w, 3) for k in (0, 1)}
    geometry = root/f"{name}-geometry.bin"
    if geometry.exists():
        dtype = np.dtype([("world", "<f4", 4), ("normal", "<f4", 4), ("hit", "<u4", 4), ("direct0", "<f4", 4), ("direct1", "<f4", 4)])
        data = np.fromfile(geometry, dtype).reshape(h, w)
        arrays["hit"] = data["hit"][..., :3]
        arrays["normal"] = data["normal"][..., :3]
        # The R0a-only measurement shader records the centre ideal-reflection
        # guide's secondary placement in the otherwise unused normal.w.
        arrays["reflection_id"] = data["normal"][..., 3].astype("u4")
        arrays["reflection_world"] = data["direct0"][..., :3]
        placement = arrays["hit"][..., 0]
        for pop, names in scene["populations"].items():
            ids = [i+1 for i, m in enumerate(scene["meshes"]) if m["name"] in names]
            arrays["pop_"+pop] = np.isin(placement, ids)
        reflector_names = scene["populations"].get("reflector")
        if reflector_names is None:
            reflector_names = [name for pop, names in scene["populations"].items()
                               if pop.startswith(("metal-r", "dielectric-r")) for name in names]
            arrays["pop_reflector"] = np.isin(placement, [i+1 for i, m in enumerate(scene["meshes"]) if m["name"] in reflector_names])
        roughnesses = sorted({scene["materials"][m["material"]]["roughness"] for m in scene["meshes"] if m["name"] in reflector_names})
        for roughness in roughnesses:
            ids = [i+1 for i, m in enumerate(scene["meshes"]) if m["name"] in reflector_names
                   and scene["materials"][m["material"]]["roughness"]==roughness]
            arrays[f"pop_reflector-r{roughness:g}"] = np.isin(placement, ids)
        for band, low, high in (("B0", 0., .05), ("B1", .05, .4), ("B2", .4, 1.01)):
            ids = [i+1 for i, m in enumerate(scene["meshes"]) if m["name"] in reflector_names
                   and low<=scene["materials"][m["material"]]["roughness"]<high]
            arrays["pop_reflector-"+band] = np.isin(placement, ids)
        arrays["pop_image"] = np.ones((h, w), bool)
        pose = next(p for p in scene["poses"] if p["name"]==meta["pose"])
        eye = np.asarray(pose["eye"]); forward = np.asarray(pose["target"])-eye; forward /= np.linalg.norm(forward)
        right = np.cross(forward, [0., 1., 0.]); right /= np.linalg.norm(right)
        up = np.cross(right, forward); delta = arrays["reflection_world"]-eye
        depth = delta@forward; tan_y = math.tan(math.radians(scene["fov_y_degrees"])/2)
        offscreen = (depth<=0)|(np.abs(delta@right)>depth*tan_y*w/h)|(np.abs(delta@up)>depth*tan_y)
        arrays["pop_reflected-offscreen"] = offscreen&arrays.get("pop_reflector", False)&(arrays["reflection_id"]>0)
        for label, predicate in (("reflected-red", lambda n: n=="offscreen-red"),
                                 ("reflected-blue", lambda n: n=="offscreen-blue"),
                                 ("reflected-thin", lambda n: n.startswith(("rail-", "pole-"))),
                                 ("reflected-shell", lambda n: n=="open-shell-back"),
                                 ("reflected-toon", lambda n: n=="toon-object")):
            ids = [i+1 for i, m in enumerate(scene["meshes"]) if predicate(m["name"])]
            if ids:
                arrays["pop_"+label] = np.isin(arrays["reflection_id"], ids)&arrays.get("pop_reflector", False)
    target = root/(name+".npz")
    score.archive(target, arrays)
    # Inspectable full-frame reference; no gain fitting and no pixel mask removal.
    from PIL import Image
    image = (arrays["L0"]+arrays["L1"])/2
    Image.fromarray(np.clip(score.codes(image, -14), 0, 255).astype("u1")).save(root/(name+".png"))
    save(root/(name+".archive.json"), dict(sha256=sha(target), source_metadata_sha256=sha(root/(name+".json")),
        relative_split_noise_rms=float(np.sqrt(np.mean(((arrays["L0"]-arrays["L1"])/2)**2))/max(1e-20, np.sqrt(np.mean(image**2)))),
        note="Actual split halves retained; same-seed independent-AS replay must qualify before scoring"))
    deleted = {}
    for suffix in ("mean.f32", "se.f32", "half0.f32", "half1.f32", "geometry.bin"):
        path = root/f"{name}-{suffix}"
        if path.exists() and path.stat().st_size>5*1024**2:
            deleted[path.name] = dict(bytes=path.stat().st_size, sha256=sha(path)); path.unlink()
    save(root/(name+".deleted-raw.json"), deleted)


def reflected_thin_rois(scene, pose, arrays):
    """Fixed projected bounds include subpixel rails with no centre-guide pixel."""
    h,w = arrays["L0"].shape[:2]
    eye = np.asarray(pose["eye"], dtype="f8")
    forward = np.asarray(pose["target"], dtype="f8")-eye; forward /= np.linalg.norm(forward)
    right = np.cross(forward, [0.,1.,0.]); right /= np.linalg.norm(right)
    up = np.cross(right, forward)
    focal = h/(2*math.tan(math.radians(scene["fov_y_degrees"])/2))
    inventory = {}
    for mesh in scene["meshes"]:
        if not mesh["name"].startswith(("rail-", "pole-")):
            continue
        points = np.asarray(mesh["positions"], dtype="f8").copy(); points[:,1] *= -1
        delta = points-eye; depth = delta@forward
        if (depth<=0).any():
            raise ValueError("thin-feature bounds cross camera plane")
        x = w/2+focal*(delta@right)/depth; y = h/2-focal*(delta@up)/depth
        x0,x1 = max(0,int(np.floor(x.min()))-1), min(w,int(np.ceil(x.max()))+2)
        y0,y1 = max(0,int(np.floor(y.min()))-1), min(h,int(np.ceil(y.max()))+2)
        mask = np.zeros((h,w),bool); mask[y0:y1,x0:x1] = True
        prefix = "pop_thin-roi-"+mesh["name"]
        arrays[prefix] = mask&arrays["pop_reflector"]
        for key in list(arrays):
            if key.startswith("pop_reflector-r"):
                arrays[prefix+"-"+key.removeprefix("pop_reflector-")] = mask&arrays[key]
        inventory[mesh["name"]] = dict(bounds_xyxy=[x0,y0,x1,y1], pixels=int(arrays[prefix].sum()))
    return inventory


def assemble_references(args):
    cpu_note("assemble qualified static reference manifest, including fixed thin-feature ROIs"); no_quiet()
    root = OUT/"reference"/args.label; root.mkdir(parents=True, exist_ok=True)
    cases = {}
    for fixture in args.fixtures:
        label,spp = ("rf10-guided-hi",16384) if fixture=="RF10" else ("d-ordered",4096) if fixture=="RF8" else ("d",4096)
        source_root = OUT/"reference"/label/fixture
        complete = json.loads((source_root/"complete.json").read_text())
        scene_path = OUT/"scenes"/fixture/"scene.json"; scene = json.loads(scene_path.read_text())
        if complete["scene_sha256"]!=sha(scene_path):
            proof = json.loads((OUT/"motion"/(fixture+"-static-equivalence.json")).read_text())
            if not proof["static_inputs_equal"] or complete["scene_sha256"] not in proof.get("equivalent_source_sha256",[]):
                raise ValueError("static reference input changed")
        for pose in scene["poses"]:
            case = fixture+"-"+pose["name"]
            source = source_root/str(spp)/(case+".npz")
            meta = source.with_suffix(".json")
            if not load("m7-u0b-score").qualified_replay(json.loads(meta.read_text())):
                raise ValueError("unqualified static reference "+case)
            target = root/fixture/(case+".npz"); target.parent.mkdir(exist_ok=True)
            rois = {}
            if not target.exists():
                if fixture=="RF4":
                    with np.load(source) as data:
                        arrays = {key:data[key] for key in data.files}
                    rois = reflected_thin_rois(scene, pose, arrays)
                    load("m7-u0b-score").archive(target, arrays)
                else:
                    target.hardlink_to(source)
            elif fixture=="RF4":
                with np.load(target) as data:
                    rois = {key:dict(pixels=int(data[key].sum())) for key in data.files if key.startswith("pop_thin-roi-")}
            shutil.copyfile(meta, target.with_suffix(".json"))
            cases[case] = dict(archive=str(target.relative_to(root)), samples=spp,
                archive_sha256=sha(target), source_archive=str(source), source_archive_sha256=sha(source),
                scene_sha256=sha(scene_path), metadata_sha256=sha(meta), producer_recipe=source_root.as_posix()+"/recipe.json",
                thin_rois=rois)
    save(root/"manifest.json", dict(cases=cases, complete=len(cases)==13,
        roi_definition="RF4 virtual reflection across y=0; projected vertex bounds plus one-pixel AA margin, intersected with primary reflector/roughness masks. Conservative ROI, not fitted candidate segmentation."))


def static_reference_path(args, fixture, case):
    root = OUT/"reference"/args.reference_label
    manifest = root/"manifest.json"
    if manifest.exists():
        value = json.loads(manifest.read_text())["cases"][case]
        return root/value["archive"]
    return root/fixture/str(args.spp*2)/(case+".npz")


def measurement_shader(scene, output, ordered_hits=False, planar_sun_guide=False):
    """Test-only hit closure. The existing tracer/shader and standard outputs stay untouched.

    Toon has no scatter hook: it terminates paths at its authored radiance, with
    the local ray's view direction. RF10's front sun is unoccluded for every
    nonzero direct band. This adapter requires independent validation before scoring.
    """
    ids = [i for i, m in enumerate(scene["materials"]) if m.get("program")=="toon"]
    source = (REPO/"shaders/reference_path_tracer.wgsl").read_text()
    if ordered_hits:
        old = """rayQueryInitialize(&query,scene,RayDesc(RAY_FLAG_FORCE_OPAQUE,255u,0.0,1e30,origin,direction));
    while rayQueryProceed(&query) {}
    let h=rayQueryGetCommittedIntersection(&query);"""
        ordered = """rayQueryInitialize(&query,scene,RayDesc(RAY_FLAG_FORCE_NO_OPAQUE,255u,0.0,1e30,origin,direction));
    var h=rayQueryGetCommittedIntersection(&query);
    var closest=1e30;
    while rayQueryProceed(&query) {
        let candidate=rayQueryGetCandidateIntersection(&query);
        if candidate.kind==0u {continue;}
        let item=instances[candidate.instance_custom_data];
        let at=item.offsets.y+3u*candidate.primitive_index;
        let a=vertices[item.offsets.x+indices[at]].a.xyz;
        let b=vertices[item.offsets.x+indices[at+1u]].a.xyz;
        let c=vertices[item.offsets.x+indices[at+2u]].a.xyz;
        let ro=candidate.world_to_object*vec4<f32>(origin,1.0);
        let rd=candidate.world_to_object*vec4<f32>(direction,0.0);
        let e1=b-a;let e2=c-a;let det=dot(e1,cross(rd,e2));
        if abs(det)<=1e-20 {continue;}
        let t=dot(e2,cross(ro-a,e1))/det;
        let tolerance=4.76837158203125e-7*max(1.0,abs(t));
        let first=candidate.instance_custom_data<h.instance_custom_data ||
            (candidate.instance_custom_data==h.instance_custom_data && candidate.primitive_index<h.primitive_index);
        if t>=0.0 && (h.kind==0u || t<closest-tolerance || (abs(t-closest)<=tolerance && first)) {
            h=candidate;closest=t;
        }
    }"""
        if source.count(old)!=1:
            raise ValueError("ordered-hit diagnostic query registration changed")
        source = source.replace(old, ordered)
    if planar_sun_guide:
        # Fixed proposals, not cached light or a transport shortcut. Their full
        # mixture PDF participates in continuation and sun/sky MIS weights.
        heights = [2*math.sin((scene["sun"]["angular_radius"]+a)/2)**2 for a in (.0001, .0025, .01)]
        helper = f"""
fn r0a_guide_axis()->vec3<f32> {{return frame.sun.xyz*vec3<f32>(1.0,-1.0,1.0);}}
fn r0a_guided(l:Lobe,n:vec3<f32>)->bool {{return luminance(l.diffuse)>0.0 && dot(n,r0a_guide_axis())>0.0;}}
fn r0a_guide_height(i:u32)->f32 {{return array<f32,3>({','.join(map(str,heights))})[i];}}
fn r0a_guide_pdf(wi:vec3<f32>)->f32 {{
    let axis=r0a_guide_axis(); let sine=cross(wi,axis); var pdf=0.0;
    for(var i=0u;i<3u;i+=1u) {{let height=r0a_guide_height(i);
        if dot(wi,axis)>0.0 && dot(sine,sine)<=height*(2.0-height) {{pdf+=1.0/(6.0*PI*height);}}
    }}
    return pdf;
}}
fn r0a_guide_sample()->vec3<f32> {{
    let i=min(2u,u32(rand()*3.0));let h=rand()*r0a_guide_height(i);let phi=2.0*PI*rand();
    let radius=sqrt(h*(2.0-h));return basis(r0a_guide_axis())*vec3<f32>(radius*cos(phi),radius*sin(phi),1.0-h);
}}
fn r0a_pdf(l:Lobe,n:vec3<f32>,v:vec3<f32>,wi:vec3<f32>)->f32 {{
    let original=bsdf_pdf(l,n,v,wi);
    if r0a_guided(l,n) {{return 0.5*(original+r0a_guide_pdf(wi));}}
    return original;
}}
"""
        at = source.index("fn camera_ray_time(")
        source = source[:at]+helper+source[at:]
        start = source.index("fn radiance("); end = source.index("var<private> sky_pdf_prev", start)
        body = source[start:end].replace("bsdf_pdf(", "r0a_pdf(")
        original = """if rand()<l.p_spec {
            let local_v=transpose(frame_n)*v;
            let h=frame_n*sample_vndf(l,local_v,vec2<f32>(rand(),rand()));
            wi=reflect(-v,h);
        } else {
            let r=sqrt(rand());
            let phi=2.0*PI*rand();
            wi=frame_n*vec3<f32>(r*cos(phi),r*sin(phi),sqrt(max(0.0,1.0-r*r)));
        }"""
        if body.count(original)!=1:
            raise ValueError("reference continuation sampler changed")
        body = body.replace(original, "if r0a_guided(l,hit.n) && rand()<0.5 {wi=r0a_guide_sample();} else {\n        "+original+"\n        }")
        source = source[:start]+body+source[end:]
    needle = "total+=throughput*m.emission.rgb;"
    if source.count(needle)!=1 or (ids and scene["sun"]["to_light"][2]<=0):
        raise ValueError("toon reference registration changed")
    guide = "aux.normal=vec4<f32>(reference_centre.ng,dot(reference_centre.ng,frame.sun.xyz));"
    if source.count(guide)!=1:
        raise ValueError("reference geometry guide layout changed")
    source = source.replace("var<private> sky_pdf_prev:f32;", "var<private> sky_pdf_prev:f32;\nvar<private> r0a_guide_point:vec3<f32>;")
    source = source.replace(guide, guide+"\n        let guide_direction=reflect(ray[1],reference_centre.n);\n        let guide_hit=trace(offset(reference_centre.p,reference_centre.ng),guide_direction);\n        aux.normal.w=f32(guide_hit.identity.x);\n        r0a_guide_point=guide_hit.p;\n")
    source = source.replace("if frame.environment.w>0.0 {geometry[pixel]=aux;}",
        "if frame.environment.w>0.0 {aux.direct_even=vec4<f32>(r0a_guide_point,aux.direct_even.w);geometry[pixel]=aux;}")
    if not ids:
        output.write_text(source)
        return output
    condition = " || ".join(f"hit.material=={i}u" for i in ids)
    closure = f"""
        if {condition} {{
            let toon_nv=clamp(dot(hit.n,-direction),0.0,1.0);
            let incident=clamp((dot(hit.n,frame.sun.xyz)-0.12)/0.78,0.0,1.0);
            let band=floor(incident*3.0+0.5)/3.0;
            let tint=mix(vec3<f32>(0.12,0.08,0.24),vec3<f32>(1.0),band);
            let sun_lux=vec3<f32>({','.join(str(float(v)) for v in scene['sun']['lux'])});
            let rim=vec3<f32>(0.12,0.2,0.3)*pow(1.0-toon_nv,4.0)*{2.0**(-scene['exposure_stops'])};
            total+=throughput*(m.base.rgb*frame.sky.rgb*0.25+sun_lux*m.base.rgb*tint/PI+rim);
            break;
        }}
        """
    output.write_text(source.replace(needle, closure+needle))
    return output


def reference_pending(args):
    pending = []
    for name in args.fixtures:
        complete = OUT/"reference"/args.label/name/"complete.json"
        if not complete.exists():
            pending.append(name); continue
        record = json.loads(complete.read_text())
        if record["samples"]!=[args.spp, args.spp*2] or record.get("scene_sha256")!=sha(OUT/"scenes"/name/"scene.json"):
            pending.append(name)
    return pending


def references(args):
    while pending := reference_pending(args):
        u = ue()
        with u.lease("reference-batch-"+args.label, args.minutes) as owner:
            u.R0A_BATCH_OWNER = owner
            u.lease = lambda label, minutes: contextlib.nullcontext(owner)
            reference_cases(argparse.Namespace(**(vars(args)|dict(fixtures=pending))), u)
        if len(reference_pending(args))>=len(pending):
            raise ValueError("reference batch made no progress")


def reference_cases(args, u):
    for name in args.fixtures:
        scene_path = OUT/"scenes"/name/"scene.json"
        scene = json.loads(scene_path.read_text())
        if u.remaining_seconds(u.R0A_BATCH_OWNER)<120+60*len(scene["poses"]):
            return
        if any(m.get("program") not in (None, "toon") for m in scene["materials"]):
            raise ValueError(name+": unsupported custom-program reference")
        root = OUT/"reference"/args.label/name
        if (root/"complete.json").exists() and json.loads((root/"complete.json").read_text())["samples"]==[args.spp, args.spp*2] and json.loads((root/"complete.json").read_text()).get("scene_sha256")==sha(scene_path):
            continue
        root.mkdir(parents=True, exist_ok=True)
        binary = REFERENCE_BINARY
        shader = measurement_shader(scene, root/"measurement-reference.wgsl", getattr(args,"ordered_hits",False), getattr(args,"planar_sun_guide",False))
        previous_recipe = json.loads((root/"recipe.json").read_text()) if (root/"recipe.json").exists() else {}
        recover_inputs = previous_recipe.get("binary_sha256")==sha(binary) and previous_recipe.get("scene_sha256")==sha(scene_path) and previous_recipe.get("shader_override_sha256")==sha(shader)
        if previous_recipe and not recover_inputs:
            raise ValueError("reference binary, shader or scene changed; choose a fresh --label")
        previous = root/"process"
        if previous.exists():
            archived = root/("process-attempt-"+str(time.time_ns()))
            previous.rename(archived)
            if (root/"recipe.json").exists():
                shutil.copyfile(root/"recipe.json", archived/"parent-recipe.json")
            for path in root.glob("[0-9]*/*.json"):
                destination = archived/"state-metadata"/path.relative_to(root)
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(path, destination)
        # A process interrupted by a resource/conflict guard may have completed
        # earlier states. Preserve and reuse only independently qualified states.
        for spp in (args.spp, args.spp*2):
            run = root/str(spp)
            for pose in scene["poses"]:
                case = name+"-"+pose["name"]
                meta = run/(case+".json")
                if recover_inputs and meta.exists() and not (run/(case+".npz")).exists() and (run/(case+"-half0.f32")).exists():
                    value = json.loads(meta.read_text())
                    if value.get("scene_sha256")==sha(scene_path) and load("m7-u0b-score").qualified_replay(value):
                        cpu_note("recover qualified reference state "+case+" "+str(spp))
                        compact_reference(run, case, scene)
        if previous_recipe:
            for spp in (args.spp, args.spp*2):
                cleanup_raw(root/str(spp))
        env = dict(os.environ, RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
            RENDERLAB_M7_REFERENCE_OUT=str(root), RENDERLAB_M7_R0A_SCENES=str(scene_path.parent),
            RENDERLAB_M7_R0A_SAMPLES=f"{args.spp},{args.spp*2}",
            RENDERLAB_M7_MAX_DEPTH=str(args.depth),
            RENDERLAB_M7_R0A_AS_BUILD=getattr(args,"as_build","trace"),
            RENDERLAB_M7_SHADER_OVERRIDE=str(shader), VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation")
        env.pop("VK_INSTANCE_LAYERS", None)
        with u.lease("references-"+name, args.minutes) as owner:
            save(root/"recipe.json", dict(binary_sha256=sha(binary), scene_sha256=sha(scene_path),
                shader_override_sha256=sha(shader), lease=owner,
                environment={k:v for k,v in env.items() if k.startswith("RENDERLAB_M7_")}, timing_eligible_for_claim=False))
            u.guarded([binary, "reference::r0a_tests::gpu_r0a_references", "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                root/"process", timeout=min(600, int(u.remaining_seconds(owner))-10), cwd=REPO, env=env)
        cpu_note("archive per-state qualified references "+name)
        for spp in (args.spp, args.spp*2):
            run = root/str(spp)
            if (run/"complete.json").exists():
                continue
            for pose in scene["poses"]:
                case = name+"-"+pose["name"]
                metadata = json.loads((run/(case+".json")).read_text())
                if not load("m7-u0b-score").qualified_replay(metadata):
                    raise ValueError("unqualified per-state independent-AS replay")
                if not (run/(case+".npz")).exists():
                    compact_reference(run, case, scene)
            save(run/"complete.json", dict(strict_replay=True, samples=spp, max_depth=args.depth))
        comparisons = {}
        lower_archives = {}
        for pose in scene["poses"]:
            case = name+"-"+pose["name"]
            low = root/str(args.spp)/(case+".npz")
            with np.load(low) as a, np.load(root/str(args.spp*2)/(case+".npz")) as b:
                A=(a["L0"]+a["L1"])/2; B=(b["L0"]+b["L1"])/2
                comparisons[case] = dict(doubling_rgb_relative_rms=float(np.sqrt(np.mean((A-B)**2)/max(1e-20, np.mean(B**2)))),
                    relative_energy_change=float((A.sum(dtype="f8")-B.sum(dtype="f8"))/max(1e-20, B.sum(dtype="f8"))))
            lower_archives[low.name] = dict(bytes=low.stat().st_size, sha256=sha(low))
        save(root/"complete.json", dict(samples=[args.spp, args.spp*2], comparisons=comparisons,
            scene_sha256=sha(scene_path),
            lower_sample_archives=lower_archives,
            reference_qualification="Convergence metrics recorded; per-population noise and final image must be reviewed before a floor claim"))
        for pose in scene["poses"]:
            low = root/str(args.spp)/(name+"-"+pose["name"]+".npz")
            if low.stat().st_size>5*1024**2:
                low.unlink()


def prepare(args):
    u = ue(); out = OUT/"import"; out.mkdir(parents=True, exist_ok=True)
    with u.lease("import-reflection-fixtures", 35) as owner:
        first = max((int(p.name[8:]) for p in out.glob("attempt-*") if p.name[8:].isdigit()), default=-1)+1
        for attempt in range(first, first+12):
            run = out/f"attempt-{attempt}"
            if run.exists():
                continue
            try:
                u.guarded([u.EDITOR, u.PROJECT, *u.UE_FLAGS, *u.COMMON_INI, "-NullRHI", "-run=pythonscript",
                    "-script="+str(REPO/"scripts/m7-r0a-ue-prepare.py"), "-abslog="+str(run/"ue.log")],
                    run, timeout=int(u.remaining_seconds(owner))-15,
                    env=dict(os.environ, RENDERLAB_R0A_FIXTURES=",".join(args.fixtures)))
                break
            except RuntimeError:
                log = (run/"stdout.log").read_text(errors="replace")
                if "Assertion failed: CurrentApplication.IsValid()" not in log or (out/"failure.txt").exists():
                    raise
        else:
            raise RuntimeError("twelve import attempts exhausted; no successful commandlet completion")
        if not all((out/(name+".json")).exists() for name in args.fixtures):
            raise ValueError("fixture imports incomplete")
        if "RF10" in args.fixtures:
            check_toon_adapter()


def check_toon_adapter():
    marker = OUT/"import/toon-adapter.json"
    adapter = json.loads(marker.read_text())
    scene = json.loads((OUT/"scenes/RF10/scene.json").read_text())
    x,y,z = scene["sun"]["to_light"]
    if adapter.get("source_scene_sha256")!=sha(OUT/"scenes/RF10/scene.json") or adapter.get("adapter_source_sha256")!=sha(REPO/"scripts/m7-r0a-ue-prepare.py") or not np.allclose(adapter.get("sun_world", []), [-z,x,y], atol=1e-12, rtol=0):
        raise ValueError("RF10 material source/basis not qualified for cooking")
    return adapter


def cook(args):
    check_toon_adapter()
    u = ue(); out = OUT/"cook"/args.label; out.mkdir(parents=True, exist_ok=True)
    archive = OUT/args.package
    maps = ["/Game/M7R0a/"+name+"/Bench" for name in args.fixtures]
    with u.lease("cook-"+args.label, 35) as owner:
        u.guarded([u.EDITOR, u.PROJECT, *u.UE_FLAGS, *u.COMMON_INI, "-NullRHI", "-run=cook", "-TargetPlatform=Windows",
            "-Map="+"+".join(maps), "-unversioned", "-skipeditorcontent", "-cookcultures=en", "-NoZenAutoLaunch",
            "-abslog="+str(out/"cook/ue.log")], out/"cook", timeout=int(u.remaining_seconds(owner))-300)
        u.guarded([os.environ["COMSPEC"], "/d", "/c", u.UAT, "BuildCookRun", f"-project={u.PROJECT}", "-nop4", "-skipbuild",
            "-skipcook", "-stage", "-pak", "-archive", f"-archivedirectory={archive}", "-clientconfig=Development",
            "-targetplatform=Win64", "-unattended", "-utf8output", "-nocompile", "-nocompileuat", "-installed"],
            out/"package", timeout=int(u.remaining_seconds(owner))-10)
    save(out/"identity.json", dict(parent="E:/renderlab-work/m7-u0b/cook/b/identity.json", parent_sha256=sha(Path("E:/renderlab-work/m7-u0b/cook/b/identity.json")),
        toon_adapter=check_toon_adapter(),
        executable_sha256=sha(archive/"Windows/Engine/Binaries/Win64/UnrealGame.exe"), maps=maps,
        content={str(p.relative_to(archive)): sha(p) for p in archive.rglob("*") if p.suffix in (".pak", ".utoc", ".ucas")}))


def cleanup_stage(args):
    """Remove only the staging copy this unit created, after a complete hash proof."""
    u = ue(); stage = Path("E:/renderlab-work/m7-u0/ue-project/Saved/StagedBuilds")
    expected = Path("E:/renderlab-work/m7-u0/ue-project/Saved/StagedBuilds").resolve()
    archive = OUT/args.package
    if not (OUT/"cook"/args.cook_label/"identity.json").exists():
        raise ValueError("no successful R0a package identity")
    if not stage.exists():
        return
    with u.lease("staging-retention-proof", 3):
        cpu_note("hash duplicated R0a Unreal staging files before removal")
        if stage.resolve()!=expected or expected.parent.name!="Saved":
            raise ValueError("unexpected staging cleanup target")
        records = []
        for source in stage.rglob("*"):
            if not source.is_file():
                continue
            relative = source.relative_to(stage); kept = archive/relative
            if not kept.is_file() or sha(source)!=sha(kept):
                raise ValueError("staging archive mismatch; retained "+str(relative))
            records.append(dict(path=str(relative), bytes=source.stat().st_size, sha256=sha(kept)))
        save(OUT/"staging-retention-proof.json", dict(stage=str(stage), kept=str(archive), files=records,
            total_bytes=sum(r["bytes"] for r in records), mismatches=0))
        shutil.rmtree(stage)
        if stage.exists():
            raise ValueError("staging cleanup did not finish")


def configurations(u):
    configs = {}
    for quality, suffix in ((3, "epic"), (4, "cine")):
        for mode, backend, lighting in (("sw-mesh", "sw", 0), ("hw-sc", "hw", 0), ("hw-hit", "hw", 2)):
            c = u.spec(backend, quality, lighting, shadows="hrt" if backend=="hw" else "vsm",
                mesh_sdfs=1 if backend=="sw" else None)
            if backend=="sw":
                c["commands"] += ["r.RayTracing.Enable 0"]
                c["required"] += [("r.RayTracing.Enable", "0")]
                c["ini"] += ["-ini:Engine:[/Script/Engine.RendererSettings]:r.RayTracing.EnableOnDemand=1"]
            else:
                c["ini"] += ["-ini:Engine:[/Script/Engine.RendererSettings]:r.DistanceFields.SupportEvenIfHardwareRayTracingSupported=0"]
            c["commands"] += ["r.OneFrameThreadLag 0", "r.SSR.Quality 0"]
            c["required"] += [("r.OneFrameThreadLag", "0"), ("r.SSR.Quality", "0")]
            configs[mode+"-"+suffix] = c
    for ssr in (0, 1):
        c = u.spec("sw", 3, reflections="none", mesh_sdfs=1)
        c["commands"] += ["r.ReflectionMethod "+str(2 if ssr else 0), "r.SSR.Quality "+str(4 if ssr else 0),
            "r.OneFrameThreadLag 0", "r.RayTracing.Enable 0"]
        c["required"] = [(k, v) for k, v in c["required"] if k!="r.ReflectionMethod"]
        c["required"] += [("r.ReflectionMethod", str(2 if ssr else 0)), ("r.SSR.Quality", str(4 if ssr else 0)),
            ("r.OneFrameThreadLag", "0"), ("r.RayTracing.Enable", "0")]
        c["ini"] += ["-ini:Engine:[/Script/Engine.RendererSettings]:r.RayTracing.EnableOnDemand=1"]
        configs["ssr-"+("on" if ssr else "off")+"-informative"] = c
    return configs


def capture_pending(args):
    paths = []
    for name in args.fixtures:
        scene = json.loads((OUT/"scenes"/name/"scene.json").read_text())
        for pose in scene["poses"]:
            for config in args.configs:
                root = OUT/"capture"/args.label/name/pose["name"]/config
                if not (root/"complete.json").exists():
                    paths.append(root)
    return paths


def capture(args):
    while pending := capture_pending(args):
        u = ue()
        with u.lease("capture-batch-"+args.label, args.minutes) as owner:
            u.R0A_BATCH_OWNER = owner
            u.lease = lambda label, minutes: contextlib.nullcontext(owner)
            capture_cases(args, u)
        if len(capture_pending(args))>=len(pending):
            raise ValueError("capture batch made no progress")


def capture_cases(args, u):
    b = load("m7-u0b"); score = load("m7-u0b-score")
    u.PACKAGE_DIR = OUT/args.package
    u.PACKAGE = u.PACKAGE_DIR/"Windows/Engine/Binaries/Win64/UnrealGame.exe"
    u.CONFIGS = configurations(u)
    u.UE_FLAGS += ["-UseFixedTimeStep", "-FPS=60"]
    for fixture in args.fixtures:
        source = OUT/"scenes"/fixture/"scene.json"; scene = json.loads(source.read_text())
        for pose in scene["poses"]:
            for config in args.configs:
                out = OUT/"capture"/args.label/fixture/pose["name"]/config
                if (out/"complete.json").exists():
                    continue
                if u.remaining_seconds(u.R0A_BATCH_OWNER)<90:
                    return
                out.mkdir(parents=True, exist_ok=True)
                for name in ("alias.json", "exclusion.json"):
                    old = out/name
                    if old.exists():
                        old.replace(out/("superseded-pilot-"+name))
                extra = [u.macro(out, "pose", b.pose_commands(pose)),
                    f"set DirectionalLightComponent LightSourceAngle {math.degrees(2*scene['sun']['angular_radius']):.9f}",
                    "r.Shadow.CacheWPOPrimitives 1", "r.Shadow.CacheWPOPrimitives 0", *u.READBACK,
                    "r.DumpGPU.Upload 0", "r.DumpGPU.Explore 0", "r.DumpGPU.Stream 0", "r.DumpGPU.Texture 2",
                    "r.DumpGPU.Buffer 2", "r.DumpGPU.PassParameters 1", "r.DumpGPU.FrameCount 1", "r.DumpGPU.Root Tonemap*",
                    f"r.DumpGPU.Directory {(out/'hdr').as_posix()}"]
                events = [(178, "getall PlayerCameraManager CameraCachePrivate"), (180, "DumpGPU")]
                with u.lease("capture-"+fixture+"-"+config, args.minutes):
                    u.launch_ue(out, config, "static", extra=extra, events=events, frames=220, level=f"/Game/M7R0a/{fixture}/Bench")
                    if not u.readback(out, config, u.scalability_expectations()):
                        raise ValueError("effective mode/settings mismatch: "+str(out))
                    camera = b.check_camera((out/"ue.log").read_text(encoding="utf-8-sig", errors="replace"),
                        [dict(pose, fov_y_radians=math.radians(scene["fov_y_degrees"]))])
                    save(out/"camera-readback.json", camera)
                    save(out/"scene-readback.json", check_scene_light((out/"ue.log").read_text(encoding="utf-8-sig", errors="replace"), scene))
                    summary = u.PROFILE.summarize(out/"profile.csv")
                    summary["pass_costs_async_off"] = u.pass_costs(out)
                    summary["timing_eligible_for_claim"] = False
                    save(out/"summary.json", summary)
                cpu_note("decode native HDR "+fixture+" "+config); no_quiet()
                score.decode(argparse.Namespace(dump=out/"hdr", out=out/"native.npz", stops=scene["exposure_stops"]))
                save(out/"complete.json", dict(scene_sha256=sha(source), package_identity_sha256=sha(OUT/"cook"/args.cook_label/"identity.json"),
                    scalability_sha256=sha(Path("C:/Program Files/Epic Games/UE_5.7/Engine/Config/BaseScalability.ini")),
                    floor_eligible="informative" not in config, timing_eligible_for_claim=False))


def baseline(args):
    u = ue(); binary = OUT/"bin/r0a-tests.exe"
    for backend in ("nonrt", "rt"):
        root = OUT/"current"/args.label/backend
        root.mkdir(parents=True, exist_ok=True)
        if (root/"complete.json").exists():
            continue
        previous = root/"process"
        if previous.exists():
            previous.rename(root/("process-attempt-"+str(time.time_ns())))
        paths = [OUT/"scenes"/fixture for fixture in args.fixtures]
        env = dict(os.environ, RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
            RENDERLAB_M7_R0A_SCENES=";".join(map(str, paths)), RENDERLAB_M7_R0A_PATH=backend,
            RENDERLAB_M7_REFERENCE_OUT=str(root), VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation")
        env.pop("VK_INSTANCE_LAYERS", None)
        with u.lease("current-"+backend, args.minutes) as owner:
            u.guarded([binary, "reference::r0a_tests::gpu_r0a_current", "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                root/"process", timeout=min(600, int(u.remaining_seconds(owner))-10), cwd=REPO, env=env)
        cpu_note("compact current-renderer HDR "+backend)
        score = load("m7-u0b-score")
        for path in root.glob("*.f32"):
            meta = json.loads(path.with_suffix(".json").read_text())
            image = np.fromfile(path, "<f4").reshape(meta["height"], meta["width"], 3)
            score.archive(path.with_suffix(".npz"), dict(L=image))
            from PIL import Image
            Image.fromarray(np.clip(score.codes(image, -14), 0, 255).astype("u1")).save(path.with_suffix(".png"))
        cleanup_raw(root)
        save(root/"complete.json", dict(binary_sha256=sha(binary), fixtures=args.fixtures,
            path=backend, timing_eligible_for_claim=False, runtime_source_head=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip()))


def crosschecks(args):
    cpu_note("generate independent Mitsuba recursive/toon controls"); no_quiet()
    fixtures = load("m7-r0a-scenes")
    names = []
    for source in (s for s in ("RF7", "RF10") if s in args.fixtures):
        scene = copy.deepcopy(fixtures.BENCHES[source]())
        widened_sun = getattr(args,"control_sun",None)
        mirror_control = getattr(args,"mirror_control",False)
        name = "X"+source+(("M" if mirror_control else "G") if source=="RF10" else "")+("W" if widened_sun is not None else "")
        names.append(name)
        scene.update(bench=name, size=[96, 64], smith="separable", camera_background=None)
        if widened_sun is not None:
            scene["sun"]["angular_radius"] = widened_sun
            scene["registration"]["independent_control_sun"] = "wider angular disk tests far-sphere emitter precision; primary RF fixtures unchanged"
        for material in scene["materials"]:
            if material["metallic"]:
                material["base_color"] = [1., 1., 1.]
                if source=="RF10" and not mirror_control:
                    # The failed mirror control is retained as XRF10. Use a broad
                    # validated GGX lobe to isolate the custom hit closure from
                    # heavy-tailed, subpixel sun hits; RF10 itself is unchanged.
                    material["roughness"] = max(.32, material["roughness"])
            else:
                material["specular"] = 0.
        directory = OUT/"scenes"/name; directory.mkdir(parents=True, exist_ok=True)
        save(directory/"scene.json", scene)
        writer_scene = copy.deepcopy(scene)
        for material in writer_scene["materials"]:
            if material.get("program")=="toon":
                material["base_color"] = [0., 0., 0.]
        fixtures.U0.write_mitsuba(writer_scene, directory/"mitsuba")
    reference_label = "oracle" if args.label=="a" else args.label
    references(argparse.Namespace(fixtures=names, label=reference_label, spp=args.spp, depth=1024, minutes=12,
        as_build=getattr(args,"as_build","trace"), ordered_hits=getattr(args,"ordered_hits",False), planar_sun_guide=getattr(args,"planar_sun_guide",False)))
    u = ue()
    interpreter = Path("E:/renderlab-work/m7-u0/tools/mitsuba-venv/Scripts/python.exe")
    for name in names:
        root = OUT/"mitsuba"/name; root.mkdir(parents=True, exist_ok=True)
        mitsuba_spp = getattr(args,"mitsuba_spp",8192)
        if mitsuba_spp!=8192:
            root = root/f"spp{mitsuba_spp}"; root.mkdir(parents=True, exist_ok=True)
        for seed in (1, 2):
            target = root/f"seed{seed}.npy"
            process_path = root/f"process-{seed}.json"
            cached = json.loads(process_path.read_text()) if process_path.exists() else {}
            if target.exists() and cached.get("exit_code")==0 and not cached.get("failures"):
                continue
            attempt = str(time.time_ns())
            for old in (root/f"stdout-{seed}.log", process_path, target, target.with_suffix(".json")):
                if old.exists():
                    old.rename(old.with_name(old.stem+"-previous-"+attempt+old.suffix))
            cpu_note(f"Mitsuba {name} seed {seed}, two CPU threads, no GPU"); no_quiet()
            if u.E2E.available_gib()<4 or shutil.disk_usage(OUT).free<5*2**30:
                raise RuntimeError("Mitsuba resource start guard")
            command = [interpreter, "-I", REPO/"scripts/m7-r0a-mitsuba.py", OUT/"scenes"/name, target, "--spp", str(mitsuba_spp), "--seed", str(seed)]
            log = root/f"stdout-{seed}.log"
            with log.open("w") as stream:
                child = subprocess.Popen(command, stdout=stream, stderr=subprocess.STDOUT, creationflags=subprocess.CREATE_NO_WINDOW)
                failures = []; memory = []; disk = []
                try:
                    while child.poll() is None:
                        ram = u.E2E.available_gib(); free = shutil.disk_usage(OUT).free/2**30
                        memory.append(ram); disk.append(free)
                        if ram<2 or free<2 or active_quiet(schedule_text(), owner_now()):
                            failures.append("CPU oracle resource guard or foreign QUIET")
                            subprocess.run(["taskkill", "/PID", str(child.pid), "/T", "/F"], capture_output=True, creationflags=subprocess.CREATE_NO_WINDOW)
                            break
                        time.sleep(.5)
                    child.wait()
                except Exception as error:
                    failures.append("CPU oracle monitor unavailable: "+repr(error))
                finally:
                    if child.poll() is None:
                        subprocess.run(["taskkill", "/PID", str(child.pid), "/T", "/F"], capture_output=True, creationflags=subprocess.CREATE_NO_WINDOW)
                        child.wait()
            save(process_path, dict(command=list(map(str, command)), exit_code=child.returncode,
                failures=failures, minimum_available_gib=min(memory, default=None), minimum_free_gib=min(disk, default=None)))
            if child.returncode or failures:
                raise RuntimeError(log.read_text()[-4000:])
        scene = json.loads((OUT/"scenes"/name/"scene.json").read_text())
        load("m7-u0-crosscheck").compare(argparse.Namespace(ours=OUT/"reference"/reference_label/name/str(args.spp*2),
            name=name+"-"+scene["poses"][0]["name"], mitsuba=[root/"seed1.npy", root/"seed2.npy"], stops=-14))
        report = OUT/"reference"/reference_label/name/str(args.spp*2)/(name+"-"+scene["poses"][0]["name"]+"-crosscheck.json")
        if json.loads(report.read_text())["pass"] is not True:
            raise ValueError("independent reference gate failed: "+str(report))


def adopt_oracle(args):
    """Reuse already qualified per-state evidence; never promote a rejected state."""
    cpu_note("reuse independently qualified 4096/8192-spp oracle states; failed states remain excluded"); no_quiet()
    for name in ("XRF7", "XRF10G"):
        scene = json.loads((OUT/"scenes"/name/"scene.json").read_text())
        case = name+"-"+scene["poses"][0]["name"]
        root = OUT/"reference"/args.label/name
        provenance = []
        for spp, label in ((4096, "oracle4"), (8192, "oracle-build" if name=="XRF10G" else "oracle4")):
            source = OUT/"reference"/label/name/str(spp)
            meta = json.loads((source/(case+".json")).read_text())
            if not load("m7-u0b-score").qualified_replay(meta):
                raise ValueError("cannot adopt an unqualified reference state")
            recipe = json.loads((source.parent/"recipe.json").read_text())
            if recipe["scene_sha256"]!=sha(OUT/"scenes"/name/"scene.json"):
                raise ValueError("oracle source input changed")
            target = root/str(spp); target.mkdir(parents=True, exist_ok=True)
            files = {}
            for path in source.glob(case+"*"):
                if path.is_file():
                    shutil.copyfile(path, target/path.name); files[path.name] = sha(path)
            compact_reference(target, case, scene)
            provenance.append(dict(spp=spp, source=str(source), files=files, recipe=recipe,
                qualification=meta["replay_qualification"]))
        save(root/"complete.json", dict(samples=[4096, 8192], scene_sha256=sha(OUT/"scenes"/name/"scene.json"),
            adopted_qualified_states=provenance, policy="unchanged bounded-as-v1; every selected state already accepted independently; failed higher states excluded"))


def probe_as(args):
    """Short paired screen, then eight retained replays if the screen qualifies."""
    u = ue(); binary = REFERENCE_BINARY
    fixture = args.fixtures[0]; source = OUT/"scenes"/fixture/"scene.json"
    scene = json.loads(source.read_text()); root = OUT/"as-probe"/args.label
    root.mkdir(parents=True, exist_ok=True)
    shader = measurement_shader(scene, root/"measurement.wgsl", getattr(args,"ordered_hits",False), getattr(args,"planar_sun_guide",False))
    reports = []
    with u.lease("as-probe-"+args.label, args.minutes) as owner:
        for repeat in range(args.repeats):
            run = root/f"repeat-{repeat:02d}"
            run.mkdir(exist_ok=True)
            env = dict(os.environ, RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
                RENDERLAB_M7_REFERENCE_OUT=str(run), RENDERLAB_M7_R0A_SCENES=str(source.parent),
                RENDERLAB_M7_R0A_SAMPLES=str(args.spp), RENDERLAB_M7_R0A_AS_BUILD=args.as_build,
                RENDERLAB_M7_SHADER_OVERRIDE=str(shader), VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation")
            env.pop("VK_INSTANCE_LAYERS", None)
            u.guarded([binary,"reference::r0a_tests::gpu_r0a_references","--exact","--ignored","--nocapture","--test-threads=1"],
                run/"process", timeout=min(600, int(u.remaining_seconds(owner))-10), cwd=REPO, env=env)
            for pose in scene["poses"][:1]:
                name = fixture+"-"+pose["name"]; raw = run/str(args.spp)
                meta = json.loads((raw/(name+".json")).read_text())
                if not load("m7-u0b-score").qualified_replay(meta):
                    raise ValueError("AS probe rejected by existing policy")
                reports.append(dict(repeat=repeat, qualification=meta["replay_qualification"]))
                compact_reference(raw, name, scene)
                save(root/"progress.json", dict(reports=reports, repeats_requested=args.repeats,
                    binary_sha256=sha(binary), scene_sha256=sha(source), shader_sha256=sha(shader), preference=args.as_build))
    save(root/"complete.json", dict(reports=reports, repeats_requested=args.repeats, preference=args.as_build,
        policy="bounded-as-v1; no changed threshold", accepted=len(reports)==args.repeats))


def compatibility(args):
    u = ue(); root = OUT/"compatibility"/args.label; root.mkdir(parents=True, exist_ok=True)
    binaries = {"old":PINNED, "new":OUT/"bin/r0a-tests.exe"}
    with u.lease("reference-default-compatibility", args.minutes) as owner:
        for name, binary in binaries.items():
            target = root/name
            env = dict(os.environ, RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
                RENDERLAB_M7_REFERENCE_OUT=str(target), VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation")
            env.pop("VK_INSTANCE_LAYERS", None); env.pop("RENDERLAB_M7_SHADER_OVERRIDE", None)
            u.guarded([binary,"reference::tests::gpu_reference_analytic","--exact","--ignored","--nocapture","--test-threads=1"],
                target/"process", timeout=min(600,int(u.remaining_seconds(owner))-10), cwd=REPO, env=env)
    pairs = []
    for old in (root/"old").iterdir():
        if old.suffix not in (".f32", ".bin"):
            continue
        new = root/"new"/old.name
        pairs.append(dict(file=old.name, old_sha256=sha(old), new_sha256=sha(new), identical=sha(old)==sha(new)))
    save(root/"comparison.json", dict(files=pairs, byte_identical=bool(pairs) and all(p["identical"] for p in pairs),
        old_binary_sha256=sha(PINNED), new_binary_sha256=sha(binaries["new"]),
        default_build_preference="PREFER_FAST_TRACE, unchanged", shader="unchanged production reference shader, no override"))
    if not pairs or not all(p["identical"] for p in pairs):
        raise ValueError("original reference output compatibility failed")


def synchronize_motion_tracks(args=None):
    """Correct event indexing without changing any imported static render input."""
    fixtures = load("m7-r0a-scenes")
    for name in ("RF3", "RF5"):
        path = OUT/"scenes"/name/"scene.json"
        before = json.loads(path.read_text()); intended = fixtures.BENCHES[name]()["tracks"]
        if before["tracks"]==intended:
            continue
        cpu_note("align "+name+" first motion with frame 32; preserve static-input equivalence proof"); no_quiet()
        old_sha = sha(path)
        imported_path = OUT/"motion"/(name+"-imported-source.json")
        if not imported_path.exists():
            save(imported_path, before)
        proof_path = OUT/"motion"/(name+"-static-equivalence.json")
        prior = json.loads(proof_path.read_text()) if proof_path.exists() else {}
        fixtures.generate(OUT/"scenes", [name])
        after = json.loads(path.read_text())
        old_static = {k:v for k,v in before.items() if k!="tracks"}
        new_static = {k:v for k,v in after.items() if k!="tracks"}
        if old_static!=new_static:
            raise ValueError("motion correction changed a baked/static input")
        digest = hashlib.sha256(json.dumps(old_static, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        if prior and prior["static_input_sha256"]!=digest:
            raise ValueError("static-input equivalence chain changed")
        equivalent = set(prior.get("equivalent_source_sha256",[]))|{old_sha,sha(path)}
        equivalent.update(v for k,v in prior.items() if k in ("imported_source_sha256","corrected_source_sha256"))
        save(proof_path, dict(imported_source_sha256=prior.get("imported_source_sha256",old_sha),
            corrected_source_sha256=sha(path), static_inputs_equal=True, static_input_sha256=digest,
            equivalent_source_sha256=sorted(equivalent),
            difference="tracks only; native maps contain no authored track. Runtime macros and path snapshots both read corrected track"))


def motion_scenes(args):
    synchronize_motion_tracks()
    cpu_note("generate motion witnesses and 8/16-node finite shutters"); no_quiet()
    fixtures = load("m7-r0a-scenes")
    manifest = {}
    for name in ("RF3", "RF5"):
        source = OUT/"scenes"/name/"scene.json"; scene = json.loads(source.read_text())
        states = []
        def write(frame, time_sample, label):
            state = fixtures.state_at(scene, scene["tracks"][0]["name"], time_sample)
            frozen = fixtures.snapshot(scene, state)
            case = f"{name}-f{frame:03d}-{label}"; frozen["bench"] = case
            save(OUT/"scenes"/case/"scene.json", frozen)
            states.append(dict(name=case, frame=frame, time=time_sample, state=state, shutter=label))
        for frame in scene["registration"]["witness_frames"]:
            write(frame, frame, "zero")
        for frame in (48, 64):
            for nodes in (8, 16):
                for k in range(nodes):
                    time_sample = frame+((k+.5)/nodes-.5)*.5
                    write(frame, time_sample, f"sh{nodes}-t{k:02d}")
        manifest[name] = dict(source_sha256=sha(source), states=states,
            quadrature="uniform midpoint, centred 180-degree shutter at 60 Hz; common camera/object/sun time",
            motion_reference="Zero-shutter centres and finite-shutter images are scored separately")
    save(OUT/"motion/manifest.json", manifest)
    return manifest


def motion_references(args):
    manifest = motion_scenes(args)
    for fixture, registration in manifest.items():
        if fixture not in args.fixtures:
            continue
        integrated = [OUT/"motion/reference"/f"{fixture}-f{frame:03d}-sh{nodes}.npz" for frame in (48, 64) for nodes in (8, 16)]
        finite_complete = all(p.exists() and p.with_suffix(".json").exists() for p in integrated)
        names = [s["name"] for s in registration["states"] if s["shutter"]=="zero"]
        references(argparse.Namespace(fixtures=names, label=args.label, spp=args.spp, depth=1024,
            minutes=args.minutes, as_build=args.as_build, ordered_hits=args.ordered_hits))
        if finite_complete:
            continue
        score = load("m7-u0b-score")
        for frame in (48, 64):
            for nodes in (8, 16):
                target = OUT/"motion/reference"/f"{fixture}-f{frame:03d}-sh{nodes}.npz"
                if target.exists() and target.with_suffix(".json").exists():
                    continue
                states = [s for s in registration["states"] if s["frame"]==frame and s["shutter"].startswith(f"sh{nodes}-")]
                references(argparse.Namespace(fixtures=[s["name"] for s in states], label=args.label,
                    spp=max(2, args.spp//16), depth=1024, minutes=args.minutes, as_build=args.as_build, ordered_hits=args.ordered_hits))
                halves = [None, None]
                members = []
                for state in states:
                    path = OUT/"reference"/args.label/state["name"]/str(max(2, args.spp//16)*2)/(state["name"]+"-centre.npz")
                    with np.load(path) as image:
                        for k in (0, 1):
                            value = image[f"L{k}"].astype("f8")/nodes
                            halves[k] = value if halves[k] is None else halves[k]+value
                    members.append(dict(archive=str(path), sha256=sha(path), time=state["time"]))
                if len(members)!=nodes:
                    raise ValueError("incomplete shutter quadrature")
                target.parent.mkdir(parents=True, exist_ok=True)
                score.archive(target, {f"L{k}": halves[k].astype("f4") for k in (0, 1)})
                save(target.with_suffix(".json"), dict(members=members, definition=registration["quadrature"],
                    convergence="compare 8 versus 16 time nodes; actual split halves include shared-time covariance"))
                # Bound peak disk usage to one quadrature. The integrated halves
                # and member hashes retain the measurement after summarizing.
                for member in members:
                    path = Path(member["archive"])
                    if path.stat().st_size>5*1024**2:
                        path.unlink()


def motion_baseline(args):
    manifest = json.loads((OUT/"motion/manifest.json").read_text())
    names = [s["name"] for registration in manifest.values() for s in registration["states"] if s["shutter"]=="zero"]
    baseline(argparse.Namespace(label=args.label+"-motion", fixtures=names, minutes=args.minutes))


def motion_capture(args):
    synchronize_motion_tracks()
    u = ue(); b = load("m7-u0b"); fixtures = load("m7-r0a-scenes")
    u.PACKAGE_DIR = OUT/args.package; u.PACKAGE = u.PACKAGE_DIR/"Windows/Engine/Binaries/Win64/UnrealGame.exe"
    u.CONFIGS = configurations(u); u.UE_FLAGS += ["-UseFixedTimeStep", "-FPS=60"]
    groups = [(31, 9), (48, 1), (64, 1), (79, 9), (95, 1)]
    for fixture in ("RF3", "RF5"):
        source = OUT/"scenes"/fixture/"scene.json"; scene = json.loads(source.read_text())
        imported = json.loads((OUT/"import"/(fixture+".json")).read_text())
        actor_names = {m["source"]: m["actor"] for m in imported["meshes"]}
        for config in args.configs:
            out = OUT/"motion/capture"/args.label/fixture/config
            if (out/"complete.json").exists():
                continue
            out.mkdir(parents=True, exist_ok=True)
            macros = OUT/"m"/hashlib.sha256(str(out).encode()).hexdigest()[:8]
            macros.mkdir(parents=True, exist_ok=True)
            events = []; expected = []
            for i in range(97):
                commands = []
                if i:
                    commands += ["getall PlayerCameraManager CameraCachePrivate", "getall StaticMeshComponent RelativeLocation",
                                 "getall DirectionalLightComponent RelativeRotation"]
                if i<96:
                    state = fixtures.state_at(scene, scene["tracks"][0]["name"], i)
                    expected.append(dict(state, fov_y_radians=math.radians(scene["fov_y_degrees"])))
                    commands += b.pose_commands(state)
                    for name, (x, y, z) in state["movers"].items():
                        commands += [f'ke {actor_names[name]} K2_SetActorLocation "(X={-100*z:.9f},Y={100*x:.9f},Z={100*y:.9f})" false () false']
                    x, y, z = state["to_light"]
                    pitch = math.degrees(math.atan2(-y, math.hypot(x, z)))
                    yaw = math.degrees(math.atan2(-x, z))
                    commands += [f'ke DirectionalLight K2_SetActorRotation "(Pitch={pitch:.9f},Yaw={yaw:.9f},Roll=0)" false']
                for first, count in groups:
                    if i==first-1:
                        commands += [f"r.DumpGPU.FrameCount {count}", f"r.DumpGPU.Directory {(out/f'hdr-{first:03}').as_posix()}", "DumpGPU"]
                events.append((96+i, u.macro(macros, f"f{i}", commands)))
            extra = ["r.DumpGPU.Upload 0", "r.DumpGPU.Explore 0", "r.DumpGPU.Stream 0", "r.DumpGPU.Texture 2",
                     "r.DumpGPU.Buffer 2", "r.DumpGPU.PassParameters 1", "r.DumpGPU.Root Tonemap*",
                     f"set DirectionalLightComponent LightSourceAngle {math.degrees(2*scene['sun']['angular_radius']):.9f}",
                     "r.Shadow.CacheWPOPrimitives 1", "r.Shadow.CacheWPOPrimitives 0", *u.READBACK]
            with u.lease("motion-"+fixture+"-"+config, args.minutes):
                u.launch_ue(out, config, "static", extra=extra, events=events, frames=260, level=f"/Game/M7R0a/{fixture}/Bench")
                if not u.readback(out, config, u.scalability_expectations()):
                    raise ValueError("motion settings/tracing-mode mismatch")
            log = (out/"ue.log").read_text(encoding="utf-8-sig", errors="replace")
            save(out/"scene-readback.json", check_scene_light(log, scene))
            camera = b.check_camera(log, expected)
            save(out/"camera-readback.json", dict(all_ok=True, frames=camera))
            transforms = check_motion_transforms(log, expected, actor_names)
            save(out/"motion-readback.json", dict(all_ok=True, frames=transforms))
            save(out/"recipe.json", dict(scene_sha256=sha(source), frames=expected, groups=groups,
                object_actor_names=actor_names, shutter="zero", fixed_delta_seconds=1/60, synchronization="r.OneFrameThreadLag 0",
                motion_readback_status="camera, object transforms and light direction qualified per frame"))
            cpu_note("decode selected native motion groups "+fixture+" "+config)
            score = load("m7-u0b-score")
            for first, count in groups:
                score.decode(argparse.Namespace(dump=out/f"hdr-{first:03}", out=out/f"native-{first:03}.npz", stops=scene["exposure_stops"]))
            save(out/"complete.json", dict(camera_ok=True, settings_ok=True, motion_settings_qualified=True,
                timing_eligible_for_claim=False))


def check_scene_light(log, scene):
    values = [float(v) for v in re.findall(r"\.LightSourceAngle = ([0-9.eE+-]+)", log)]
    expected = math.degrees(2*scene["sun"]["angular_radius"])
    if not values or abs(values[-1]-expected)>2e-5:
        raise ValueError("native sun angle readback does not match fixture")
    intensities = []
    for line in log.splitlines():
        if "SkyLightComponent" in line and (m := re.search(r"\.Intensity = ([0-9.eE+-]+)", line)):
            intensities.append(float(m[1]))
    sky = max(scene["sky"]["constant"])
    if not intensities or abs(intensities[-1]-sky)>1e-4:
        raise ValueError("native sky intensity readback does not match fixture")
    return dict(expected_sun_angle_degrees=expected, observed_sun_angles_degrees=values,
        expected_sky_intensity=sky, observed_sky_intensities=intensities, all_ok=True,
        source="native component getall queries; runtime angle followed by U0b's proxy/cache invalidation")


def check_motion_transforms(log, expected, actor_names):
    """Fail closed on missing/shifted object or light readbacks, just as for cameras."""
    sections = re.split(r"[^\r\n]*CameraCachePrivate = [^\r\n]*", log)[1:]
    if len(sections)!=len(expected):
        raise ValueError("motion transform readback frame count")
    number = r"([-0-9.eE+]+)"
    reports = []
    for i, (section, state) in enumerate(zip(sections, expected)):
        objects = {}
        for name, position in state["movers"].items():
            actor = re.escape(actor_names[name])
            matches = re.findall(actor+r"\.[^\r\n]*?\.RelativeLocation = \(X="+number+r",Y="+number+r",Z="+number+r"\)", section)
            if len(matches)!=1:
                raise ValueError(f"frame {i}: missing/ambiguous mover {name}: {matches}")
            actual = list(map(float, matches[0])); x, y, z = position
            error = max(abs(a-b) for a, b in zip(actual, [-100*z, 100*x, 100*y]))
            if error>.001:
                raise ValueError(f"frame {i}: moved-object mismatch {name}: {error} cm")
            objects[name] = dict(actual_ue_cm=actual, maximum_error_cm=error)
        rotation = re.findall(r"DirectionalLight[^\r\n]*?\.RelativeRotation = \(Pitch="+number+r",Yaw="+number+r",Roll="+number+r"\)", section)
        if len(rotation)!=1:
            raise ValueError(f"frame {i}: missing/ambiguous sun rotation")
        pitch, yaw, roll = map(math.radians, map(float, rotation[0]))
        direction = [-math.cos(pitch)*math.sin(yaw), -math.sin(pitch), math.cos(pitch)*math.cos(yaw)]
        error = max(abs(a-b) for a, b in zip(direction, state["to_light"]))
        if error>2e-5 or abs(roll)>1e-5:
            raise ValueError(f"frame {i}: sun mismatch {error}")
        reports.append(dict(frame=i, objects=objects, sun_direction=direction, sun_maximum_error=error))
    return reports


def motion_score(args):
    cpu_note("score registered native motion centres and age witnesses"); no_quiet()
    rows = []; codes = load("m7-u0b-score").codes
    for fixture in ("RF3", "RF5"):
        scene = json.loads((OUT/"scenes"/fixture/"scene.json").read_text())
        for config in [*args.configs, "renderlab-nonrt", "renderlab-rt"]:
            current = config.startswith("renderlab-")
            root = OUT/"current"/((args.current_label or args.label)+"-motion")/config.removeprefix("renderlab-") if current else OUT/"motion/capture"/args.label/fixture/config
            if not (root/"complete.json").exists():
                continue
            if not current and not json.loads((root/"complete.json").read_text())["motion_settings_qualified"]:
                raise ValueError("unqualified moving native capture")
            native_frames = {}
            for first, count in (() if current else ((31, 9), (48, 1), (64, 1), (79, 9), (95, 1))):
                decoder = json.loads((root/f"native-{first:03}.json").read_text())["frames"]
                if len(decoder)!=count:
                    raise ValueError("native motion group coverage mismatch")
                for offset, record in enumerate(decoder):
                    native_frames[first+offset] = Path(record["archive"])
            cache = OUT/"scores/motion-cache"/(fixture+"-"+config+".json")
            cached = json.loads(cache.read_text()) if cache.exists() else {}
            witnesses = scene["registration"]["witness_frames"]
            valid = (cached.get("scoring_version")==MOTION_SCORING_VERSION and
                cached.get("fixture")==fixture and cached.get("config")==config and
                [f["frame"] for f in cached.get("frames",[])]==witnesses)
            if valid:
                for value in cached["frames"]:
                    frame = value["frame"]; name = f"{fixture}-f{frame:03d}-zero"
                    candidate_path = root/(name+"-centre.npz") if current else native_frames[frame]
                    reference_path = OUT/"reference"/args.reference_label/name/str(args.spp*2)/(name+"-centre.npz")
                    if not candidate_path.exists() or not reference_path.exists() or sha(candidate_path)!=value["native_sha256"] or sha(reference_path)!=value["reference_sha256"]:
                        valid = False; break
            if valid:
                rows.append(cached); save(root/"scores.json",cached)
                continue
            previous = None; frame_scores = []; residuals = []
            for frame in scene["registration"]["witness_frames"]:
                name = f"{fixture}-f{frame:03d}-zero"
                if current:
                    native_frames[frame] = root/(name+"-centre.npz")
                reference = OUT/"reference"/args.reference_label/name/str(args.spp*2)/(name+"-centre.npz")
                if not reference.exists():
                    raise ValueError("missing motion reference "+str(reference))
                with np.load(reference) as r:
                    half0, half1 = r["L0"], r["L1"]
                    populations = {k[4:]: r[k] for k in r.files if k.startswith("pop_")}
                with np.load(native_frames[frame]) as n:
                    candidate = n["L"] if current else n["hdr"][..., :3]/n["pre_exposure"]
                truth = (half0+half1)/2
                scores = {name: measures(candidate, half0, half1, mask) for name, mask in populations.items()}
                features = feature_measures(candidate, truth, populations)
                frame_scores.append(dict(frame=frame, measures=scores, features=features,
                    native_sha256=sha(native_frames[frame]), reference_sha256=sha(reference)))
                if previous is not None and frame==previous["frame"]+1:
                    mask = populations.get("reflector", populations["image"])|previous["mask"]
                    values = {}; trails = {}
                    for stops in (-14, -10):
                        candidate_codes = codes(candidate, stops); reference_codes = codes(truth, stops)
                        error = candidate_codes-reference_codes
                        change = error-previous["errors"][stops]
                        values[str(stops)] = dict(rgb_rms=float(np.sqrt(np.mean(change[mask]**2))),
                            p99=float(np.quantile(np.max(np.abs(change[mask]), axis=-1), .99)))
                        for feature, channel in (("reflected-red", 0), ("reflected-blue", 2)):
                            if feature not in populations or feature not in previous["features"]:
                                continue
                            old_only = previous["features"][feature]&~populations[feature]&mask
                            other = [c for c in (0, 1, 2) if c!=channel]
                            signal = lambda v: np.maximum(v[..., channel]-.5*v[..., other].sum(-1), 0)
                            visible = old_only&(signal(candidate_codes)-signal(reference_codes)>2.)
                            trails[f"{feature}@{stops}"] = trail_extent(visible, populations[feature])
                    residuals.append(dict(pair=[previous["frame"], frame], display_residual=values, visible_trails=trails,
                        trail_definition="excess target-colour contrast >2 display codes in old-only centre-reflection footprint; Chebyshev distance from current footprint, capped lower bound at 33 px"))
                previous = dict(frame=frame, mask=populations.get("reflector", populations["image"]),
                    features={k:v for k,v in populations.items() if k.startswith("reflected-")},
                    errors={stops:codes(candidate, stops)-codes(truth, stops) for stops in (-14, -10)})
            record = dict(fixture=fixture, config=config, frames=frame_scores, adjacent_motion_residuals=residuals,
                scoring_version=MOTION_SCORING_VERSION,
                candidate_scope="independent state snapshots, eight warm frames each; no continuous history claim" if current else "continuous 96-frame motion, verified transforms and sun",
                ages={"motion-start":{str(age):32+age-1 for age in (1, 2, 4, 8)},
                      "motion-stop":{str(age):80+age-1 for age in (1, 2, 4, 8)}},
                settled_age8_frame=87, missing_measures=[], timing_eligible_for_claim=False)
            save(root/"scores.json", record); save(cache,record); rows.append(record)
    save(OUT/"scores"/(args.label+"-motion.json"), dict(rows=rows, complete=len(rows)==2*(len(args.configs)+2),
        scope="zero-shutter centres; finite-shutter references reported independently"))


def measures(candidate, half0, half1, mask):
    """Unfitted full-population errors. Noise is reported, never hidden by pixel rejection."""
    reference = (half0+half1)/2
    error = candidate-reference
    lum = lambda x: x@np.array([.2126, .7152, .0722])
    c, r = candidate[mask], reference[mask]
    if not len(r):
        return dict(pixels=0, status="no visible primary pixels in registered population")
    scale = max(1e-20, float(np.mean(r.astype("f8")**2)))
    signed = float(np.sum(c-r, dtype="f8")/max(1e-20, float(np.sum(r, dtype="f8"))))
    result = dict(pixels=int(mask.sum()), rgb_relative_rms=float(np.sqrt(np.mean(error[mask].astype("f8")**2)/scale)),
        luminance_relative_l1=float(np.abs(lum(c)-lum(r)).sum()/max(1e-20, np.abs(lum(r)).sum())),
        signed_energy_bias=signed,
        absolute_energy_bias=abs(signed),
        split_noise_relative_rms=float(np.sqrt(np.mean(((half0[mask]-half1[mask])/2).astype("f8")**2)/scale)))
    codes = load("m7-u0b-score").codes
    for stops in (-14, -10):
        e = codes(c, stops)-codes(r, stops)
        result[f"display_rms_{stops}"] = float(np.sqrt(np.mean(e**2)))
        noise = (codes(half0[mask], stops)-codes(half1[mask], stops))/2
        result[f"reference_split_display_noise_rms_{stops}"] = float(np.sqrt(np.mean(noise**2)))
        result[f"p99_absolute_display_error_{stops}"] = float(np.quantile(np.max(np.abs(e), axis=-1), .99))
    return result


def feature_measures(candidate, reference, populations):
    """Centre-ray feature guides are geometry, not a fitted candidate segmentation.

    Energy is radiance in the registered footprint; it includes the complete
    image there. Colour centroids use the whole reflector, retaining spill and
    trails outside that footprint. Edge widths apply to coloured B0/B1 steps.
    """
    result = {}
    reflector = populations.get("reflector")
    if reflector is None:
        return result
    for feature, channel in (("reflected-red", 0), ("reflected-blue", 2),
                             ("reflected-shell", 0), ("reflected-toon", 1), ("reflected-thin", None)):
        mask = populations.get(feature)
        if mask is None or not mask.any():
            continue
        value = dict(guide_pixels=int(mask.sum()), footprint_energy_ratio=float(candidate[mask].sum(dtype="f8")/
            max(1e-20, reference[mask].sum(dtype="f8"))))
        if channel is not None:
            def contrast(image):
                other = [k for k in (0, 1, 2) if k!=channel]
                return np.maximum(image[..., channel]-.5*image[..., other].sum(-1), 0)
            c, r = contrast(candidate), contrast(reference)
            ys, xs = np.nonzero(reflector)
            def centre(signal):
                w = signal[reflector].astype("f8"); total = float(w.sum())
                return None if total<=1e-20 else [float(w@xs/total), float(w@ys/total)]
            cc, rc = centre(c), centre(r)
            value.update(candidate_centroid=cc, reference_centroid=rc,
                centroid_error_pixels=float(np.linalg.norm(np.array(cc)-rc)) if cc is not None and rc is not None else None,
                chromatic_energy_ratio=float(c[reflector].sum(dtype="f8")/max(1e-20, r[reflector].sum(dtype="f8"))))
            widths = []
            minimum_step = max(1., .1*float(r[reflector].max()))
            # Horizontal transition witnesses, one every eight rows. Use the
            # reference's levels for both images; a missing edge stays a miss.
            for y in range(8, mask.shape[0]-8, 8):
                edges = np.flatnonzero(mask[y, 1:]!=mask[y, :-1])+1
                for x in edges:
                    if x<16 or x+16>=mask.shape[1] or not reflector[y, x-16:x+17].all():
                        continue
                    rr = r[y, x-16:x+17]; cr = c[y, x-16:x+17]
                    low, high = float(rr[:3].mean()), float(rr[-3:].mean())
                    if abs(high-low)<minimum_step:
                        continue
                    def width(line):
                        line = (line-low)/(high-low)
                        crossing = []
                        for threshold in (.1, .9):
                            indices = np.flatnonzero((line[:-1]<threshold)&(line[1:]>=threshold))
                            if not len(indices):
                                return None
                            at = int(indices[np.argmin(np.abs(indices-16))])
                            crossing.append(at+float((threshold-line[at])/(line[at+1]-line[at])))
                        return max(0., crossing[1]-crossing[0])
                    rw, cw = width(rr), width(cr)
                    if rw is not None:
                        widths.append((rw, cw))
            valid = [cw-rw for rw, cw in widths if cw is not None]
            value["edge_spread"] = dict(eligible_reference_edges=len(widths), candidate_missing_edges=sum(cw is None for _, cw in widths),
                median_extra_pixels=float(np.median(valid)) if valid else None,
                p95_extra_pixels=float(np.quantile(valid, .95)) if valid else None,
                definition="reference-level 10–90% chromatic edge width, 33-pixel row, complete-image radiance")
        result[feature] = value
    return result


def trail_extent(visible_error, current_feature, cap=32):
    """Chebyshev extent from current geometry; values beyond cap remain a lower bound."""
    if not visible_error.any():
        return dict(pixels=0, extent_pixels=0, lower_bound=False)
    covered = current_feature.copy()
    h, w = covered.shape
    for distance in range(cap+1):
        if not (visible_error&~covered).any():
            return dict(pixels=int(visible_error.sum()), extent_pixels=distance, lower_bound=False)
        padded = np.pad(covered, 1)
        covered = np.logical_or.reduce([padded[y:y+h, x:x+w] for y in range(3) for x in range(3)])
    return dict(pixels=int(visible_error.sum()), extent_pixels=cap+1, lower_bound=True)


def score_static(args):
    cpu_note("score static reflection floors against actual split-half references"); no_quiet()
    rows = []
    for fixture in args.fixtures:
        scene = json.loads((OUT/"scenes"/fixture/"scene.json").read_text())
        for pose in scene["poses"]:
            case = fixture+"-"+pose["name"]
            reference_path = static_reference_path(args, fixture, case)
            if not reference_path.exists():
                continue
            with np.load(reference_path) as ref:
                h0, h1 = ref["L0"], ref["L1"]
                populations = {k[4:]: ref[k] for k in ref.files if k.startswith("pop_")}
            candidates = []
            for config in args.configs:
                root = OUT/"capture"/args.label/fixture/pose["name"]/config
                if not (root/"complete.json").exists():
                    continue
                provenance = json.loads((root/"complete.json").read_text())
                if provenance["scene_sha256"]!=sha(OUT/"scenes"/fixture/"scene.json"):
                    equivalence = OUT/"motion"/(fixture+"-static-equivalence.json")
                    proof = json.loads(equivalence.read_text()) if equivalence.exists() else {}
                    if not proof.get("static_inputs_equal") or proof.get("imported_source_sha256")!=provenance["scene_sha256"] or proof.get("corrected_source_sha256")!=sha(OUT/"scenes"/fixture/"scene.json"):
                        raise ValueError("static capture input hash mismatch")
                with np.load(root/"native.npz") as native:
                    image = native["hdr"][..., :3]/native["pre_exposure"]
                candidates.append((config, root, image))
            for backend in ("nonrt", "rt"):
                path = OUT/"current"/(args.current_label or args.label)/backend/(case+".npz")
                unsupported = path.parent/(case+".unsupported.json")
                if unsupported.exists():
                    failure = json.loads(unsupported.read_text())
                    record = dict(fixture=fixture, pose=pose["name"], config="renderlab-"+backend,
                        status="unsupported", reason=failure["reason"], measures={}, features={},
                        scene_sha256=failure["scene_sha256"], unsupported_metadata_sha256=sha(unsupported),
                        scope="explicit current-renderer image-production miss; no substituted material or numeric pass")
                    save(path.parent/(case+"-scores.json"), record); rows.append(record)
                    continue
                if path.exists():
                    with np.load(path) as native:
                        candidates.append(("renderlab-"+backend, path.parent, native["L"]))
            for config, root, image in candidates:
                if not np.isfinite(image).all() or image.shape!=h0.shape:
                    raise ValueError("candidate dimensions/finite check failed")
                scores = {name: measures(image, h0, h1, mask) for name, mask in populations.items()}
                record = dict(fixture=fixture, pose=pose["name"], config=config, measures=scores,
                    features=feature_measures(image, (h0+h1)/2, populations),
                    scene_sha256=sha(OUT/"scenes"/fixture/"scene.json"), reference_archive_sha256=sha(reference_path),
                    candidate_archive_sha256=sha(root/(case+".npz") if config.startswith("renderlab-") else root/"native.npz"),
                    scope="static primary-surface populations; no fitted gain, denoising or noisy-pixel removal",
                    missing_measures=["trail width", "motion and settle"])
                save(root/(case+"-scores.json"), record)
                from PIL import Image
                codes = load("m7-u0b-score").codes
                r = (h0+h1)/2
                sheet = np.concatenate([codes(r, -14), codes(image, -14), np.minimum(255, 4*np.abs(codes(image, -14)-codes(r, -14)))], axis=1)
                Image.fromarray(np.clip(sheet, 0, 255).astype("u1")).save(root/(case+"-comparison.png"))
                rows.append(record)
    expected = {(fixture, pose["name"], config) for fixture in args.fixtures
                for pose in json.loads((OUT/"scenes"/fixture/"scene.json").read_text())["poses"]
                for config in [*args.configs, "renderlab-nonrt", "renderlab-rt"]}
    missing = sorted(expected-{(r["fixture"], r["pose"], r["config"]) for r in rows})
    save(OUT/"scores"/(args.label+"-static.json"), dict(rows=rows, complete=not missing,
        fixtures=args.fixtures, configs=args.configs, expected_cases=len(expected),
        missing_cases=missing, reason="Static coverage only; unit acceptance separately requires independent controls, motion and full-image inspection"))


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("command", choices=("generate", "reference", "prepare", "cook", "capture", "checks", "build", "baseline", "score", "crosschecks", "adopt-oracle", "assemble-references", "cleanup-stage", "prune-pilots", "prune-superseded-arrays", "register-motion", "probe-as", "compatibility",
        "motion-scenes", "motion-reference", "motion-capture", "motion-score", "motion-baseline"))
    p.add_argument("--fixtures", nargs="+", default=[f"RF{i}" for i in range(1, 11)])
    p.add_argument("--label", default="a")
    p.add_argument("--spp", type=int, default=2048)
    p.add_argument("--depth", type=int, default=1024)
    p.add_argument("--minutes", type=int, default=12)
    p.add_argument("--configs", nargs="+", default=["sw-mesh-epic", "hw-sc-epic", "hw-hit-epic"])
    p.add_argument("--cook-label", default="a")
    p.add_argument("--reference-label", default="b")
    p.add_argument("--current-label")
    p.add_argument("--as-build", choices=("trace", "build"), default="trace")
    p.add_argument("--control-sun", type=float)
    p.add_argument("--ordered-hits", action="store_true")
    p.add_argument("--mirror-control", action="store_true")
    p.add_argument("--mitsuba-spp", type=int, default=8192)
    p.add_argument("--planar-sun-guide", action="store_true")
    p.add_argument("--package", default="ue-package")
    p.add_argument("--repeats", type=int, default=8)
    args = p.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    if args.command == "generate":
        cpu_note("generate reflection fixtures"); no_quiet()
        print(json.dumps(load("m7-r0a-scenes").generate(OUT/"scenes", args.fixtures), indent=2))
    else:
        {"reference": references, "prepare": prepare, "cook": cook, "capture": capture, "checks": checks, "build": checks, "cleanup-stage": cleanup_stage, "prune-pilots":prune_pilots, "prune-superseded-arrays":prune_superseded_arrays,
         "probe-as": probe_as, "compatibility": compatibility,
         "register-motion":synchronize_motion_tracks,
         "adopt-oracle":adopt_oracle,
         "assemble-references":assemble_references,
         "baseline": baseline, "score": score_static, "crosschecks": crosschecks, "motion-scenes": motion_scenes,
         "motion-reference": motion_references, "motion-capture": motion_capture, "motion-score": motion_score,
         "motion-baseline": motion_baseline}[args.command](args)


if __name__ == "__main__":
    main()
