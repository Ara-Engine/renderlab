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
    def frame(name,p,spp=1024,close=None,sky_only=False):
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
            frame(f"{track}-{i:03}",p,512 if track=="q-walk" else 256)
        for i in ((0,1,4,8,16,32,48,64,65,72,95,96,104,128,160) if track=="q-walk" else (0,1,4,8,16,32,47)):
            p=seq[i];frame(f"{track}-{i:03}-shutter",seq[max(0,i-1)],128,close=p)
    distinct={}
    for f in frames:
        key=json.dumps([f[k] for k in ("open","close","width","height","samples","sky_only")],sort_keys=True)
        if key in distinct:f["alias_of"]=distinct[key]
        else:distinct[key]=f["name"]
    manifest={k:base[k] for k in ("asset","environment","lighting","cameras")};manifest["frames"]=frames
    manifest["sequence_sha256"]={name:__import__("hashlib").sha256(Path(f"E:/renderlab-work/t1-gpu/sequences/{name}.json").read_bytes()).hexdigest() for name in ("q-walk","q-disocclusion")}
    (OUT/"quarry.json").write_text(json.dumps(manifest,indent=2))
    smoke=dict(manifest,frames=[dict(frames[0],name="quarry-smoke",samples=16,width=480,height=270)])
    (OUT/"quarry-smoke.json").write_text(json.dumps(smoke,indent=2))


if __name__=="__main__":
    OUT.mkdir(parents=True,exist_ok=True);textured_crosscheck();quarry()
