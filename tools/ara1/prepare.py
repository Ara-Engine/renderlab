"""Generate private matched scenes and replay manifests (CPU only).

Produces a source-faithful frozen game geometry subset and uses the existing quarry
RLA in place. Ara's original PBR sample is exported by the external Ara adapter.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct
import subprocess

import numpy as np

ROOT = Path("D:/Coding/renderlab-work/ara1")


def save(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n")


def sphere(segments=48, rings=24):
    vertices = []
    for r in range(rings+1):
        p = math.pi*r/rings
        for s in range(segments+1):
            a = math.tau*s/segments
            n = [math.sin(p)*math.cos(a), math.cos(p), math.sin(p)*math.sin(a)]
            vertices.append(n+n+[s/segments,r/rings])
    indices = []
    for r in range(rings):
        for s in range(segments):
            a = r*(segments+1)+s
            b = a+segments+1
            indices += [a,a+1,b,a+1,b+1,b]
    return np.asarray(vertices, dtype="<f4"), np.asarray(indices, dtype="<u4")


def cube():
    vertices, indices = [], []
    for n,u,v in [([1,0,0],[0,0,-1],[0,1,0]),([-1,0,0],[0,0,1],[0,1,0]),
                  ([0,1,0],[1,0,0],[0,0,-1]),([0,-1,0],[1,0,0],[0,0,1]),
                  ([0,0,1],[1,0,0],[0,1,0]),([0,0,-1],[-1,0,0],[0,1,0])]:
        base = len(vertices)
        for s,t in [(-1,-1),(1,-1),(1,1),(-1,1)]:
            p = (np.asarray(n)+s*np.asarray(u)+t*np.asarray(v))/2
            vertices.append([*p,*n,.5+.5*s,.5-.5*t])
        indices += [base,base+1,base+2,base,base+2,base+3]
    return np.asarray(vertices,"<f4"),np.asarray(indices,"<u4")


def plane(size=60, cells=8):
    vertices, indices = [], []
    for z in range(cells+1):
        for x in range(cells+1):
            vertices.append([(x/cells-.5)*size,0,(z/cells-.5)*size,0,1,0,x/cells*size,z/cells*size])
    for z in range(cells):
        for x in range(cells):
            a=z*(cells+1)+x
            indices += [a,a+cells+1,a+1,a+1,a+cells+1,a+cells+2]
    return np.asarray(vertices,"<f4"),np.asarray(indices,"<u4")


def game_glb(path):
    """Re-express original game_scene.rs geometry/layout as glTF, frozen at t=0."""
    data = bytearray()
    doc = dict(asset=dict(version="2.0",generator="Renderlab ARA1 local measurement adapter"),
               buffers=[{}],bufferViews=[],accessors=[],materials=[],meshes=[],nodes=[],scenes=[dict(nodes=[])],scene=0)
    for color,metal,rough in [([.62,.6,.56],0,.9),([.56,.36,.18],0,.75),([.8,.09,.06],0,.6),
                            ([1,.78,.34],1,.25),([.1,.3,.85],0,.35),([.95,.95,.97],1,.02),
                            ([.95,.55,.2],0,.5),([.34,.37,.28],0,.95)]:
        doc["materials"].append(dict(pbrMetallicRoughness=dict(baseColorFactor=color+[1],metallicFactor=metal,roughnessFactor=rough),doubleSided=True))
    def accessor(array, component, kind, target):
        while len(data)%4:data.append(0)
        raw=array.tobytes();offset=len(data);data.extend(raw)
        view=len(doc["bufferViews"])
        doc["bufferViews"].append(dict(buffer=0,byteOffset=offset,byteLength=len(raw),target=target))
        item=dict(bufferView=view,componentType=component,count=len(array),type=kind)
        if kind=="VEC3":item.update(min=array.min(0).tolist(),max=array.max(0).tolist())
        doc["accessors"].append(item)
        return len(doc["accessors"])-1
    shapes=[]
    for vertices,indices in [cube(),sphere(),plane()]:
        shapes.append(dict(attributes=dict(POSITION=accessor(vertices[:,:3].copy(),5126,"VEC3",34962),
            NORMAL=accessor(vertices[:,3:6].copy(),5126,"VEC3",34962),TEXCOORD_0=accessor(vertices[:,6:8].copy(),5126,"VEC2",34962)),
            indices=accessor(indices,5125,"SCALAR",34963),mode=4))
    def add(name, shape, material, position, scale=(1,1,1), turn=0):
        primitive=shapes[shape] | dict(material=material)
        mesh=len(doc["meshes"]);doc["meshes"].append(dict(name=name,primitives=[primitive]))
        node=dict(name=name,mesh=mesh,translation=list(position),scale=list(scale),rotation=[0,math.sin(turn/2),0,math.cos(turn/2)])
        doc["scenes"][0]["nodes"].append(len(doc["nodes"]));doc["nodes"].append(node)
    add("ground",2,7,(0,0,0))
    for x in (-2.2,2.2):add("arch-pillar",0,0,(x,2,-3),(.8,4,.8))
    add("arch-lintel",0,0,(0,4.4,-3),(5.2,.8,1))
    add("red-wall",0,2,(-5.5,1.5,0),(.3,3,6))
    for x,y,z,turn in [(4,.5,1,.1),(5.1,.5,1.3,-.25),(4.5,1.5,1.1,.4),(4.2,.5,2.3,0)]:
        add("crate",0,1,(x,y,z),turn=turn)
    add("gold",1,3,(1.8,1,3.2))
    add("chrome",1,5,(-1.2,.8,5.6),(.8,.8,.8))
    add("spinner-frozen",0,4,(-2.5,.6,3))
    add("hero-body-standard-material",1,6,(0,.55,2),(.55,.55,.55))
    add("hero-head-standard-material",1,6,(0,1.35,2),(.35,.35,.35))
    doc["buffers"][0]["byteLength"]=len(data)
    encoded=json.dumps(doc,separators=(",",":")).encode()
    encoded+=b" "*((-len(encoded))%4);data+=b"\0"*((-len(data))%4)
    path.write_bytes(struct.pack("<III",0x46546C67,2,28+len(encoded)+len(data))+
                     struct.pack("<II",len(encoded),0x4E4F534A)+encoded+struct.pack("<II",len(data),0x004E4942)+data)


def frames(eye,target,fov,dry):
    counts=(2,2,2) if dry else (40,32,16)
    result=[]
    for phase,count in zip(("still","move","cut"),counts):
        for i in range(count):
            shift=0 if phase=="still" else (.02*i if phase=="move" else -.8)
            result.append(dict(eye=[eye[0]+shift,*eye[1:]],target=[target[0]+shift,*target[1:]],fov_y=fov,
                               phase=phase,cut=phase=="cut" and i==0,capture=dry or i==count-1 or (phase=="cut" and i in (0,1,2,4,8))))
    return result


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--root",type=Path,default=ROOT)
    p.add_argument("--size",default="320x180")
    p.add_argument("--dry",action="store_true")
    p.add_argument("--startup-minimal",action="store_true",help="32x18, 1024 Renderlab GI slots / eight Ara probes; unqualified startup smoke only")
    p.add_argument("--prepare-binary",type=Path)
    p.add_argument("--ara-binary",type=Path)
    p.add_argument("--quarry-manifest",type=Path,default=Path("D:/Coding/renderlab-work/m6-interactive-climb/runs/timing-c11-1.manifest.json"))
    a=p.parse_args();folder=a.root/("scenes-minimal" if a.startup_minimal else "scenes-dry" if a.dry else "scenes");folder.mkdir(parents=True,exist_ok=True)
    size=[32,18] if a.startup_minimal else [int(n) for n in a.size.split("x")]
    glb=folder/"game.glb";game_glb(glb)
    if a.prepare_binary:subprocess.run([str(a.prepare_binary),str(glb),str(folder/"game.rla")],check=True)
    if a.ara_binary:subprocess.run([str(a.ara_binary),"export-sample",str(folder/"ara-pbr.rla")],check=True)
    common=dict(version=1,size=size,to_light=[.45,.75,.5],sun_lux=[120000]*3,sun_radius=.00465,environment=[3500,5000,7500],
                exposure_stops=-14.5,lights=[],startup_minimal=a.startup_minimal)
    game=dict(common,name="game-opaque-frozen",asset="game.rla",frames=frames([14*math.sin(.6),5.5,14*math.cos(.6)],[0,.775,1],math.radians(55),a.dry),
              notes=["game_scene.rs layout/geometry at t=0; standard PBR replaces the programmable toon hero", "no emblem/wet decal, rigid animation, particles, skinning or participating medium in this matched subset", "uniform sky on both engines; original atmosphere run remains a separate feature row", "sun disk radius 0.00465 rad matches Ara main's fixed fidelity sun, versus game_scene's original 0.04 rad"])
    save(folder/"game.json",game)
    sample=dict(common,name="ara-pbr-opaque",asset="ara-pbr.rla",to_light=[-.6,1,.4],sun_lux=[3500]*3,environment=[30,35,45],exposure_stops=math.log2(.0015),
                frames=frames([10,8,14],[0,1,.2],math.radians(50),a.dry),
                notes=["Ara's original common sample exporter; cyan emissive and blended sphere omitted explicitly", "uniform environment replaces authored studio HDR on both engines; no media", "original mesh/tangent/UV/material factors retained; shared generated mips from source mip zero"])
    save(folder/"ara-pbr.json",sample)
    if a.quarry_manifest.exists():
        original=json.loads(a.quarry_manifest.read_text());poses=json.loads(Path(original["cameras"]).read_text())["frames"]
        light=json.loads(Path(original["lighting"]).read_text());sun=light["lights"][0]
        quarry=dict(common,name="quarry-neutral",asset=original["asset"],to_light=[-x for x in sun["travel"]],sun_lux=sun["lux"],
                    environment=light["environment"],exposure_stops=light["exposure_stops"],
                    frames=frames(poses[0]["eye"],poses[0]["target"],poses[0]["fov_y_radians"],a.dry),
                    notes=["original full-detail private quarry RLA; local evaluation only, no redistribution rights asserted", "uniform sky replaces RLE2 environment on both engines: existing HDR quarry references do not match this case", "camera near=0.1m for Ara compatibility; original Renderlab near was 0.005m", "no geometry decimation; no source asset copied into the public repository"])
        save(folder/"quarry.json",quarry)
    identities={f.name:hashlib.sha256(f.read_bytes()).hexdigest() for f in folder.glob("*.json")}
    save(folder/"identities.json",identities)
    print(folder)


if __name__=="__main__":main()
