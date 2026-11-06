"""S2/S3 authored geometry and fixed 60 Hz tracks, shared by both engines."""
import argparse
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import struct

import numpy as np


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def sun(azimuth, elevation):
    a, e = map(math.radians, (azimuth, elevation))
    return [math.cos(e)*math.sin(a), math.sin(e), math.cos(e)*math.cos(a)]


def quad(meshes, name, points, roughness=.6, moving=False, thin=False):
    p = np.asarray(points, dtype=float)
    n = np.cross(p[1]-p[0], p[2]-p[0]); n /= np.linalg.norm(n)
    meshes.append(dict(name=name, positions=p.tolist(), normals=[n.tolist()]*4,
                       indices=[0, 1, 2, 0, 2, 3], thin=thin, roughness=roughness, moving=moving))


def box(meshes, name, center, extent):
    c = np.array(center); e = np.array(extent)/2
    for axis in range(3):
        for side in (-1, 1):
            other = [a for a in range(3) if a != axis]
            points = []
            for u, v in ((-1,-1),(1,-1),(1,1),(-1,1)):
                p = c.copy(); p[axis] += side*e[axis]; p[other] += e[other]*[u,v]; points.append(p)
            if np.cross(points[1]-points[0], points[2]-points[0])[axis]*side < 0:
                points.reverse()
            quad(meshes, f"{name}-{axis}-{side}", points)


def base(bench):
    return dict(schema=1, bench=bench, size=[1920,1080], meshes=[], tracks=[], poses=[],
                sun_lux=[1000]*3, exposure_stops=-8, environment=[0,0,0], sun_radius=.04,
                material=dict(base_color=[.5,.5,.5,1], roughness=.6, metallic=0, double_sided=True),
                registration=dict(reference="S1 256 jittered primaries x 256 salted sun rays; native-pixel crop",
                    contact_lost="contact & candidate-film > .5", leaks="umbra & candidate-film > .1",
                    solid_flips="lit error < -.5 or umbra error > .5", solid_gate=.002,
                    single_surface="all 256 reference primaries accepted on centre placement/depth",
                    shutter=0, hz=60, motion_repeats=8,
                    coverage_limit="Per-frame film metrics are registered native-pixel crops, not full-frame film acceptance.",
                    environment="Black environment and black visible background in both engines."))


def s2():
    s = base("S2"); m = s["meshes"]
    eye = np.array([0.,1.,3.]); target = np.array([0.,.2,0.])
    forward = target-eye; forward /= np.linalg.norm(forward)
    focal = 1080/(2*math.tan(math.radians(45)/2))
    depth = float(np.dot(-eye, forward)); metres_per_pixel = depth/focal
    s.update(fov_y_degrees=45, to_light=sun(35,20), crop=[704,480,1216,768],
             width_registration=dict(focal_pixels=focal, floor_depth_m=depth, metres_per_pixel=metres_per_pixel),
             poses=[dict(eye=eye.tolist(), target=target.tolist())])
    quad(m,"floor",[(-1,0,1),(1,0,1),(1,0,-1),(-1,0,-1)])
    for i, px in enumerate((.25,.5,1.,2.)):
        x = -.22+i*.145; width = px*metres_per_pixel
        # Vertical ribbons face the camera; their horizontal width is exact at their centre depth.
        z = 0.; y = .13; d = float(np.dot(np.array([x,y,z])-eye, forward)); width = px*d/focal
        quad(m,f"wire-{px:g}px",[(x-width/2,.03,z),(x+width/2,.03,z),(x+width/2,.23,z),(x-width/2,.23,z)],thin=True)
        m[-1].update(projected_width_px=px, width_m=width, centre_view_depth_m=d)
    a = np.array([-.23,.03,-.10]); b = a+np.array([.5/math.sqrt(2),0,-.5/math.sqrt(2)])
    offset = np.array([.0015,0,.0015])
    quad(m,"diagonal-bar",[a-offset,b-offset,b+offset,a+offset],thin=True)
    for label,x,rough in (("glossy",-.22,.1),("diffuse",.05,.6)):
        for j in range(4):
            z0=-.44+j*.05; z1=z0+.05
            h0=.002+(j%2)*.003;h1=.002+((j+1)%2)*.003
            quad(m,f"{label}-{j}",[(x-.1,h0,z0),(x+.1,h0,z0),(x+.1,h1,z1),(x-.1,h1,z1)],roughness=rough)
    quad(m,"shadow-blocker",[(-.05,.13,.04),(.20,.17,.04),(.20,.17,-.16),(-.05,.13,-.16)],moving=True,thin=True)
    quad(m,"reveal-plate",[(-.10,.02,.40),(.10,.02,.40),(.10,.32,.40),(-.10,.32,.40)],moving=True,thin=True)
    for step in (.25,.5,1.):
        frames=[]
        for frame in range(160):
            offset = step*metres_per_pixel*(frame if frame<64 else 64 if frame<96 else 159-frame)
            shift = np.array([offset,0,0]); plate = min(1,max(0,(frame-15)/16))*(1-min(1,max(0,(frame-111)/16)))*.40
            frames.append(dict(frame=frame,seconds=frame/60,eye=(eye+shift).tolist(),target=(target+shift).tolist(),
                               to_light=s["to_light"],plate_x=plate,blocker_x=0.,phase="out" if frame<64 else "hold" if frame<96 else "return"))
        s["tracks"].append(dict(name=f"camera-{step:g}px",frames=frames,pixels_per_frame=step))
    frames=[]
    for frame in range(160):
        offset = .001*(frame if frame<64 else 64 if frame<96 else 159-frame)
        frames.append(dict(frame=frame,seconds=frame/60,eye=eye.tolist(),target=target.tolist(),to_light=s["to_light"],
                           plate_x=0.,blocker_x=offset,phase="shadow-out" if frame<64 else "hold" if frame<96 else "shadow-return"))
    s["tracks"].append(dict(name="shadow-step",frames=frames))
    s["radii"]=[0.,.04]
    return s


