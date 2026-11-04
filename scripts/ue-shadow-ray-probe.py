"""Small CPU source-geometry probes for saved quarry shadow disagreements.

This is an independent source-triangle diagnostic, not an Unreal emulator or a
new full-image oracle. Cone-cull once, retain private triangles, then vary one
ray parameter at a time. No GPU, process launch, image capture, or new dependency.
"""
import argparse
import importlib.util
import json
import math
from pathlib import Path
import time

import numpy as np
from numba import njit


def module(path):
    spec=importlib.util.spec_from_file_location(path.stem.replace('-','_'),path)
    m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
    return m


def unit(v):
    return v/np.linalg.norm(v)


@njit
def trace(tris, origins, directions, tmins, tmaxs, epsilon, normal_cull, normals):
    output=np.ones(len(directions))
    nearest=np.full(len(directions),np.inf)
    for i in range(len(directions)):
        dx,dy,dz=directions[i];ox,oy,oz=origins[i]
        if normal_cull and normals[i,0]*dx+normals[i,1]*dy+normals[i,2]*dz<=0:
            output[i]=0
            continue
        for k in range(len(tris)):
            ax,ay,az=tris[k,0]
            ex,ey,ez=tris[k,1]-tris[k,0]
            fx,fy,fz=tris[k,2]-tris[k,0]
            hx=dy*fz-dz*fy;hy=dz*fx-dx*fz;hz=dx*fy-dy*fx
            det=ex*hx+ey*hy+ez*hz
            if abs(det)<1e-12:continue
            sx=ox-ax;sy=oy-ay;sz=oz-az
            u=(sx*hx+sy*hy+sz*hz)/det
            if u<0 or u>1:continue
            qx=sy*ez-sz*ey;qy=sz*ex-sx*ez;qz=sx*ey-sy*ex
            v=(dx*qx+dy*qy+dz*qz)/det
            if v<0 or u+v>1:continue
            t=(fx*qx+fy*qy+fz*qz)/det
            if t<tmins[i] or t>tmaxs[i]:continue
            # UE epsilon ray culls backfaces before resuming two-sided tracing.
            if t<epsilon and det<=0:continue
            output[i]=0
            if t<nearest[i]:nearest[i]=t
    return output,nearest


