"""Small current-policy CPU ray replay for RT1b, before any GPU experiment.

Uses RT1's current hit records, not F0's displaced receiver. Local triangle
candidates are a screening aid, not an empty-space proof; baseline agreement is
reported per pixel and the retained variant still needs the full GPU image gate.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import time

for key in ("OPENBLAS_NUM_THREADS", "OMP_NUM_THREADS", "NUMBA_NUM_THREADS"):
    os.environ[key] = "2"
import numpy as np
from numba import njit


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


@njit
def trace(origins, directions, triangles, epsilon):
    blocked = 0
    for r in range(len(origins)):
        for k in range(len(triangles)):
            a, b, c = triangles[k, 0], triangles[k, 1], triangles[k, 2]
            ab, ac = b-a, c-a
            h = np.cross(directions[r], ac)
            determinant = np.dot(ab, h)
            if abs(determinant) < 1e-24:
                continue
            s = origins[r]-a
            u = np.dot(s, h)/determinant
            if u < 0 or u > 1:
                continue
            q = np.cross(s, ab)
            v = np.dot(directions[r], q)/determinant
            t = np.dot(ac, q)/determinant
            if v >= 0 and u+v <= 1 and epsilon <= t <= .25:
                blocked += 1
                break
    return 1-blocked/len(origins)


@njit
def closest_camera(eye, directions, triangles):
    ids=np.full(len(directions),-1,dtype=np.int32);bary=np.zeros((len(directions),2))
    for r in range(len(directions)):
        best=1e30
        for k in range(len(triangles)):
            a,b,c=triangles[k,0],triangles[k,1],triangles[k,2]
            ab,ac=b-a,c-a;h=np.cross(directions[r],ac);det=np.dot(ab,h)
            if abs(det)<1e-24:continue
            s=eye-a;u=np.dot(s,h)/det
            if u<0 or u>1:continue
            q=np.cross(s,ab);v=np.dot(directions[r],q)/det;t=np.dot(ac,q)/det
            if v>=0 and u+v<=1 and 0<t<best:
                best=t;ids[r]=k;bary[r,0]=u;bary[r,1]=v
    return ids,bary


def hashed(x):
    x = int(x) & 0xffffffff
    x = ((x ^ (x >> 16))*0x7feb352d) & 0xffffffff
    x = ((x ^ (x >> 15))*0x846ca68b) & 0xffffffff
    return (x ^ (x >> 16)) & 0xffffffff


def shifts(seed):
    return np.array([seed >> 8, hashed(seed ^ 0xa511e9b3) >> 8])/16777216


def source_directions(x, y, count, stratified):
    shift = shifts(hashed(x+y*1920))
    i = np.arange(count)
    if stratified == 2 and count == 8:
        disk = np.c_[(i+shift[0])/8,(i*.6180339887498949+shift[1])%1]
    elif stratified and count == 8:
        disk = np.c_[(i % 2 + shift[0])/2, (i//2 + shift[1])/4]
    else:
        disk = (i[:, None].astype('f4')*np.array([.7548776662466927,.5698402909980532],dtype='f4')+shift.astype('f4')) % 1
    light = np.array([.4,.8,.3]); light /= np.linalg.norm(light)
    u = np.cross(-light, [0,0,1]); u /= np.linalg.norm(u)
    v = np.cross(u, -light)
    angle = 2*np.pi*disk[:, 1]
    directions = light + .04*np.sqrt(disk[:, 0, None])*(np.cos(angle[:,None])*u+np.sin(angle[:,None])*v)
    return directions/np.linalg.norm(directions,axis=1)[:,None]


def receiver(source, hit, inverse, eye, x, y):
    mesh, model = source.placements[int(hit[0])-1]
    _, vertices, indices = source.meshes[mesh][0]
    points = vertices[indices[hit[1]],:3].astype(float)@model[:3,:3].astype(float).T+model[:3,3]
    bary = hit[2:].copy().view('<f4').astype(float)
    weights = np.r_[1-bary.sum(),bary]
    ab, ac = points[1]-points[0],points[2]-points[0]
    n = np.cross(ab,ac); n /= np.linalg.norm(n)
    point = weights@points
    n *= 1 if n@(eye-point)>=0 else -1
    extent = np.float32(np.abs(points).max())
    epsilon = max(1e-5, 16*float(np.nextafter(extent,np.float32(np.inf))-extent))
    ndc = (np.array([x+.5,y+.5])/np.array([1920,1080]))*np.array([2,-2])+[-1,1]
    a = inverse@np.r_[ndc,1,1]; b = inverse@np.r_[ndc,0,1]
    near, far = a[:3]/a[3], b[:3]/b[3]
    step = [2/1920,-2/1080]
    da, db = inverse[:,0]*step[0],inverse[:,1]*step[1]
    dnear = [(q[:3]-near*q[3])/a[3] for q in (da,db)]
    dfar = [(q[:3]-far*q[3])/b[3] for q in (da,db)]
    direction = far-near; denominator = n@direction
    t = n@(point-near)/denominator
    gradients = []
    area = np.cross(ab,ac)@n
    du, dv = np.cross(ac,n)/area,np.cross(n,ab)/area
    for dn, df in zip(dnear,dfar):
        q = dn+t*(df-dn); dp = q-direction*(n@q/denominator)
        u, v = du@dp,dv@dp
        gradients.append(np.array([-u-v,u,v]))
    return point, n, epsilon, points, weights, np.array(gradients)


def origins(x,y,count,point,n,epsilon,points,weights,gradients,mode):
    if mode == 'centre':
        return np.tile(point+epsilon*n,(count,1))
    grid = np.array([4,2]) if count == 8 else np.array([16,16])
    index = (np.arange(count)*5) % count
    shift = shifts(hashed(x+y*1920)^0x68bc21eb)
    samples = (np.c_[index % grid[0],index//grid[0]]+.5)/grid
    samples = (samples+shift) % 1-.5
    delta = samples@gradients
    if mode == 'clipped':
        scale = np.minimum(1, np.min(np.where(delta < 0, weights/np.maximum(-delta,1e-30), np.inf),axis=1))
        delta *= scale[:,None]
    return point+delta@points+epsilon*n


def surface_origins(plane_origins, centre, normal, epsilon, eye, triangles, indices, primitive):
    """Follow the existing mesh adjacency to the facet under each footprint sample.
    Unknown topology or a 16-step miss keeps the centre receiver, never empty space.
    """
    links=np.full((len(indices),3),-1,dtype=np.int32); edges={}
    for i,face in enumerate(indices):
        for edge in range(3):
            key=tuple(sorted((int(face[(edge+1)%3]),int(face[(edge+2)%3]))))
            if key in edges:
                other,side=edges[key];links[i,edge]=other;links[other,side]=i
            else:edges[key]=(i,edge)
    output=[];fallback=0;steps=[];probes=[]
    direction=centre-eye;direction/=np.linalg.norm(direction)
    for origin in plane_origins:
        base=origin-epsilon*normal;idx=primitive;accepted=False
        for step in range(16):
            a,b,c=triangles[idx];ab,ac=b-a,c-a
            h=np.cross(direction,ac);det=ab@h
            if abs(det)<1e-24:break
            s=base-a;u=s@h/det;q=np.cross(s,ab);v=direction@q/det;t=ac@q/det
            bary=np.array([1-u-v,u,v])
            if bary.min()>=-1e-10:
                point=base+t*direction;n=np.cross(ab,ac);n/=np.linalg.norm(n);n*=1 if n@(eye-point)>=0 else -1
                output.append(point+epsilon*n);steps.append(step);probes.append(dict(local_triangle=int(idx),point=point.tolist()));accepted=True;break
            idx=links[idx,int(np.argmin(bary))]
            if idx<0:break
        if not accepted:output.append(centre+epsilon*normal);fallback+=1;probes.append(None)
    return np.array(output),fallback,max(steps,default=0),probes


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--asset',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--limit',type=int,default=0)
    p.add_argument('--captures',type=Path,default=Path('E:/renderlab-work/rt1/current'))
    p.add_argument('--pixels',type=Path,help='explicit pose/x/y witness JSON')
    p.add_argument('--rays',type=int,choices=(8,256),default=8)
    p.add_argument('--proof',action='store_true',help='one independent closest camera intersection per witness')
    p.add_argument('--camera-replay',action='store_true',help='local closest primary and original 2 percent depth conditioning at every origin')
    a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
    start=time.perf_counter()
    loader=module('rt1_source',Path(__file__).with_name('quarry-shadow-probe.py'))
    source=loader.Source(a.asset)
    rt1=Path('E:/renderlab-work/rt1')
    selected=json.loads((rt1/'final-replay/replay.json').read_text())['selected_point_reference']
    bounds=[]
    for mesh,model in source.placements:
        _,vertices,_=source.meshes[mesh][0]
        lo,hi=vertices[:,:3].min(0),vertices[:,:3].max(0)
        corners=np.array([[lo[0] if i&1==0 else hi[0],lo[1] if i&2==0 else hi[1],lo[2] if i&4==0 else hi[2]] for i in range(8)])
        world=corners@model[:3,:3].T+model[:3,3]
        bounds.append((world.min(0),world.max(0),np.linalg.inv(model.astype(float))))
    result=[]
    for pose in (0,1):
        label='baseline' if (a.captures/f'pose-{pose}-baseline.hit.u32').exists() else 'interactive'
        hits=np.memmap(a.captures/f'pose-{pose}-{label}.hit.u32',mode='r',dtype='<u4',shape=(1080,1920,4))
        counts=np.memmap(a.captures/f'pose-{pose}-{label}.u32',mode='r',dtype='<u4',shape=(1080,1920))
        info=json.loads(Path(f'D:/Coding/renderlab-work/m6-exact-quality/f0/quarry-policy-p{pose}/pose-{pose}.json').read_text())
        inverse=np.array(info['camera_inverse']).reshape(4,4).T;eye=np.array(info['eye'][:3])
        witnesses = ([r for r in json.loads(a.pixels.read_text()) if r['pose']==pose] if a.pixels else selected[str(pose)]['pixels'])[:a.limit or None]
        if not a.limit and not a.pixels:
            extra = [(1002,491),(910,635),(1212,653),(974,640)] if pose == 0 else [(1169,384)]
            known = {(r['x'],r['y']) for r in witnesses}
            witnesses += [dict(x=x,y=y) for x,y in extra if (x,y) not in known]
        for row in witnesses:
            x,y=row['x'],row['y']
            rec=receiver(source,hits[y,x],inverse,eye,x,y)
            point,n,epsilon,points,weights,gradients=rec
            extent=np.abs(gradients@points).sum(axis=0)*.5+.002
            lo=point-extent;hi=point+extent+np.array([.425,.85,.32])*.25
            triangles=[];triangle_slots=[];triangle_primitives=[];root_mesh=None
            for slot,(mesh,model) in enumerate(source.placements):
                lower,upper,inv=bounds[slot]
                if np.any(upper<lo) or np.any(lower>hi):continue
                corners=np.array([[lo[0] if i&1==0 else hi[0],lo[1] if i&2==0 else hi[1],lo[2] if i&4==0 else hi[2]] for i in range(8)])
                local=corners@inv[:3,:3].T+inv[:3,3]
                lmin,lmax=local.min(0)-.1,local.max(0)+.1
                _,vertices,indices=source.meshes[mesh][0]
                near=np.all((vertices[:,:3]>=lmin)&(vertices[:,:3]<=lmax),axis=1)
                pick=indices[near[indices].any(axis=1)]
                world_triangles=vertices[pick,:3].astype(float)@model[:3,:3].astype(float).T+model[:3,3]
                triangles.append(world_triangles)
                triangle_slots.extend([slot]*len(world_triangles))
                triangle_primitives.extend(np.flatnonzero(near[indices].any(axis=1)).tolist())
                if slot==int(hits[y,x,0])-1:
                    original=np.flatnonzero(near[indices].any(axis=1))
                    root_mesh=(world_triangles,pick,int(np.searchsorted(original,int(hits[y,x,1]))),original)
            triangles=np.ascontiguousarray(np.concatenate(triangles))
            values={}
            for mode,stratified in [('centre',False),('clipped',False),('plane',False),('plane',True),('clipped',True)]:
                rays=source_directions(x,y,a.rays,stratified)
                origin=origins(x,y,a.rays,*rec,mode)
                values[mode+('-stratified' if stratified else '')]=float(trace(origin,rays,triangles,epsilon))
            origin=origins(x,y,a.rays,*rec,'plane')
            surface, fallback, walk_steps, probes=surface_origins(origin,point,n,epsilon,eye,*root_mesh[:3])
            for stratified, suffix in ((False,''),(True,'-stratified'),(2,'-sunflower')):
                values['surface'+suffix]=float(trace(surface,source_directions(x,y,a.rays,stratified),triangles,epsilon))
            record=dict(pose=pose,x=x,y=y,current_failure='point_visibility' in row,production=float(counts[y,x]/256),epsilon=epsilon,
                        viewed_ndl=float(n@np.array([.4,.8,.3])/np.linalg.norm([.4,.8,.3])),thin=False,
                        triangles=len(triangles),surface_fallbacks=fallback,surface_max_steps=walk_steps,values=values)
            if a.camera_replay:
                targets=origin-epsilon*n;view=targets-eye;view/=np.linalg.norm(view,axis=1)[:,None]
                identities,bary=closest_camera(eye,view,triangles)
                actual=[];accepted_indices=[];other=0;wrong_depth=0;different=0;gaps=[]
                forward=np.array(info['forward'][:3]);centre_depth=(point-eye)@forward
                for i,identity in enumerate(identities):
                    if identity<0:continue
                    if probes[i] is not None:
                        walked=int(root_mesh[3][probes[i]['local_triangle']])
                        different+=triangle_slots[identity]!=int(hits[y,x,0])-1 or triangle_primitives[identity]!=walked
                        gp=triangles[identity];u0,v0=bary[i];nearest=np.array([1-u0-v0,u0,v0])@gp
                        gaps.append(float(np.linalg.norm(np.array(probes[i]['point'])-nearest)))
                    if triangle_slots[identity]!=int(hits[y,x,0])-1:other+=1;continue
                    tri=triangles[identity];u,v=bary[i];pnt=np.array([1-u-v,u,v])@tri
                    if abs((pnt-eye)@forward-centre_depth)>.02*centre_depth:wrong_depth+=1;continue
                    gn=np.cross(tri[1]-tri[0],tri[2]-tri[0]);gn/=np.linalg.norm(gn);gn*=1 if gn@(eye-pnt)>=0 else -1
                    actual.append(pnt+epsilon*gn);accepted_indices.append(i)
                estimates={};fallback_estimates={}
                for pattern,name in ((False,'r2'),(True,'stratified'),(2,'sunflower')):
                    directions=source_directions(x,y,a.rays,pattern)
                    estimates[name]=float(trace(np.array(actual),directions[accepted_indices],triangles,epsilon)) if actual else None
                    # A production estimator cannot silently classify an unknown
                    # primary sample as lit. Also screen the conservative centre
                    # fallback against the same fixed eight-ray budget.
                    conservative=np.tile(point+epsilon*n,(a.rays,1))
                    conservative[accepted_indices]=np.array(actual).reshape((-1,3))
                    fallback_estimates[name]=float(trace(conservative,directions,triangles,epsilon))
                record['camera_replay']=dict(accepted=len(actual),other_placement=other,depth_rejected=wrong_depth,
                    different_triangles=int(different),max_point_gap=max(gaps,default=0),gaps_over_40um=sum(g>4e-5 for g in gaps),
                    visibility=estimates['r2'],estimates=estimates,centre_fallback_estimates=fallback_estimates,
                    scope='local candidate geometry; not an empty-space or full-scene visibility certificate')
            if a.proof:
                sample=next((i for i,p in enumerate(probes) if p is not None and p['local_triangle']!=root_mesh[2]),None)
                if sample is not None:
                    target=origin[sample]-epsilon*n;direction=target-eye;direction/=np.linalg.norm(direction)
                    distance,identity,_=source.intersect(eye,direction)
                    observed=probes[sample];walked_primitive=int(root_mesh[3][observed['local_triangle']])
                    record['camera_proof']=dict(sample=sample,direction=direction.tolist(),eye=eye.tolist(),closest=identity,
                        closest_distance=distance,walked_primitive=walked_primitive,walked_point=observed['point'],
                        walked_distance=float(np.linalg.norm(np.array(observed['point'])-eye)),origin_target=target.tolist())
            result.append(record)
            (a.output/'pixels.json').write_text(json.dumps(result,indent=2))
    summary={mode:sum(r['values'][mode]<=.9 for r in result) for mode in result[0]['values']}
    if a.camera_replay:
        for policy in ('estimates','centre_fallback_estimates'):
            summary['camera_'+policy]={name:sum(r['camera_replay'][policy][name] is not None and r['camera_replay'][policy][name]<=.9 for r in result) for name in ('r2','stratified','sunflower')}
    summary.update(pixels=len(result),baseline_matches=sum(r['production']==r['values']['centre'] for r in result),seconds=time.perf_counter()-start)
    (a.output/'summary.json').write_text(json.dumps(summary,indent=2));print(summary)


if __name__=='__main__':main()