def s3(cells=512):
    s=base("S3"); m=s["meshes"]
    s.update(fov_y_degrees=50,to_light=sun(35,15),crop=[768,480,1152,696],radii=[.04],
             poses=[dict(eye=[4,3,5],target=[0,0,0])],seed=0x51257,cells=cells)
    # Smooth seeded Fourier noise plus explicitly bounded 1--5 mm ridges.
    rng=np.random.default_rng(s["seed"]); phases=rng.uniform(0,2*math.pi,4)
    z,x=np.meshgrid(np.linspace(-2,2,cells+1),np.linspace(-2,2,cells+1),indexing="ij")
    y=.012*np.sin(x*2+phases[0])*np.cos(z*1.7+phases[1])+.006*np.sin(x*4.1+z*2.3+phases[2])
    y+=(.001+.004*(.5+.5*np.sin(z*1.3+phases[3])))*np.maximum(0,np.cos(x*30+z*7))**8
    p=np.stack([x,y,z],-1); dz,dx=np.gradient(y,4/cells,4/cells)
    n=np.stack([-dx,np.ones_like(y),-dz],-1);n/=np.linalg.norm(n,axis=-1)[...,None]
    a=np.arange(cells)[:,None]*(cells+1)+np.arange(cells)[None,:]
    indices=np.stack([a,a+cells+1,a+1,a+1,a+cells+1,a+cells+2],-1).ravel()
    m.append(dict(name="slab",positions=p.reshape(-1,3).tolist(),normals=n.reshape(-1,3).tolist(),indices=indices.tolist(),thin=False,roughness=.6,moving=False))
    for iz in range(8):
        for ix in range(8):
            xx=-1.75+ix*.5;zz=-1.75+iz*.5;h=.07+.08*float(rng.random())
            box(m,f"pillar-{iz}-{ix}",[xx,h/2-.025,zz],[.06,h+.05,.06])
    faces=m[1:]; merged=dict(name="pillars",positions=[],normals=[],indices=[],thin=False,roughness=.6,moving=False)
    for face in faces:
        start=len(merged["positions"]);merged["positions"].extend(face["positions"]);merged["normals"].extend(face["normals"])
        merged["indices"].extend(i+start for i in face["indices"])
    s["meshes"]=[m[0],merged]
    # 32 static + 128 slow + 32 hold + 128 fast + 32 hold + jump and 32 recovery.
    frames=[]
    az=35.
    for phase,count,rate in (("static",32,0),("rotate-0.25",128,.25),("hold-slow",32,0),
                             ("rotate-1",128,1.),("hold-fast",32,0),("jump-recovery",33,0)):
        if phase=="jump-recovery":az+=5
        for age in range(count):
            az+=rate/60;f=len(frames)
            frames.append(dict(frame=f,seconds=f/60,eye=[4,3,5],target=[0,0,0],to_light=sun(az,15),
                               azimuth_degrees=az,plate_x=0.,blocker_x=0.,phase=phase,phase_age=age))
    s["tracks"]=[dict(name="sun",frames=frames)]
    return s


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("bench",choices=["S2","S3"])
    parser.add_argument("--output",type=Path,required=True);parser.add_argument("--cells",type=int,default=512)
    args=parser.parse_args();out=args.output;out.mkdir(parents=True,exist_ok=True)
    s=s2() if args.bench=="S2" else s3(args.cells)
    s["triangles"]=sum(len(m["indices"])//3 for m in s["meshes"])
    load("bench-contact-fold-scene").write_gltf(s,out)
    n=16;header=[124,0x100f,n,n,n*16,0,1]+[0]*11+[32,4,int.from_bytes(b"DX10","little"),0,0,0,0,0]+[0x1008,0xfe00,0,0,0]
    (out/"constant.dds").write_bytes(b"DDS "+struct.pack("<31I",*header)+struct.pack("<5I",2,3,4,1,0)+struct.pack("<4f",1,1,1,1)*(6*n*n))
    (out/"scene.json").write_text(json.dumps(s,separators=(",",":")))
    (out/"identities.json").write_text(json.dumps({p.name:hashlib.file_digest(p.open("rb"),"sha256").hexdigest() for p in out.iterdir() if p.is_file() and p.name!="identities.json"},indent=2))
    print(json.dumps(dict(bench=args.bench,triangles=s["triangles"],frames=sum(len(t["frames"]) for t in s["tracks"]),crop=s["crop"])))


if __name__=="__main__":main()
