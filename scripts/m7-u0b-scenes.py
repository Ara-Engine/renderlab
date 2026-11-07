"""Generate the textured independent cross-check and exact quarry camera manifests (CPU only)."""
import importlib.util
import json
import math
from pathlib import Path

import numpy as np
from PIL import Image

REPO=Path(__file__).resolve().parent.parent
OUT=Path("E:/renderlab-work/m7-u0b")


def textured_crosscheck():
    spec=importlib.util.spec_from_file_location("scenes",REPO/"scripts/m7-u0-scenes.py")
    s=importlib.util.module_from_spec(spec);spec.loader.exec_module(s)
    scene=s.x1();scene.update(bench="X3",size=[160,120])
    scene["materials"][0]["base_color"]=[1.,1.,1.]
    scene["materials"][0]["base_texture"]=0
    path=Path("E:/renderlab-work/m7-u0/scenes/X3");path.mkdir(parents=True,exist_ok=True)
    y,x=np.indices((64,64))
    pixels=np.stack([40+180*((x//8+y//8)%2),50+150*x/63,30+200*y/63],-1).astype("u1")
    Image.fromarray(pixels).save(path/"texture.png")
    mips=[]
    for level in range(7):
        size=64>>level
        im=np.asarray(Image.fromarray(pixels).resize((size,size),Image.Resampling.BOX))
        rgba=np.concatenate([im,np.full((size,size,1),255,dtype="u1")],-1)
        name=f"mip-{level}.rgba8";(path/name).write_bytes(rgba.tobytes())
        mips.append(dict(width=size,height=size,file=name))
    scene["textures"]=[dict(mips=mips)]
    for mesh in scene["meshes"]:
        mesh["uvs"]=([[0,0],[1,0],[1,1],[0,1]]*(len(mesh["positions"])//4)
                     if mesh["name"]!="sphere" else [[0,0]]*len(mesh["positions"]))
    (path/"scene.json").write_text(json.dumps(scene,separators=(",",":")))
    mi=path/"mitsuba";s.write_mitsuba(scene,mi)
    # Rewrite only generated PLY vertex layouts; Mitsuba remains an external BSD-3 tool.
    for mesh in scene["meshes"]:
        p=np.asarray(mesh["positions"],"<f4");n=np.asarray(mesh["normals"],"<f4");uv=np.asarray(mesh["uvs"],"<f4")
        idx=np.asarray(mesh["indices"],"<u4").reshape(-1,3)
        header=(f"ply\nformat binary_little_endian 1.0\nelement vertex {len(p)}\nproperty float x\nproperty float y\nproperty float z\n"
                f"property float nx\nproperty float ny\nproperty float nz\nproperty float s\nproperty float t\nelement face {len(idx)}\nproperty list uchar uint vertex_indices\nend_header\n")
        faces=np.zeros(len(idx),dtype=[("n","u1"),("i","<u4",3)]);faces["n"]=3;faces["i"]=idx
        (mi/(mesh["name"]+".ply")).write_bytes(header.encode()+np.hstack([p,n,uv]).astype("<f4").tobytes()+faces.tobytes())
    xml=(mi/"scene.xml").read_text()
    xml=xml.replace('<rgb name="reflectance" value="1.0, 1.0, 1.0"/>',
                    '<texture type="bitmap" name="reflectance"><string name="filename" value="../texture.png"/>'
                    '<string name="filter_type" value="bilinear"/><string name="wrap_mode" value="repeat"/></texture>',1)
    (mi/"scene.xml").write_text(xml)


def quarry():
    base=json.loads(Path("D:/Coding/renderlab-work/m6-interactive-climb/runs/timing-c11-1.manifest.json").read_text())
    poses=json.loads(Path(base["cameras"]).read_text())["frames"]
    frames=[]
    def frame(name,p,spp=4096,close=None,sky_only=False):
        frames.append(dict(name=name,open={k:p[k] for k in ("eye","target")},close={k:(close or p)[k] for k in ("eye","target")},
                           fov_y=p.get("fov_y_radians",.85),width=1920,height=1080,samples=spp,sky_only=sky_only))
    for i,p in enumerate(poses):
        frame(f"quarry-p{i}",p);frame(f"quarry-p{i}-sky",p,sky_only=True)
    # Same pure translation as U0 pan (2 cm in source +X / frame), zero shutter plus
    # a one-frame box shutter. Explicit endpoints make the temporal integration reproducible.
    for index in (97,112,128,159):
        p=dict(poses[0]);shift=.02*(index-96)
        p["eye"]=[poses[0]["eye"][0]+shift,*poses[0]["eye"][1:]]
        p["target"]=[poses[0]["target"][0]+shift,*poses[0]["target"][1:]]
        prev=dict(p,eye=[p["eye"][0]-.02,*p["eye"][1:]],target=[p["target"][0]-.02,*p["target"][1:]])
        frame(f"pan-{index:03}",p,256);frame(f"pan-{index:03}-shutter",prev,256,close=p)
    for track in ("q-walk","q-disocclusion"):
        seq=json.loads(Path(f"E:/renderlab-work/t1-gpu/sequences/{track}.json").read_text())["frames"]
        for i,p in enumerate(seq):
            frame(f"{track}-{i:03}",p,1024)
        for i in (sorted(set(range(32))|{48,64,65,72,95,96,104,120,128,160}) if track=="q-walk" else (0,1,4,8,16,32,47)):
            p=seq[i];frame(f"{track}-{i:03}-shutter",seq[max(0,i-1)],512,close=p)
    distinct={}
    for f in frames:
        key=json.dumps([f[k] for k in ("open","close","fov_y","width","height","samples","sky_only")],sort_keys=True)
        if key in distinct:f["alias_of"]=distinct[key]
        else:distinct[key]=f["name"]
    manifest={k:base[k] for k in ("asset","environment","lighting","cameras")};manifest["frames"]=frames
    manifest["camera_background"]=[0,0,0]
    manifest["sequence_sha256"]={name:__import__("hashlib").sha256(Path(f"E:/renderlab-work/t1-gpu/sequences/{name}.json").read_bytes()).hexdigest() for name in ("q-walk","q-disocclusion")}
    (OUT/"quarry.json").write_text(json.dumps(manifest,indent=2))
    for name,selected in (("quarry-static",frames[:4]),("quarry-pan",frames[4:12]),("quarry-temporal",frames[12:])):
        (OUT/(name+".json")).write_text(json.dumps(dict(manifest,frames=selected),indent=2))
    by_name={f["name"]:f for f in frames};priority=[];seen=set()
    def add(name):
        if name in seen:return
        f=by_name[name]
        if "alias_of" in f:add(f["alias_of"])
        seen.add(name);priority.append(f)
    for i in (0,1,47,48,49,63,64,56,65,95,96,97,119,120,121,127,128,129,159,160):add(f"q-walk-{i:03}")
    # TR0's original per-measure witness set, ahead of the broader contiguous campaign.
    scored=(0,1,2,3,4,5,6,7,8,12,16,20,24,28,31,32,33,36,40,44,48,52,56,60,61,64,
            68,72,76,80,81,84,88,92,96,99,100,104,108,112,116,120,124,127,128,129,
            132,136,140,144,148,152,153,154,155,156,157,158,159,160)
    for i in scored:add(f"q-walk-{i:03}")
    (OUT/"quarry-t1b-witnesses.json").write_text(json.dumps(dict(manifest,frames=list(priority)),indent=2))
    for f in frames[12:]:add(f["name"])
    (OUT/"quarry-priority.json").write_text(json.dumps(dict(manifest,frames=priority),indent=2))
    smoke=dict(manifest,frames=[dict(frames[0],name="quarry-smoke",samples=16,width=480,height=270)])
    (OUT/"quarry-smoke.json").write_text(json.dumps(smoke,indent=2))


def benches():
    """Explicit primary compositing control. Preserve U0's original scene identities and
    package; only the reference recipe changes. Every secondary ray still sees the sky."""
    import hashlib
    controls={}
    for name in ("SV","S4","S5","S2G"):
        source=Path("E:/renderlab-work/m7-u0/scenes")/name/"scene.json"
        scene=json.loads(source.read_text())
        identity=hashlib.sha256(source.read_bytes()).hexdigest()
        scene["camera_background"]=[0.,0.,0.]
        scene["reference_control"]=dict(source_scene_sha256=identity,primary_background="black; matches UE specified-lighting-cube maps",secondary_environment="unchanged")
        out=OUT/"scenes"/name;out.mkdir(parents=True,exist_ok=True)
        target=out/"scene.json";target.write_text(json.dumps(scene,separators=(",",":")))
        controls[name]=dict(source_scene_sha256=identity,reference_scene_sha256=hashlib.sha256(target.read_bytes()).hexdigest(),
                            primary_background=[0,0,0],silhouettes="per-sample primary compositing; no sky mask")
    (OUT/"bench-background-controls.json").write_text(json.dumps(controls,indent=2))


if __name__=="__main__":
    OUT.mkdir(parents=True,exist_ok=True);textured_crosscheck();quarry();benches()