def prepare(args):
    probe=module(Path(__file__).with_name('quarry-shadow-probe.py'))
    src=probe.Source(args.asset)
    args.output.mkdir(parents=True,exist_ok=False)
    root=args.reference
    ref=np.load(root/'pose-0-reference.npz')['ref']
    world=np.fromfile(root/'pose-0-world.f32','<f4').reshape(1080,1920,4)[...,:3]
    geom=np.fromfile(root/'pose-0-normal.f32','<f4').reshape(1080,1920,4)[...,:3]
    camera=json.loads(args.camera.read_text())['frames'][0]
    light=unit(np.array([.4,.8,.3]));u=unit(np.cross(-light,[0,0,1]));v=np.cross(u,-light)
    basis=np.stack([u,v,light],axis=-1)
    pixels=[(1203,669),(928,325),(1326,607),(1269,328),(987,477),(1388,529),
            (945,276),(934,292),(1110,669),(1577,595),(1254,549),(1082,359)]
    receivers=np.array([world[y,x] for x,y in pixels]);rp=receivers@basis
    # Guard is larger than every tested offset. Cone contains all .08-radian
    # disks and +/-1-degree direction probes. It only prunes impossible hits.
    kept=[[] for _ in pixels];primary=[[] for _ in pixels]
    start=time.monotonic()
    for mesh,model in src.placements:
        for primitive,(_,vertices,indices) in enumerate(src.meshes[mesh]):
            for first in range(0,len(indices),32768):
                ids=indices[first:first+32768]
                tris=(vertices[ids,:3]@model[:3,:3].T+model[:3,3]).astype('f8')
                projected=tris@basis
                lo=projected.min(1);hi=projected.max(1)
                for i in range(len(pixels)):
                    rel_lo=lo-rp[i];rel_hi=hi-rp[i]
                    radius=.11*np.maximum(rel_hi[:,2],0)+.025
                    mask=(rel_hi[:,2]>=-.025)&np.all(rel_lo[:,:2]<=radius[:,None],1)&np.all(rel_hi[:,:2]>=-radius[:,None],1)
                    if mask.any():kept[i].append(tris[mask])
                    near=np.all(tris.min(1)<receivers[i]+.002,1)&np.all(tris.max(1)>receivers[i]-.002,1)
                    for idx in np.flatnonzero(near):
                        tri=tris[idx];eye=np.array(camera['eye']);d=unit(receivers[i]-eye)
                        e1,e2=tri[1]-tri[0],tri[2]-tri[0];h=np.cross(d,e2);det=e1@h
                        if abs(det)<1e-12:continue
                        ss=eye-tri[0];bu=(ss@h)/det; q=np.cross(ss,e1);bv=(d@q)/det;bt=(e2@q)/det
                        if bu<0 or bv<0 or bu+bv>1:continue
                        hit=eye+d*bt;distance=np.linalg.norm(hit-receivers[i])
                        vn=vertices[ids[idx],3:6].astype('f8')@np.linalg.inv(model[:3,:3])
                        ns=unit(vn[0]*(1-bu-bv)+vn[1]*bu+vn[2]*bv)
                        if ns@(eye-hit)<0:ns=-ns
                        primary[i].append(dict(distance=distance,hit=hit.tolist(),shading_normal=ns.tolist(),
                            geometry_normal=unit(np.cross(e1,e2)).tolist(),triangle=[mesh,primitive,first+int(idx)]))
    bounds=src.bounds()
    eye=np.array(camera['eye']);f=unit(np.array(camera['target'])-eye);right=unit(np.cross(f,camera['up']));up=np.cross(right,f)
    view=np.eye(4);view[:3,:3]=np.stack([right,up,-f]);view[:3,3]=-view[:3,:3]@eye
    views=probe.replay_views(dict(camera,width=1920,height=1080),view,-light,4096,bounds)
    records=[]
    for i,(x,y) in enumerate(pixels):
        tris=np.concatenate(kept[i]);np.save(args.output/f'triangles-{x}-{y}.npy',tris)
        near=min(primary[i],key=lambda p:p['distance'])
        depth=(receivers[i]-eye)@f
        layer=next(z for z in views if z['near']<=depth<=z['far'])
        records.append(dict(pixel=[x,y],world=receivers[i].tolist(),geometric=geom[y,x].tolist(),
            reference=float(ref[y,x]),primary=near,triangle_count=len(tris),view=layer))
    data=dict(records=records,eye=eye.tolist(),light=light.tolist(),u=u.tolist(),v=v.tolist(),
              asset=str(args.asset),preparation_seconds=time.monotonic()-start)
    (args.output/'receivers.json').write_text(json.dumps(data,indent=2)+'\n')


