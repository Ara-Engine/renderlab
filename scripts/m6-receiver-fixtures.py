"""R1 continuous disk oracles and frozen sigma_F fixture checks (CPU, two threads)."""
import argparse
import importlib.util
import json
import math
from pathlib import Path
import time

import numpy as np
from numba import njit, prange, set_num_threads

def load(name, filename):
    spec=importlib.util.spec_from_file_location(name,Path(__file__).with_name(filename))
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    return module

f0=load("fixture_f0","m6-film-fixtures.py")
score=f0.score

@njit
def slope_cdf(k,theta,slope):
    if k < -1e30:return 0.
    if k > 1e30:return 1.
    # l=(sin(theta),0,cos(theta)); choose u=(-cos(theta),0,sin(theta)).
    # A rotated uniform disk has the identical continuous measure.
    c=math.cos(theta)-k*math.sin(theta)
    radius=slope*abs(math.sin(theta)+k*math.cos(theta))
    if radius==0:return 1. if c<=0 else 0.
    h=max(-1.,min(1.,-c/radius))
    return .5+(math.asin(h)+h*math.sqrt(max(0.,1-h*h)))/math.pi

@njit
def profile_visibility(ox,oz,profile,theta,slope):
    # Project each infinitely extruded segment onto ray slope; union intervals
    # before integrating their continuous disk CDF. The fixture's +/-8m ends
    # exceed every primary and shadow reach by orders of magnitude.
    intervals=np.empty((len(profile),2));count=0
    for j in range(len(profile)-1):
        x0,z0=profile[j];x1,z1=profile[j+1]
        if x1<=ox:continue
        a=(z1-z0)/(x1-x0)
        if x0<=ox:
            z_at=z0+a*(ox-x0)
            k0=1e35 if z_at>oz else -1e35
            if abs(z_at-oz)<1e-15:k0=a
        else:k0=(z0-oz)/(x0-ox)
        k1=(z1-oz)/(x1-ox)
        lo,hi=min(k0,k1),max(k0,k1)
        pos=count
        while pos>0 and intervals[pos-1,0]>lo:
            intervals[pos]=intervals[pos-1];pos-=1
        intervals[pos,0]=lo;intervals[pos,1]=hi;count+=1
    end=-1e36;blocked=0.
    for j in range(count):
        lo,hi=intervals[j]
        if hi>max(lo,end):blocked+=slope_cdf(hi,theta,slope)-slope_cdf(max(lo,end),theta,slope)
        end=max(end,hi)
    return max(0.,min(1.,1.-blocked))

@njit(parallel=True)
def saw_film(profile,centre_depth,centre_cover,jitters,theta,bias,viewed):
    height,width=centre_depth.shape
    ref=np.ones((height,width));counts=np.zeros((height,width),np.int32)
    scale=2*math.tan(math.radians(55)/2)/height
    for y in prange(height):
        for x in range(width):
            if not centre_cover[y,x]:continue
            total=0.;n=0
            for i in range(256):
                dx=(x+jitters[i,0]-width/2)*scale
                depth=1e30;px=0.;pz=0.;nx=0.;nz=1.
                for j in range(len(profile)-1):
                    x0,z0=profile[j];x1,z1=profile[j+1];a=(z1-z0)/(x1-x0)
                    z=(z0+a*(dx*.15-x0))/(1+a*dx)
                    d=.15-z;xx=dx*d
                    if x0<=xx<=x1 and 0<d<depth:
                        depth=d;px=xx;pz=z;nx=-a/math.sqrt(1+a*a);nz=1/math.sqrt(1+a*a)
                if depth==1e30 or abs(depth-centre_depth[y,x])>.02*centre_depth[y,x]:continue
                sign=1. if viewed or nx*math.sin(theta)+nz*math.cos(theta)>=0 else -1.
                total+=profile_visibility(px+sign*nx*bias,pz+sign*nz*bias,profile,theta,math.tan(.04));n+=1
            if n:ref[y,x]=total/n
            counts[y,x]=n
    return ref,counts

