"""Optional D0 missing-data supplement using an EXISTING reference executable.

Generates a measurement-only shader override. No renderer/source/build changes.
The atlas stores independent per-pixel sub-cone rays, their actual directions,
throughputs and hit distances. Original RF content and production user programs
are reused. A raw texture carries the unchanged production DFG bytes.
"""
import argparse
import copy
import importlib.util
import json
import math
import os
from pathlib import Path
import time
import ctypes
from ctypes import wintypes

REPO = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("d0", REPO / "scripts/m7-d0.py")
d0 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(d0)
OUT = d0.OUT
W, H = 128, 96
# K4, K8, K16, K32 strata; a suppressed-specular primary; high-K control.
LAYERS = 62


class BasicLimits(ctypes.Structure):
    _fields_ = [("process_time",ctypes.c_longlong),("job_time",ctypes.c_longlong),("flags",wintypes.DWORD),
                ("min_working_set",ctypes.c_size_t),("max_working_set",ctypes.c_size_t),("active_processes",wintypes.DWORD),
                ("affinity",ctypes.c_size_t),("priority",wintypes.DWORD),("scheduling",wintypes.DWORD)]


class ExtendedLimits(ctypes.Structure):
    _fields_ = [("basic",BasicLimits),("io",ctypes.c_ulonglong*6),("process_memory",ctypes.c_size_t),
                ("job_memory",ctypes.c_size_t),("peak_process_memory",ctypes.c_size_t),("peak_job_memory",ctypes.c_size_t)]


def bounded_child_factory(binary, original):
    """Hard-cap the capture executable's commit to 1.25 GiB; parent stays small."""
    kernel = ctypes.windll.kernel32
    kernel.CreateJobObjectW.argtypes = [ctypes.c_void_p,wintypes.LPCWSTR]
    kernel.CreateJobObjectW.restype = wintypes.HANDLE
    kernel.SetInformationJobObject.argtypes = [wintypes.HANDLE,ctypes.c_int,ctypes.c_void_p,wintypes.DWORD]
    kernel.AssignProcessToJobObject.argtypes = [wintypes.HANDLE,wintypes.HANDLE]
    kernel.QueryInformationJobObject.argtypes = [wintypes.HANDLE,ctypes.c_int,ctypes.c_void_p,wintypes.DWORD,ctypes.c_void_p]
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    job = kernel.CreateJobObjectW(None,None)
    if not job:
        raise OSError("CreateJobObjectW failed")
    limit = ExtendedLimits()
    limit.basic.flags = 0x300  # PROCESS_MEMORY | JOB_MEMORY, including descendants.
    limit.process_memory = limit.job_memory = 5*2**28
    if not kernel.SetInformationJobObject(job,9,ctypes.byref(limit),ctypes.sizeof(limit)):
        kernel.CloseHandle(job)
        raise OSError("SetInformationJobObject failed")
    def popen(command,*args,**kwargs):
        process = original(command,*args,**kwargs)
        if isinstance(command,(list,tuple)) and str(command[0])==str(binary):
            if not kernel.AssignProcessToJobObject(job,wintypes.HANDLE(int(process._handle))):
                process.terminate()
                process.wait()
                raise OSError("AssignProcessToJobObject failed; capture stopped")
        return process
    def finish():
        info = ExtendedLimits()
        ok = kernel.QueryInformationJobObject(job,9,ctypes.byref(info),ctypes.sizeof(info),None)
        kernel.CloseHandle(job)
        if not ok:
            raise OSError("QueryInformationJobObject failed")
        return dict(commit_limit_mib=1280,peak_capture_commit_mib=info.peak_job_memory/2**20,parent=d0.memory())
    return popen,finish


def function(source, name):
    start = source.index("fn " + name + "(")
    end, depth = source.index("{", start)+1, 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end]