def run(args):
    data=json.loads((args.output/'receivers.json').read_text());rows=[]
    light=np.array(data['light']);u=np.array(data['u']);v=np.array(data['v']);eye=np.array(data['eye'])
    camera=json.loads(args.camera.read_text())['frames'][0]
    forward=unit(np.array(camera['target'])-eye)
    right=unit(np.cross(forward,camera['up']));up=np.cross(right,forward)
    ty=math.tan(camera['fov_y_radians']/2);tx=ty*1920/1080
    worlds=np.fromfile(args.reference/'pose-0-world.f32','<f4').reshape(1080,1920,4)[...,:3]
    cover=np.load(args.reference/'pose-0-reference.npz')['cover']
    zmap=(worlds-eye)@forward
    device_depth=np.divide(.005,zmap,out=np.zeros_like(zmap),where=cover)
    def screen_skip(origin,length):
        def project(p):
            rel=p-eye;z=rel@forward
            return np.array([((rel@right)/(z*tx)+1)*960, (1-(rel@up)/(z*ty))*540,.005/z])
        a=project(origin);b=project(origin+light*length)
        def read(p):
            x,y=np.floor(p[:2]).astype(int)
            return device_depth[np.clip(y,0,1079),np.clip(x,0,1919)]
        initial=read(a)
        for t in [.25,.5,.75,1]:
            sample=a+(b-a)*t;depth=read(sample)
            if depth!=initial and sample[2]<depth:return length*max(0,t-.375)
        return length
    n=args.rays
    # Deterministic stratified slope disk. These are not production seed bits.
    q=np.arange(n);r=(q+.5)/n;theta=2*np.pi*np.mod(q*.6180339887498949,1)
    disk=np.sqrt(r)[:,None]*(np.cos(theta)[:,None]*u+np.sin(theta)[:,None]*v)
    variants=[('rl',.001,'geom',True,.04,0,False,0,0)]
    variants += [(f'normal-{bias}',bias,'geom',True,.04,0,False,0,0) for bias in [0,.0005,.002,.005,.01]]
    variants += [('no-tmin',.001,'geom',False,.04,0,False,0,0)]
    variants += [(f'angle-{a}',.001,'geom',True,a,0,False,0,0) for a in [.02,.08]]
    variants += [('smooth',.001,'smooth',True,.04,0,False,0,0),
                 ('ue-origin',.001,'ue',False,.04,0,False,0,0),
                 ('ue-cull',.001,'ue',False,.04,0,True,0,0),
                 ('ue-epsilon',.001,'ue',False,.04,.01,True,0,0)]
    variants += [('vsm-normal',0,'vsm',False,.04,0,False,0,0),
                 ('vsm-screen-midpoint',0,'vsm-screen',False,.04,0,False,0,0)]
    variants += [(f'dir-{axis}-{sign}',.001,'geom',True,.04,0,False,sign*math.pi/180 if axis==0 else 0,
                  sign*math.pi/180 if axis==1 else 0) for axis in [0,1] for sign in [-1,1]]
    for rec in data['records']:
        x,y=rec['pixel'];tris=np.load(args.output/f'triangles-{x}-{y}.npy')
        world=np.array(rec['world']);ng=np.array(rec['geometric']);ng=ng if ng@light>=0 else -ng
        ns=np.array(rec['primary']['shading_normal']);matrix=np.array(rec['view']['matrix'])
        row=dict(pixel=[x,y],reference=rec['reference'],geometric_nl=float(ng@light),shading_nl=float(ns@light),
                 receiver_error_mm=1000*rec['primary']['distance'],variants=[])
        vsm_bias=max(.0002,.0005*np.linalg.norm(world-eye)*tx)
        vsm_screen_length=.015*((world-eye)@forward)*ty
        vsm_skip=screen_skip(world+ns*vsm_bias,vsm_screen_length)
        row.update(vsm_default_normal_bias_m=vsm_bias,vsm_default_screen_length_m=vsm_screen_length,
                   source_depth_screen_skip_m=vsm_skip)
        for name,bias,kind,interval,angle,epsilon,cull,du,dv in variants:
            l=unit(light+du*u+dv*v)
            dirs=l+math.tan(angle)*disk;dirs/=np.linalg.norm(dirs,axis=-1)[:,None]
            normals=np.tile(ns if kind in ('smooth','ue','vsm','vsm-screen') else ng,(n,1))
            if kind=='ue':
                raybias=max(np.linalg.norm(world-eye)*1e-4,bias)
                nb=raybias+(0.0001-raybias)*np.clip(dirs@ns,0,1)
                origin=world+unit(eye-world)*raybias+nb[:,None]*ns
            elif kind in ('vsm','vsm-screen'):
                origin=np.tile(world+ns*vsm_bias,(n,1))
                if kind=='vsm-screen':origin+=dirs*vsm_skip
            else:origin=np.tile(world,(n,1))+normals*bias
            rate=-(dirs@matrix[2,:3]);projected=origin@matrix[:3,:3].T+matrix[:3,3]
            tmin=.00002/rate if interval else np.zeros(n)
            tmax=projected[:,2]/rate if interval else np.full(n,100)
            if interval:
                for axis in range(2):
                    delta=dirs@matrix[axis,:3]
                    cap=np.where(delta>0,(1-projected[:,axis])/delta,(-1-projected[:,axis])/delta)
                    tmax=np.minimum(tmax,cap)
            vis,hit=trace(tris,origin,dirs,tmin,tmax,epsilon,cull,normals)
            finite=hit[np.isfinite(hit)]
            row['variants'].append(dict(name=name,visibility=float(vis.mean()),
                hit_distance_quantiles=np.quantile(finite,[0,.1,.5,.9,1]).tolist() if finite.size else [],
                tmin_median=float(np.median(tmin))))
        rows.append(row)
        print(json.dumps(row),flush=True)
    (args.output/'rays.json').write_text(json.dumps(dict(rays=n,records=rows),indent=2)+'\n')


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--asset',type=Path,default=Path('D:/Coding/projects/desktop/renderlab/measurements/quarry-quality/detail/full.rla'))
    p.add_argument('--reference',type=Path,default=Path('D:/Coding/renderlab-work/m6-adaptive/gpu-phase-quarry/quarry'))
    p.add_argument('--camera',type=Path,default=Path('D:/Coding/renderlab-work/m6-hardware-msaa/quarry-cameras.json'))
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--prepare',action='store_true')
    p.add_argument('--rays',type=int,default=1024)
    a=p.parse_args()
    if a.prepare:prepare(a)
    run(a)