def check(directory,policy):
    d=score.decode(directory,0);work=directory.name.split("-")[0]
    if work in ("flat","convex"):
        valid=d["cover"]&(d["n"]>0)
        false=int((valid&(d["ref"]!=1)).sum())
        score.sheet([("Expected unshadowed",score.vis(np.ones_like(d["ref"]),d["cover"])),("GPU film visibility",score.vis(d["ref"],d["cover"])),("GPU film radiance +4",score.tone(d["hdr"],4))],width=256).save(directory/"oracle.png")
        return dict(passed=false==0,false_shadow_pixels=false,covered=int(valid.sum()),max_error=float(np.max(np.abs(d["ref"][valid]-1))),sigma_F_rms=float(np.sqrt(np.mean(d["sigma"][valid]**2))))
    legacy=None
    if work=="sawtooth":
        g=json.loads((directory/"geometry.json").read_text());profile=np.asarray(g["profile"])
        # Vertex y=+/-8 sets 16 ulp(8) for every fixture facet.
        eps=16*float(np.spacing(np.float32(8)))
        reference,n=saw_film(profile,d["surface_depth"],d["cover"],f0.jitters(),math.radians(g["incidence"]),eps,policy=="viewed")
        old,_=saw_film(profile,d["surface_depth"],d["cover"],f0.jitters(),math.radians(g["incidence"]),.001,False)
        legacy=old-reference
    else:
        rectangles=json.loads((directory/"rectangles.json").read_text())
        rects=np.array([[*r["center"],*r["half"],r["angle"]] for r in rectangles])
        # Ground quad has vertices at +/-8; blockers are inside it and retain 10um.
        # The 5.26um ground-only increase is evaluated rather than reusing F0 blindly.
        reference,n=rect_film(rects,d["hit"][...,0],d["surface_depth"],f0.jitters())
    valid=d["cover"]&(n>0)&(d["n"]>0)
    partial=valid&(reference>1e-9)&(reference<1-1e-9)
    rows={}
    for name,mask in (("all",valid),("partial",partial)):
        rows[name]=score.summary((d["ref"]-reference)[mask],d["sigma"][mask])
        rows[name]["within_sigma_F"]=rows[name].get("rms",0)<=rows[name].get("noise_rms",0)
    rows["accepted_count_mismatches"]=int((d["cover"]&(d["n"]!=n)).sum())
    rows["missing_oracle_samples"]=int((d["cover"]&((n==0)|(d["n"]==0))).sum())
    rows["passed"]=all(rows[k]["within_sigma_F"] for k in ("all","partial")) and rows["missing_oracle_samples"]==0
    if legacy is not None:rows["one_mm_magnitude_vs_oracle"]=score.lb.summary(legacy[valid])
    np.savez_compressed(directory/"analytic.npz",ref=reference,n=n)
    panels=[("Continuous analytic film",score.vis(reference,d["cover"])),("GPU film",score.vis(d["ref"],d["cover"])),("GPU minus analytic x8",score.error(d["ref"]-reference,d["cover"],valid))]
    if legacy is not None:panels.append(("1mm light-side magnitude (continuous)",score.vis(old,d["cover"])))
    score.sheet(panels,width=512).save(directory/"oracle.png")
    return rows

@njit(parallel=True)
def rect_film(rects,hit,depth,jitter):
    h,w=hit.shape;ref=np.ones((h,w));accepted=np.zeros((h,w),np.int32)
    scale=math.tan(math.radians(55)/2)*2/h
    for y in prange(h):
        for x in range(w):
            if hit[y,x]==0:continue
            total=0.;n=0
            for i in range(256):
                ox=(x+jitter[i,0]-w/2)*scale;oy=(h/2-y-jitter[i,1])*scale
                z=0.;instance=1
                for ri in range(len(rects)):
                    cx,cy,rz,hx,hy,angle=rects[ri]
                    if rz<=z:continue
                    px=ox*(4-rz)-cx;py=oy*(4-rz)-cy;c=math.cos(angle);s=math.sin(angle)
                    if abs(c*px+s*py)<=hx and abs(-s*px+c*py)<=hy:z=rz;instance=ri+2
                ds=4-z
                if instance!=hit[y,x] or abs(ds-depth[y,x])>.02*depth[y,x]:continue
                bias=16*float(np.spacing(np.float32(8))) if instance==1 else 1e-5
                total+=f0.disk_visibility(ox*ds,oy*ds,z+bias,rects);n+=1
            if n:ref[y,x]=total/n
            accepted[y,x]=n
    return ref,accepted

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument("directories",type=Path,nargs="*");p.add_argument("--check-oracle",action="store_true");a=p.parse_args()
    set_num_threads(2);start=time.perf_counter()
    if a.check_oracle:
        # Independent f64 ray/segment intersections, not a slope interval union.
        count=262144;i=np.arange(count);radius=np.sqrt((i+.5)/count);az=i*(math.pi*(3-math.sqrt(5)))
        theta=math.radians(85);s=math.tan(.04)
        dx=math.sin(theta)-s*radius*np.cos(az)*math.cos(theta)
        dz=math.cos(theta)+s*radius*np.cos(az)*math.sin(theta)
        profile=np.array([[-.009,0.],[-.005,.0018],[-.001,0.],[.007,.0036],[.015,0.]])
        max_error=0.
        partial_checks=0
        for ox,oz in ((-.006,.0024),(.001,.0030),(.004,.0033),(-.007,.00091),(.012,.00136)):
            blocked=np.zeros(count,bool)
            for (x0,z0),(x1,z1) in zip(profile[:-1],profile[1:]):
                a0=(z1-z0)/(x1-x0);denom=dz-a0*dx
                t=(z0+a0*(ox-x0)-oz)/denom;xx=ox+t*dx
                blocked|=(t>=0)&(xx>=x0)&(xx<=x1)
            numerical=1-blocked.mean();analytic=profile_visibility(ox,oz,profile,theta,s)
            partial_checks+=int(0.01<analytic<.99)
            max_error=max(max_error,abs(numerical-analytic))
        assert max_error<2e-4,max_error
        assert partial_checks>=3,partial_checks
        print("profile oracle vs 262144 independent rays, max error",max_error,"partial cases",partial_checks,flush=True)
        return
    if not a.directories:p.error("directories or --check-oracle required")
    passed=True
    for directory in a.directories:
        policy="viewed" if "viewed" in directory.name else "magnitude"
        result=check(directory,policy)
        (directory/"validation.json").write_text(json.dumps(result,indent=2))
        print(directory.name,json.dumps(result),"elapsed",time.perf_counter()-start,flush=True)
        passed=passed and result["passed"]
    if not passed:raise SystemExit(1)

if __name__=="__main__":main()