def shader(rect, oracle_k):
    source = (REPO / "shaders/reference_path_tracer.wgsl").read_text()
    source = source[:source.index("var<private> reference_centre")]
    source = source.replace("struct Material {", "struct RefMaterial {").replace("array<Material>", "array<RefMaterial>").replace("m:Material", "m:RefMaterial")
    source = source.replace("environment_direction(", "reference_environment_direction(")
    source = source.replace("identity:vec2<u32>};", "identity:vec2<u32>,source_front:bool};")
    source = source.replace("vec2<u32>(0u));", "vec2<u32>(0u),true);")
    source = source.replace("if dot(ng,direction)>0.0 {ng=-ng;n=-n;}", "let source_front=dot(ng,direction)<=0.0; if !source_front {ng=-ng;n=-n;}")
    source = source.replace("h.primitive_index+1u));", "h.primitive_index+1u),source_front);")
    source = source.replace("let size=vec2<f32>(frame.size.xy);", "let size=vec2<f32>(1920.0,1080.0);")
    api = (REPO / "shaders/program_api.wgsl").read_text()
    source += api[api.index("struct SurfaceInput {"):api.index("// One light's")]
    helper = (REPO / "shaders/reflection_probe.wgsl").read_text()
    helper = helper[:helper.index("fn primary_ray_probe")]
    helper = helper.replace("@group(0) @binding(11) var<uniform> probe:ProbeSettings;", "var<private> probe:ProbeSettings;")
    for binding, name in ((12,"primary"), (13,"secondary"), (14,"incident")):
        start = helper.index(f"@group(0) @binding({binding})")
        end = helper.index(";", start)+1
        helper = helper[:start] + helper[end:]
    helper = helper.replace("@group(0) @binding(10) var<storage,read> dfg_values:array<vec2<f32>>;", "fn dfg_value(i:u32)->vec2<f32> {return unpack2x16float(texture_texels[i]);}")
    import re
    helper = re.sub(r"dfg_values\[([^]]+)\]", r"dfg_value(\1)", helper)
    source += helper
    scene = (REPO / "shaders/scene.wgsl").read_text()
    for name in ("exposed_product", "brdf"):
        source += function(scene, name) + "\n"
    source += "struct LightSample {index:u32,direction:vec3<f32>,attenuation:f32,intensity:vec3<f32>};\n"
    for name in ("light_count", "light_sample", "light_visibility", "expose", "rl_finite", "material_normal"):
        source += function(api, name) + "\n"
    standard = (REPO / "shaders/programs/standard.wgsl").read_text()
    standard = standard.replace("Surface", "StdSurface").replace("StdSurfaceInput", "SurfaceInput").replace("fn surface(", "fn std_surface(").replace("fn shade(", "fn std_shade(")
    toon = (REPO / "shaders/programs/toon.wgsl").read_text()
    toon = toon.replace("Surface", "ToonSurface").replace("ToonSurfaceInput", "SurfaceInput").replace("fn surface(", "fn toon_surface(").replace("fn shade(", "fn toon_shade(")
    toon = toon.replace("Parameters", "ToonParameters").replace("DEFAULT_PARAMETERS", "TOON_DEFAULT_PARAMETERS").replace("params.", "toon_params.")
    source += standard + toon + "\nvar<private> toon_params:ToonParameters=TOON_DEFAULT_PARAMETERS;\n"
    source += f"""
var<private> d0_aux:Geometry;
fn d0_ray(h:Hit,v:vec3<f32>,k:u32,count:u32,pixel:u32)->vec3<f32> {{
    let m=material_at_hit(h); var l=lobe(m,dot(h.n,v));l.p_spec=1.0;
    var direction=reflect(-v,h.n);var weight=vec3<f32>(0.0);
    if m.params.x<0.001 {{
        if k!=0u {{return vec3<f32>(0.0);}}
        weight=l.f0+(1.0-l.f0)*pow(1.0-clamp(dot(h.n,v),0.0,1.0),5.0);
    }} else {{
        let rotation=f32(pcg(pixel)>>8u)/16777216.0;
        let u=vec2<f32>((f32(k)+0.5)/f32(count),fract(rotation+f32(k)*0.618033989));
        let b=basis(h.n);let micro=b*sample_vndf(l,transpose(b)*v,u);direction=reflect(-v,micro);
        let pdf=bsdf_pdf(l,h.n,v,direction);
        if pdf>0.0 {{weight=max(bsdf(m,l,h.n,v,direction)-l.diffuse,vec3<f32>(0.0))*max(dot(h.n,direction),0.0)/(pdf*f32(count));}}
    }}
    d0_aux.direct_even=vec4<f32>(direction,m.params.x);
    d0_aux.direct_odd=vec4<f32>(weight,1.0);
    if dot(h.ng,direction)<=0.0 {{return vec3<f32>(0.0);}}
    let secondary_hit=trace(offset(h.p,h.ng),direction);
    d0_aux.normal.w=select(0.0,length(secondary_hit.p-h.p),secondary_hit.hit);
    d0_aux.ids.z=secondary_hit.identity.x;
    d0_aux.ids.w=1u;
    if !secondary_hit.hit {{return sky_radiance(direction);}}
    suppress_specular=false;
    return own_program(pack(secondary_hit,direction),vec2<u32>(pixel%1920u,pixel/1920u));
}}
@compute @workgroup_size(8,8)
fn trace_samples(@builtin(global_invocation_id) id:vec3<u32>) {{
    if any(id.xy>=frame.size.xy) {{return;}}
    let layer=id.y/{H}u;
    let at=vec2<u32>(id.x+{rect[0]}u,id.y%{H}u+{rect[1]}u);
    let pixel=at.y*1920u+at.x;
    let ray=camera_ray_time(vec2<f32>(at)+0.5,0.0);
    let hit=trace(ray[0],ray[1]);
    probe=ProbeSettings(1u,1u,0u,1.0);setup();
    d0_aux.world=vec4<f32>(hit.p,select(0.0,1.0,hit.hit));
    d0_aux.normal=vec4<f32>(hit.ng,0.0);
    d0_aux.ids=vec4<u32>(hit.identity.x,hit.material+1u,0u,0u);
    var value=frame.background.rgb;
    if hit.hit {{
        if layer==60u {{
            suppress_specular=true;
            value=own_program(pack(hit,ray[1]),at);
        }} else if layer==61u {{
            value=vec3<f32>(0.0);
            for(var k=0u;k<{oracle_k}u;k+=1u) {{
                let incident_value=d0_ray(hit,-ray[1],k,{oracle_k}u,pixel);
                value+=incident_value*d0_aux.direct_odd.rgb;
            }}
        }} else {{
            var k=layer; var count=4u;
            if layer>=28u {{k=layer-28u;count=32u;}}
            else if layer>=12u {{k=layer-12u;count=16u;}}
            else if layer>=4u {{k=layer-4u;count=8u;}}
            value=d0_ray(hit,-ray[1],k,count,pixel);
        }}
    }}
    let index=id.y*frame.size.x+id.x;
    sums[2u*index]+=vec4<f32>(value,0.0);
    sums[2u*index+1u]+=vec4<f32>(value,0.0);
    geometry[index]=d0_aux;
}}
"""
    return source


