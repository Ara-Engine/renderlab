"""Independent double-precision triangle integrator on S1 film witness pixels.

CPU primary/any-hit intersections and a scrambled Sobol source disk cross-check
the existing GPU reference. Both the registered 10-um origin and zero-offset
physical diagnostics are retained. This is sampled validation, not a full-frame
path-tracer/radiance certificate.
"""
import argparse
import importlib.util
import json
import math
from pathlib import Path

import numpy as np
from numba import njit,prange,set_num_threads
from scipy.stats import qmc


@njit
def hit(origin,direction,triangles,tmin):
    best=1e30;index=-1
    for i in range(len(triangles)):
        a,b,c=triangles[i];u=b-a;v=c-a
        p=np.cross(direction,v);det=np.dot(u,p)
        if abs(det)<1e-15:continue
        s=origin-a;bu=np.dot(s,p)/det
        if bu<0 or bu>1:continue
        q=np.cross(s,u);bv=np.dot(direction,q)/det
        if bv<0 or bu+bv>1:continue
        t=np.dot(v,q)/det
        if t>tmin and t<best:best=t;index=i
    return index,best


@njit
def hash32(x):
    x=np.uint64(x)&np.uint64(0xffffffff)
    x=((x^(x>>np.uint64(16)))*np.uint64(0x7feb352d))&np.uint64(0xffffffff)
    x=((x^(x>>np.uint64(15)))*np.uint64(0x846ca68b))&np.uint64(0xffffffff)
    return (x^(x>>np.uint64(16)))&np.uint64(0xffffffff)


@njit(parallel=True)
def integrate(coords,triangles,placements,normals,thin,eye,forward,right,up,to_light,directions,epsilon):
    result=np.ones(len(coords));accepted=np.zeros(len(coords),np.int32)
    f=1080/(2*math.tan(math.pi/8))
    for k in prange(len(coords)):
        x,y=coords[k];centre_ray=forward+((x+.5-960)/f)*right+((540-y-.5)/f)*up
        ci,cd=hit(eye,centre_ray,triangles,1e-10)
        if ci<0:continue
        total=0.;n=0
        for s in range(256):
            h=hash32(np.uint64(s)^np.uint64(0x83d2e19b))
            qx=np.float32(h>>np.uint64(8))*np.float32(1/16777216)
            qy=np.float32(hash32(h)>>np.uint64(8))*np.float32(1/16777216)
            jx=np.float32((np.float32(s%16)+qx)/np.float32(16))
            jy=np.float32((np.float32(s//16)+qy)/np.float32(16))
            ray=forward+((x+jx-960)/f)*right+((540-y-jy)/f)*up
            i,d=hit(eye,ray,triangles,1e-10)
            if i<0 or placements[i]!=placements[ci] or abs(d-cd)>.02*cd:continue
            point=eye+d*ray;normal=normals[i].copy()
            if np.dot(normal,eye-point)<0:normal=-normal
            visible=0
            for direction in directions:
                oriented=normal
                if thin[i] and np.dot(normal,direction)<0:oriented=-normal
                origin=point+epsilon*oriented
                blocker,_=hit(origin,direction,triangles,max(epsilon,1e-10))
                visible += blocker<0
            total+=visible/len(directions);n+=1
        if n:result[k]=total/n
        accepted[k]=n
    return result,accepted


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--root",type=Path,required=True)
    parser.add_argument("--solid-witnesses",action="store_true",help="audit the two largest Temporal solid errors per pose")
    args=parser.parse_args();set_num_threads(2);root=args.root
    s=json.loads((root/"fixture/scene.json").read_text());triangles=[];placements=[];normals=[];thin=[]
    for mi,m in enumerate(s["meshes"]):
        p=np.array(m["positions"],float)
        for idx in np.array(m["indices"]).reshape(-1,3):
            triangles.append(p[idx]);placements.append(mi);normals.append(m["normals"][0]);thin.append(m["thin"])
    triangles=np.array(triangles);placements=np.array(placements);normals=np.array(normals);thin=np.array(thin)
    to_light=np.array(s["to_light"]);basis=np.cross(to_light,[0,1,0]);basis/=np.linalg.norm(basis);other=np.cross(to_light,basis)
    disk=qmc.Sobol(2,scramble=True,seed=7143).random_base2(10)
    directions=to_light+math.tan(.04)*np.sqrt(disk[:,0,None])*(np.cos(2*np.pi*disk[:,1,None])*basis+np.sin(2*np.pi*disk[:,1,None])*other)
    directions/=np.linalg.norm(directions,axis=1,keepdims=True)
    results={}
    for pose in (0,1):
        ref=np.load(root/f"renderlab/p{pose}-black/reference.npz")
        yy,xx=np.nonzero(ref["cover"]&(ref["ref"]>.01)&(ref["ref"]<.99))
        select=np.linspace(0,len(xx)-1,12).astype(int);coords=np.stack((xx[select],yy[select]),axis=1).astype(np.int64)
        if args.solid_witnesses:
            witnesses=json.loads((root/"solid-witnesses.json").read_text())[str(pose)][:2]
            coords=np.array([[p["x"],p["y"]] for p in witnesses],dtype=np.int64)
        eye=np.array(s["poses"][pose]["eye"]);forward=np.array(s["poses"][pose]["target"])-eye;forward/=np.linalg.norm(forward)
        right=np.cross(forward,[0,1,0]);right/=np.linalg.norm(right);up=np.cross(right,forward)
        cpu,n=integrate(coords,triangles,placements,normals,thin,eye,forward,right,up,to_light,directions,1e-5)
        physical,physical_n=integrate(coords,triangles,placements,normals,thin,eye,forward,right,up,to_light,directions,0.)
        gpu=ref["ref"][coords[:,1],coords[:,0]];sigma=ref["sigma"][coords[:,1],coords[:,0]]
        # Conservative independent light-integration uncertainty, plus GPU split half.
        bound=5*np.sqrt(sigma*sigma+cpu*(1-cpu)/len(directions))+1/256
        results[str(pose)]=dict(pixels=coords.tolist(),cpu_visibility=cpu.tolist(),gpu_visibility=gpu.tolist(),accepted=n.tolist(),
             gpu_split_sigma=sigma.tolist(),bound=bound.tolist(),max_error=float(np.max(np.abs(cpu-gpu))),
             passed=bool(np.all(np.abs(cpu-gpu)<=bound)),physical_zero_offset=physical.tolist(),physical_accepted=physical_n.tolist(),
             physical_policy_max_delta=float(np.max(np.abs(physical-cpu))))
    (root/("oracle-solids.json" if args.solid_witnesses else "oracle.json")).write_text(json.dumps(dict(method="f64 Moller-Trumbore, same 256 footprint strata, independent 1024-point scrambled Sobol disk",results=results),indent=2))
    print(json.dumps(results,indent=2))


if __name__ == "__main__":main()
