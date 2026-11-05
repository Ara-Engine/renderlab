"""CPU analytic film oracle for F0's existing thin/contact and speck/wire fixtures.

Integrates the continuous slope disk by polygon intervals with exact breakpoints
and 64-node Gauss-Legendre quadrature. Independent primary intersections are analytic
plane intersections. All pixels and all 256 registered footprint locations are used.
The scalar implementation is cross-checked against the frozen scipy adaptive oracle.

py -3 -B scripts/m6-film-fixtures.py --root D:/Coding/renderlab-work/m6-exact-quality/f0
"""
import argparse
import importlib.util
import json
import math
from pathlib import Path
import sys
import time

import numpy as np
from numba import njit, prange, set_num_threads

spec=importlib.util.spec_from_file_location("film_score",Path(__file__).with_name("m6-film-score.py"))
score=importlib.util.module_from_spec(spec);spec.loader.exec_module(score)
import fresh_gate as frozen

GX,GW=np.polynomial.legendre.leggauss(64)
S=math.tan(.04)


@njit
def hash32(x):
    x=np.uint64(x)&np.uint64(0xffffffff)
    x=((x^(x>>np.uint64(16)))*np.uint64(0x7feb352d))&np.uint64(0xffffffff)
    x=((x^(x>>np.uint64(15)))*np.uint64(0x846ca68b))&np.uint64(0xffffffff)
    return (x^(x>>np.uint64(16)))&np.uint64(0xffffffff)