def prepare(args):
    root = OUT / "supplement" / args.label
    root.mkdir(parents=True, exist_ok=True)
    original = OUT.parent / f"m7-r0a/scenes/{args.fixture}/scene.json"
    scene = json.loads(original.read_text())
    source = copy.deepcopy(scene)
    pose = next(p for p in scene["poses"] if p["name"] == args.pose)
    eye, target = pose["eye"], pose["target"]
    dx, dz = target[0]-eye[0], target[2]-eye[2]
    length = math.hypot(dx,dz)
    right = [-dz/length, 0, dx/length]
    pixel_world = 2*math.dist(eye,target)*math.tan(math.radians(scene["fov_y_degrees"])/2)/1080
    source["poses"] = []
    for frame in range(args.frames):
        shift = frame*.25*pixel_world
        source["poses"].append(dict(name=f"f{frame:02}", eye=[v+shift*r for v,r in zip(eye,right)], target=[v+shift*r for v,r in zip(target,right)]))
    source["size"] = [W,H*LAYERS]
    assert not source.get("textures"), "DFG data must be the first raw texture"
    source["textures"] = [dict(mips=[dict(width=128,height=128,file="dfg.raw")])]
    (root / "dfg.raw").write_bytes((REPO / "src/dfg.rg16f").read_bytes())
    d0.save(root / "scene.json", source)
    (root / "capture.wgsl").write_text(shader(args.rect, args.oracle), encoding="utf-8")
    d0.save(root / "recipe.json", dict(fixture=args.fixture, pose=args.pose, rect=args.rect,
            original_scene_sha256=d0.sha(original), scene_sha256=d0.sha(root/"scene.json"),
            shader_sha256=d0.sha(root/"capture.wgsl"), oracle_k=args.oracle, frames=args.frames,
            layout="60 individual K4/8/16/32 incident layers; primary base; high-K weighted specular", native_size=[1920,1080],
            shader_provenance="own measurement override, repository R0b helpers and unchanged user sources; raw production DFG"))
    print(root)


def prepare_film(args):
    """Native pixel and quarter-pixel shutter integration with the existing tracer."""
    root=OUT/"supplement"/args.label
    root.mkdir(parents=True,exist_ok=True)
    parent=OUT/"supplement"/args.source_label
    scene=json.loads((parent/"scene.json").read_text())
    recipe=json.loads((parent/"recipe.json").read_text())
    if args.still:
        scene["poses"]=[scene["poses"][-1]]
    scene["size"]=[W,H]
    (root/"dfg.raw").write_bytes((parent/"dfg.raw").read_bytes())
    d0.save(root/"scene.json",scene)
    r0a=d0.load("m7-r0a")
    r0a.measurement_shader(scene,root/"capture.wgsl",ordered_hits=True)
    source=(root/"capture.wgsl").read_text()
    original=function(source,"camera_ray_time")
    cropped=original.replace("let size=vec2<f32>(frame.size.xy);",
            f"let size=vec2<f32>(1920.0,1080.0); let native_pixel=pixel+vec2<f32>({float(recipe['rect'][0])},{float(recipe['rect'][1])});")
    cropped=cropped.replace("pixel.x/size.x","native_pixel.x/size.x").replace("pixel.y/size.y","native_pixel.y/size.y")
    first=scene["poses"][0]["eye"]
    step=[b-a for a,b in zip(first,scene["poses"][1]["eye"])] if len(scene["poses"])>1 else [0.,0.,0.]
    cropped=cropped.replace("let eye=mix(frame.eye0.xyz,frame.eye1.xyz,t);",
            f"let first=distance(frame.eye0.xyz,vec3<f32>({','.join(map(str,first))}))<1e-6; let step=select(vec3<f32>({','.join(map(str,step))}),vec3<f32>(0.0),first); let eye=frame.eye0.xyz-(1.0-t)*step;")
    (root/"capture.wgsl").write_text(source.replace(original,cropped),encoding="utf-8")
    d0.save(root/"recipe.json",recipe|dict(kind="film",samples=4096,source_label=args.source_label,frames=len(scene["poses"]),
        source_motion_recipe_sha256=d0.sha(parent/"recipe.json"),scene_sha256=d0.sha(root/"scene.json"),
        shader_sha256=d0.sha(root/"capture.wgsl"),reference="existing R0a ordered-hit path tracer, native box pixel filter, uniform shutter over previous-to-current camera translation; first frame static"))
    print(root)