@njit
def jitters():
    out=np.zeros((256,2))
    for i in range(256):
        h=hash32(np.uint64(i)^np.uint64(0x83d2e19b))
        # Round exactly as the WGSL conversion and arithmetic do.
        qx=np.float32(h>>np.uint64(8))*np.float32(1/16777216)
        qy=np.float32(hash32(h)>>np.uint64(8))*np.float32(1/16777216)
        out[i,0]=np.float32((np.float32(i%16)+qx)/np.float32(16))
        out[i,1]=np.float32((np.float32(i//16)+qy)/np.float32(16))
    return out


@njit
def disk_visibility(x,y,z,rects):
    # Each projected rectangle is the intersection of four linear half-planes
    # a*u+b*v+c>=0. Reject a rectangle only when a plane excludes the entire disk.
    planes=np.empty((len(rects),4,3));nr=0
    for r in rects:
        cx,cy,rz,hx,hy,angle=r
        h=rz-z
        if h<=0:continue
        c=math.cos(angle);sn=math.sin(angle)
        dx=abs(c)*hx+abs(sn)*hy;dy=abs(sn)*hx+abs(c)*hy
        reach_y=h/((1-S)/math.sqrt(2))*S
        if (x+h*(1+S)/(1-S)<cx-dx or x+h*(1-S)/(1+S)>cx+dx
                or y+reach_y<cy-dy or y-reach_y>cy+dy):continue
        keep=True
        for axis in range(2):
            nx=c if axis==0 else -sn;ny=sn if axis==0 else c;half=hx if axis==0 else hy
            base=nx*(x-cx)+ny*(y-cy)
            aa=S*(-base+nx*h);b=ny*h*S*math.sqrt(2);cc=base+nx*h
            vals=((-aa-half*S,-b,half-cc),(aa-half*S,b,cc+half))
            for side in range(2):
                a0,b0,c0=vals[side];planes[nr,axis*2+side]=np.array([a0,b0,c0])
                if c0+math.hypot(a0,b0)<0:keep=False
        if keep:nr+=1
    if nr==0:return 1.
    # Split at circle/edge and edge/edge crossings, so the interval-union
    # expression stays smooth on every integration span.
    flat=planes[:nr].reshape(nr*4,3)
    cuts=np.empty(2+len(flat)*2+len(flat)*len(flat));nc=2
    cuts[0]=-1;cuts[1]=1
    for i in range(len(flat)):
        a,b,c=flat[i];norm=a*a+b*b
        if norm>0 and c*c<=norm:
            mid=-a*c/norm;delta=abs(b)*math.sqrt(max(0,1-c*c/norm)/norm)
            cuts[nc]=max(-1.,min(1.,mid-delta));cuts[nc+1]=max(-1.,min(1.,mid+delta));nc+=2
        for j in range(i):
            aa,bb,cc=flat[j];det=a*bb-aa*b
            if abs(det)>1e-15:
                cuts[nc]=max(-1.,min(1.,(b*cc-bb*c)/det));nc+=1
    ordered=np.sort(cuts[:nc]);total=0.
    intervals=np.empty((nr,2))
    for ci in range(nc-1):
        left,right=ordered[ci],ordered[ci+1]
        if right-left<1e-12:continue
        for qi in range(len(GX)):
            u=(left+right)/2+(right-left)/2*GX[qi]
            circle=math.sqrt(max(0,1-u*u));ni=0
            for r in range(nr):
                lo,hi=-circle,circle
                for k in range(4):
                    a,b,c=planes[r,k]
                    if abs(b)<1e-15:
                        if a*u+c<0:lo=2.;break
                    elif b>0:lo=max(lo,(-a*u-c)/b)
                    else:hi=min(hi,(-a*u-c)/b)
                if hi>lo:
                    # Sorted interval insertion; nr is normally one or two.
                    pos=ni
                    while pos>0 and intervals[pos-1,0]>lo:
                        intervals[pos]=intervals[pos-1];pos-=1
                    intervals[pos,0]=lo;intervals[pos,1]=hi;ni+=1
            reach=-2.;length=0.
            for k in range(ni):
                lo,hi=intervals[k]
                length+=max(0,hi-max(lo,reach));reach=max(reach,hi)
            total+=(right-left)/2*GW[qi]*length
    value=1-total/math.pi
    return 0. if value<1e-9 else 1. if value>1-1e-9 else value


@njit(parallel=True)
def film(rects,centre_hit,centre_depth,bias,jitter):
    h,w=centre_hit.shape
    ref=np.ones((h,w));accepted=np.zeros((h,w),np.int32)
    scale=math.tan(math.radians(55)/2)*2/h
    for y in prange(h):
        for x in range(w):
            if centre_hit[y,x]==0:continue
            total=0.;n=0
            for i in range(256):
                ox=(x+jitter[i,0]-w/2)*scale
                oy=(h/2-y-jitter[i,1])*scale
                z=0.;instance=1
                for ri in range(len(rects)):
                    cx,cy,rz,hx,hy,angle=rects[ri]
                    if rz<=z:continue
                    px=ox*(4-rz)-cx;py=oy*(4-rz)-cy
                    c=math.cos(angle);s=math.sin(angle)
                    if abs(c*px+s*py)<=hx and abs(-s*px+c*py)<=hy:
                        z=rz;instance=ri+2
                depth=4-z
                if instance!=centre_hit[y,x] or abs(depth-centre_depth[y,x])>.02*centre_depth[y,x]:continue
                total+=disk_visibility(ox*depth,oy*depth,z+bias,rects);n+=1
            if n>0:ref[y,x]=total/n
            accepted[y,x]=n
    return ref,accepted


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument("--root",type=Path,required=True)
    p.add_argument("--workload",choices=("thin","stress","both"),default="both")
    p.add_argument("--policy",choices=("policy","minimal","both"),default="both")
    a=p.parse_args();start=time.perf_counter();set_num_threads(2)
    results={}
    for work in ("thin","stress"):
        if a.workload not in (work,"both"):continue
        for policy,bias in (("policy",.001),("minimal",1e-5)):
            if a.policy not in (policy,"both"):continue
            directory=a.root/f"{work}-{policy}";d=score.decode(directory,0)
            rectangles=json.loads((directory/"rectangles.json").read_text())
            rects=np.array([[*r["center"],*r["half"],r["angle"]] for r in rectangles])
            # Independent point-oracle cross-check at representative ground pixels.
            checks=[]
            for y,x in zip(*np.nonzero((d["ref"]>0)&(d["ref"]<1)&d["cover"])):
                if len(checks)>=32:break
                o=d["world"][y,x,:3].astype(float)+[0,0,bias]
                exact=frozen.exact(frozen.analytic_polygons(o,rectangles))
                fast=disk_visibility(*o,rects)
                checks.append(abs(exact-fast))
            assert max(checks,default=0)<2e-6,checks
            cache=a.root/f"{work}-{policy}-analytic.npz"
            if cache.exists():
                cached=np.load(cache);reference,n=cached["ref"],cached["n"]
            else:
                reference,n=film(rects,d["hit"][...,0],d["surface_depth"],bias,jitters())
            valid=d["cover"]&(n>0)&(d["n"]>0)
            e=d["ref"]-reference
            partial=valid&(reference>1e-9)&(reference<1-1e-9)
            rows={}
            for name,m in (("all",valid),("partial",partial)):
                rows[name]=score.summary(e[m],d["sigma"][m])
                rows[name]["within_sigma_F"]=rows[name]["rms"]<=rows[name]["noise_rms"]
            rows["oracle_crosscheck_max"]=max(checks,default=0)
            rows["accepted_count_mismatches"]=int((valid&(d["n"]!=n)).sum())
            rows["accepted_count_max_difference"]=int(np.max(np.abs(d["n"]-n)))
            rows["passed"]=all(v["within_sigma_F"] for k,v in rows.items() if k in ("all","partial"))
            results[f"{work}/{policy}"]=rows
            np.savez_compressed(a.root/f"{work}-{policy}-analytic.npz",ref=reference,n=n)
            panels=[("Analytic film",score.vis(reference,d["cover"])),("GPU film",score.vis(d["ref"],d["cover"])),("GPU - analytic x8",score.error(e,d["cover"],valid))]
            score.sheet(panels,width=512).save(a.root/"images"/f"{work}-{policy}-oracle.png")
            print(work,policy,rows,"elapsed",time.perf_counter()-start,flush=True)
    result=dict(cases=results,passed=all(r["passed"] for r in results.values()),seconds=time.perf_counter()-start)
    suffix="" if a.workload==a.policy=="both" else f"-{a.workload}-{a.policy}"
    (a.root/f"fixture-validation{suffix}.json").write_text(json.dumps(result,indent=2))
    if not result["passed"]:raise SystemExit(1)


if __name__=="__main__":
    main()