def run(args):
    root = OUT / "supplement" / args.label
    recipe=json.loads((root/"recipe.json").read_text())
    film=recipe.get("kind")=="film"
    u = d0.load("m7-u0-ue")
    u.OUT, u.OWNER, u.START_GIB = OUT, "m7-d0", 8.0
    deadline = time.monotonic()+args.wait_seconds
    while (d0.memory()["available_gib"] < 8 or d0.CPU_LOCK.exists() or u.LOCK.exists()) and time.monotonic() < deadline:
        time.sleep(1)
    if d0.memory()["available_gib"] < 8 or d0.CPU_LOCK.exists():
        raise SystemExit("DEFERRED: capture requires >=8 GiB and no heavy CPU worker")
    binary = OUT.parent / "m7-r0a/bin/r0a-reference-tests.exe"
    env = dict(os.environ, RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
               RENDERLAB_M7_REFERENCE_OUT=str(root/"data"), RENDERLAB_M7_BENCHES=str(root),
               RENDERLAB_M7_SPP="4096" if film else "2", RENDERLAB_M7_REPLAYS="1", RENDERLAB_M7_SAMPLES_PER_DISPATCH="64" if film else "2",
               RENDERLAB_M7_CAPTURE_GEOMETRY="0" if film else "1", RENDERLAB_M7_SHADER_OVERRIDE=str(root/"capture.wgsl"),
               VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation")
    env.pop("VK_INSTANCE_LAYERS", None)
    (root / "data").mkdir(exist_ok=True)
    u.schedule_line(f"[m7-d0 request] owner=m7-d0 missing per-cone records only; <=3 min headless functional lease {args.label}; no build/timing claim")
    with u.lease(args.label, 3) as owner:
        d0.save(root / "process-recipe.json", dict(binary=str(binary), binary_sha256=d0.sha(binary), lease=owner,
                environment={k:v for k,v in env.items() if k.startswith("RENDERLAB_")}))
        original = u.subprocess.Popen
        bounded,finish = bounded_child_factory(binary,original)
        u.subprocess.Popen = bounded
        try:
            u.guarded([binary,"reference::tests::gpu_reference_benches","--exact","--ignored","--nocapture","--test-threads=1"],
                      root/"process",timeout=165,cwd=REPO,env=env)
        finally:
            u.subprocess.Popen = original
            d0.save(root/"memory-limit.json",finish())


def joint(samples, directions, guides):
    """Fixed 5x5 screen positions; nearest known ray direction at each position.

    Radiance interpolation only, never importance/reservoir resampling. The K
    original receiver throughputs are applied after the incident values filter.
    """
    import numpy as np
    output = np.zeros_like(samples)
    for k in range(len(samples)):
        total = np.zeros_like(samples[k])
        norm = np.zeros((H,W), "f4")
        for oy in range(-2,3):
            for ox in range(-2,3):
                a,b = d0.paired_slices((H,W),oy,ox)
                dots = np.sum(directions[(slice(None),)+b]*directions[(k,)+a],axis=-1)
                selected = np.argmax(dots,axis=0)
                best = np.take_along_axis(dots,selected[None],axis=0)[0]
                source = np.take_along_axis(samples[(slice(None),)+b],selected[None,...,None],axis=0)[0]
                same = (guides["instance"][a]==guides["instance"][b]) & (guides["material"][a]==guides["material"][b])
                normal = np.clip(np.sum(guides["normal"][a]*guides["normal"][b],axis=-1),0,1)**32
                plane = np.abs(np.sum((guides["world"][b]-guides["world"][a])*guides["normal"][a],axis=-1))
                weight = same*normal*np.exp(-plane/(.0005+.002*np.maximum(guides["depth"][a],.1)))
                weight *= np.exp(64*(best-1))*math.exp(-(ox*ox+oy*oy)/2)
                total[a] += source*weight[...,None]
                norm[a] += weight
        output[k] = np.where(guides["valid"][...,None],total/np.maximum(norm[...,None],1e-20),samples[k])
    return output


def temporal(state, current, guides, previous_guides, previous_pose, scene, rect):
    """One history after spatial filtering; all validation/age stays internal.

    Primary identity, normal and plane consistency only. This does not implement
    reflected-hit reverse visibility and is not TR1's acceptance implementation.
    """
    import numpy as np
    if state is None:
        return current.copy(),np.ones((H,W),"f4")
    eye = np.asarray(previous_pose["eye"],"f4")
    forward = np.asarray(previous_pose["target"],"f4")-eye
    forward /= np.linalg.norm(forward)
    right = np.cross(forward,np.array([0,1,0],"f4"));right/=np.linalg.norm(right)
    up = np.cross(right,forward)
    delta = guides["world"]-eye
    depth = np.sum(delta*forward,axis=-1)
    tan = math.tan(math.radians(scene["fov_y_degrees"])/2)
    px = (np.sum(delta*right,axis=-1)/np.maximum(depth,.00001)/(tan*1920/1080)+1)*960-.5-rect[0]
    py = (1-np.sum(delta*up,axis=-1)/np.maximum(depth,.00001)/tan)*540-.5-rect[1]
    ix,iy=np.floor(px).astype(int),np.floor(py).astype(int)
    fx,fy=px-ix,py-iy
    rgb=np.zeros_like(current);weight=np.zeros((H,W),"f4");ages=np.zeros((H,W),"f4")
    for ox,oy in ((0,0),(1,0),(0,1),(1,1)):
        tx,ty=np.clip(ix+ox,0,W-1),np.clip(iy+oy,0,H-1)
        valid=(ix+ox>=0)&(ix+ox<W)&(iy+oy>=0)&(iy+oy<H)&(depth>0)&guides["valid"]
        valid &= guides["instance"]==previous_guides["instance"][ty,tx]
        valid &= guides["material"]==previous_guides["material"][ty,tx]
        valid &= np.sum(guides["normal"]*previous_guides["normal"][ty,tx],axis=-1)>.95
        plane=np.abs(np.sum((guides["world"]-previous_guides["world"][ty,tx])*guides["normal"],axis=-1))
        valid &= plane<.0005+.002*np.maximum(guides["depth"],.1)
        w=valid*(fx if ox else 1-fx)*(fy if oy else 1-fy)
        rgb+=state[0][ty,tx]*w[...,None];ages+=state[1][ty,tx]*w;weight+=w
    age=np.where(weight>0,np.minimum(8,ages/np.maximum(weight,1e-20)+1),1).astype("f4")
    history=rgb/np.maximum(weight[...,None],1e-20)
    return np.where((weight>0)[...,None],history+(current-history)/age[...,None],current).astype("f4"),age


def score(args):
    import numpy as np
    from PIL import Image
    root = OUT/"supplement"/args.label
    recipe = json.loads((root/"recipe.json").read_text())
    scene = json.loads((root/"scene.json").read_text())
    dtype = np.dtype([(name,kind,4) for name,kind in
                     (("world","<f4"),("normal","<f4"),("ids","<u4"),("direction","<f4"),("weight","<f4"))])
    rows, histories, previous_errors, previous_history_errors, last_images = [], {}, {}, {}, {}
    previous_film_errors,previous_film_history_errors={},{}
    previous_guides,previous_pose = None,None
    identity,oracle_frames,reference_control = {},{},{}
    for f,pose in enumerate(scene["poses"]):
        name = root.name+"-"+pose["name"]
        image_path = root/"data"/(name+"-mean.f32")
        geometry_path = root/"data"/(name+"-geometry.bin")
        identity[str(image_path)], identity[str(geometry_path)] = d0.sha(image_path), d0.sha(geometry_path)
        atlas = np.fromfile(image_path,"<f4").reshape(LAYERS,H,W,3)
        records = np.fromfile(geometry_path,dtype).reshape(LAYERS,H,W)
        base, reference = atlas[60], atlas[60]+atlas[61]
        oracle_frames[pose["name"]]=reference.copy()
        primary = records[0]
        eye = np.asarray(pose["eye"],"f4")
        forward = np.asarray(pose["target"],"f4")-eye
        forward /= np.linalg.norm(forward)
        guides = dict(world=primary["world"][...,:3],normal=primary["normal"][...,:3],instance=primary["ids"][...,0],
                      material=primary["ids"][...,1],depth=np.sum((primary["world"][...,:3]-eye)*forward,axis=-1),
                      valid=primary["world"][...,3]>0)
        rough = records[0]["direction"][...,3]
        groups = dict(image=np.ones((H,W),bool),covered=guides["valid"])
        for lo,hi,band in ((0,.05,"B0"),(.05,.4,"B1"),(.4,1.01,"B2")):
            groups[band] = guides["valid"] & (rough>=lo) & (rough<hi)
        if f==0:
            original = OUT.parent/f"m7-r0b/runs/final/{recipe['fixture']}-{recipe['pose']}-k8.npz"
            with np.load(original) as old:
                x,y = recipe["rect"]
                original_crop = old["L"][y:y+H,x:x+W]
            film_path=OUT.parent/f"m7-r0a/reference/final/{recipe['fixture']}/{recipe['fixture']}-{recipe['pose']}.npz"
            identity[str(film_path)]=d0.sha(film_path)
            with np.load(film_path) as film:
                canonical0,canonical1=film["L0"][y:y+H,x:x+W],film["L1"][y:y+H,x:x+W]
                film_crop=(canonical0+canonical1)/2
        film_frame=None
        if args.film_label:
            film_root=OUT/"supplement"/args.film_label
            film_name=film_root.name+"-"+pose["name"]
            halves=[]
            for half in (0,1):
                fp=film_root/"data"/(film_name+f"-half{half}.f32")
                identity[str(fp)]=d0.sha(fp)
                halves.append(np.fromfile(fp,"<f4").reshape(H,W,3))
            film_frame=(halves[0]+halves[1])/2
            oracle_frames["film-"+pose["name"]]=film_frame.copy()
            oracle_frames["film-half0-"+pose["name"]]=halves[0]
            oracle_frames["film-half1-"+pose["name"]]=halves[1]
            if f==0:
                scale=max(1e-20,np.mean(film_crop.astype("f8")**2))
                error=float(np.sqrt(np.mean((film_frame-film_crop).astype("f8")**2)/scale))
                noise=float(np.sqrt((np.mean(((canonical0-canonical1)/2).astype("f8")**2)+np.mean(((halves[0]-halves[1])/2).astype("f8")**2))/scale))
                reference_control=dict(rgb_rms_against_canonical=error,combined_split_noise_rms=noise,
                    signed_energy_bias=float(np.sum(film_frame-film_crop,dtype="f8")/max(1e-20,np.sum(film_crop,dtype="f8"))))
        for k,start in ((4,0),(8,4),(16,12),(32,28)):
            incoming = atlas[start:start+k]
            weights = records[start:start+k]["weight"][...,:3]
            candidates = (*d0.CANDIDATES,"joint") if k in (4,8) else ("none",)
            for kind in candidates:
                if kind=="joint":
                    filtered = joint(incoming,records[start:start+k]["direction"][...,:3],guides)
                else:
                    filtered = np.empty_like(incoming)
                    for j in range(k):
                        sample_guides = guides | dict(hit_t=records[start+j]["normal"][...,3],
                            secondary_valid=records[start+j]["ids"][...,2]>0)
                        filtered[j] = d0.spatial(incoming[j],sample_guides,kind)
                resolved = base+np.sum(filtered*weights,axis=0)
                key = f"k{k}-{kind}"
                histories[key] = temporal(histories.get(key),resolved,guides,previous_guides,previous_pose,scene,recipe["rect"])
                measurements = {}
                for group,mask in groups.items():
                    if not mask.any():
                        continue
                    r,c = reference[mask],resolved[mask]
                    scale = max(1e-20,float(np.mean(r.astype("f8")**2)))
                    error = resolved-reference
                    result = dict(pixels=int(mask.sum()),rgb_relative_rms=float(np.sqrt(np.mean((c-r).astype("f8")**2)/scale)),
                                  signed_energy_bias=float(np.sum(c-r,dtype="f8")/max(1e-20,np.sum(r,dtype="f8"))),
                                  history_rms=float(np.sqrt(np.mean((histories[key][0][mask]-r).astype("f8")**2)/scale)))
                    if key in previous_errors:
                        result["screen_residual_change"] = float(np.sqrt(np.mean((error[mask]-previous_errors[key][mask])**2)/scale))
                        he=histories[key][0]-reference
                        result["history_screen_residual_change"] = float(np.sqrt(np.mean((he[mask]-previous_history_errors[key][mask])**2)/scale))
                    if f==0:
                        film=film_crop[mask]
                        result["film_rgb_relative_rms"] = float(np.sqrt(np.mean((c-film).astype("f8")**2)/max(1e-20,np.mean(film.astype("f8")**2))))
                        result["film_signed_energy_bias"] = float(np.sum(c-film,dtype="f8")/max(1e-20,np.sum(film,dtype="f8")))
                    if film_frame is not None:
                        film=film_frame[mask]
                        film_scale=max(1e-20,np.mean(film.astype("f8")**2))
                        result["motion_film_rms"]=float(np.sqrt(np.mean((c-film).astype("f8")**2)/film_scale))
                        result["motion_film_history_rms"]=float(np.sqrt(np.mean((histories[key][0][mask]-film).astype("f8")**2)/film_scale))
                        result["motion_film_energy_bias"]=float(np.sum(c-film,dtype="f8")/max(1e-20,np.sum(film,dtype="f8")))
                        result["film_split_noise_rms"]=float(np.sqrt(np.mean(((halves[0][mask]-halves[1][mask])/2).astype("f8")**2)/film_scale))
                        if key in previous_film_errors:
                            result["motion_film_residual_change"]=float(np.sqrt(np.mean(((resolved-film_frame)[mask]-previous_film_errors[key][mask]).astype("f8")**2)/film_scale))
                            result["motion_film_history_residual_change"]=float(np.sqrt(np.mean(((histories[key][0]-film_frame)[mask]-previous_film_history_errors[key][mask]).astype("f8")**2)/film_scale))
                    measurements[group] = result
                previous_errors[key] = error.copy()
                previous_history_errors[key] = histories[key][0]-reference
                if film_frame is not None:
                    previous_film_errors[key]=resolved-film_frame
                    previous_film_history_errors[key]=histories[key][0]-film_frame
                last_images[key] = resolved.copy()
                row = dict(frame=f,k=k,candidate=kind,measures=measurements,
                           scope="native crop, current incident filter, high-K pixel-centre same-transport oracle; no TR1; not a film/Unreal gate")
                if f==0 and kind=="none" and k==8:
                    difference = resolved-original_crop
                    row["r0b_control"] = dict(max_absolute=float(np.max(np.abs(difference))),
                         relative_rms=float(np.sqrt(np.mean(difference.astype("f8")**2)/np.mean(original_crop.astype("f8")**2))))
                    assert row["r0b_control"]["relative_rms"]<1e-5,"supplement does not reproduce R0b"
                rows.append(row)
        # Raw buffers are removed only after the complete summary is durable.
        previous_guides={k:v.copy() for k,v in guides.items()}
        previous_pose=pose
        d0.guard()
    settle,film_settle={},{}
    hold_film=None
    if args.hold_label:
        hold_root=OUT/"supplement"/args.hold_label
        hold_halves=[]
        for half in (0,1):
            hp=hold_root/"data"/(hold_root.name+"-"+pose["name"]+f"-half{half}.f32")
            identity[str(hp)]=d0.sha(hp)
            hold_halves.append(np.fromfile(hp,"<f4").reshape(H,W,3))
            oracle_frames[f"hold-film-half{half}"]=hold_halves[-1]
        hold_film=(hold_halves[0]+hold_halves[1])/2
    for key,current in last_images.items():
        history = histories[key]
        curve,film_curve=[],[]
        lag_before=float(np.sqrt(np.mean((history[0]-current).astype("f8")**2)/max(1e-20,np.mean(current.astype("f8")**2))))
        for age in range(1,9):
            history = temporal(history,current,guides,guides,pose,scene,recipe["rect"])
            curve.append(float(np.sqrt(np.mean((history[0]-reference).astype("f8")**2)/max(1e-20,np.mean(reference.astype("f8")**2)))))
            if hold_film is not None:
                film_curve.append(float(np.sqrt(np.mean((history[0]-hold_film).astype("f8")**2)/max(1e-20,np.mean(hold_film.astype("f8")**2)))))
        settle[key] = curve
        if hold_film is not None:
            film_settle[key]=dict(rms_by_age=film_curve, fresh_rms=float(np.sqrt(np.mean((current-hold_film).astype("f8")**2)/max(1e-20,np.mean(hold_film.astype("f8")**2)))),
                history_lag_before_hold=lag_before,history_lag_after_eight=float(np.sqrt(np.mean((history[0]-current).astype("f8")**2)/max(1e-20,np.mean(current.astype("f8")**2)))))
    d0.save(root/"scores.json",dict(rows=rows,hold_eight_frames=settle,physical_film_hold=film_settle,reference_control=reference_control,inputs=identity,
            history="one reprojected history after spatial filtering; primary ID/normal/plane validation, internal age cap 8; no reflected-hit reverse visibility; not TR1",recipe=recipe,
            film_label=args.film_label,
            reference="fixed 1024 or recipe oracle-K sub-cones, same R0b one-bounce transport, pixel centre; optional separately archived physical film sequence"))
    np.savez_compressed(root/"references.npz",**oracle_frames)
    codes = d0.load("m7-u0b-score").codes
    Image.fromarray(np.clip(codes(reference,-14),0,255).astype("u1")).save(root/"reference-inspected.png")
    for key in ("k8-none","k8-bilateral","k8-atrous","k8-hit-radius","k8-joint","k32-none"):
        Image.fromarray(np.clip(codes(last_images[key],-14),0,255).astype("u1")).save(root/(key+"-inspected.png"))
    print(root/"scores.json")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("command", choices=["prepare", "prepare-film", "run", "score"])
    p.add_argument("--label", required=True)
    p.add_argument("--fixture", default="RF4")
    p.add_argument("--pose", default="pitch30")
    p.add_argument("--rect", nargs=2, type=int, default=[850,700])
    p.add_argument("--frames", type=int, default=9)
    p.add_argument("--oracle", type=int, default=1024)
    p.add_argument("--wait-seconds", type=int, default=0)
    p.add_argument("--source-label")
    p.add_argument("--film-label")
    p.add_argument("--hold-label")
    p.add_argument("--still",action="store_true")
    args = p.parse_args()
    if args.command == "prepare":
        d0.note("action=prepare small missing-data measurement shader; light text-only work")
        prepare(args)
    elif args.command == "prepare-film":
        d0.note("action=prepare small physical film reference shader; text-only")
        prepare_film(args)
    elif args.command == "run":
        run(args)
    else:
        deadline = time.monotonic()+args.wait_seconds
        while d0.memory()["available_gib"]<8 and time.monotonic()<deadline:
            time.sleep(1)
        with d0.cpu_job("supplement-"+args.label):
            score(args)


if __name__ == "__main__":
    main()
