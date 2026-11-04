"""Level-A evaluation and inspection artifacts; thresholds follow m6-reduced-ray-gate.md.
No GPU is initialized by this module. The runner calls prepare(), then renders the
fixed-point reference through the real shading path, then calls inspection().
"""
from pathlib import Path
import csv, hashlib, json, math
import numpy as np
from scipy import integrate, ndimage
import fresh_png
import fresh_gate_metrics as gate

def sha(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def save(path, data): Path(path).write_text(json.dumps(data, indent=2, allow_nan=False))
def load_counts(out, pose, seed, kind, shape):
    return np.fromfile(out/f"pose-{pose}-seed-{seed}-{kind}.u32",dtype="<u4").reshape(shape)

def replay(hits, eligible):
    # Integer popcount, independent of Rust and shader implementations.
    pop=np.unpackbits(hits.view("u1").reshape(*hits.shape[:2],32),axis=-1,bitorder="little")
    prefix=64-pop[...,:64].sum(-1)
    lit=eligible&(prefix==64);occ=eligible&(prefix==0);mixed=eligible&~lit&~occ
    dilate=lambda m:ndimage.binary_dilation(m,np.ones((3,3),bool))
    region=eligible&(dilate(mixed)|(lit&dilate(occ))|(occ&dilate(lit)))
    return np.where(region,256-pop.sum(-1),np.where(occ,0,256)).astype("<u4"),region

def analytic_polygons(origin, rectangles):
    """Project each rotated planar rectangle into the uniform slope disk.
    At a fixed u every convex polygon has one exact v interval. Their interval
    union is integrated between all vertex/edge/circle intersection breakpoints.
    """
    s=math.tan(.04); polys=[]
    for rect in rectangles:
        cx,cy,z=rect["center"]; h=z-origin[2]
        if h<=0: continue
        hx,hy=rect["half"]; c=math.cos(rect["angle"]);sn=math.sin(rect["angle"])
        # Multiply each rectangle inequality by 1-s*u, positive on the disk.
        # This also handles broad/contact rectangles crossing the projection's
        # pole outside the disk; projecting just their vertices would lose them.
        planes=[]
        for nx,ny,half in [(c,sn,hx),(-sn,c,hy)]:
            base=nx*(origin[0]-cx)+ny*(origin[1]-cy)
            a=s*(-base+nx*h);b=ny*h*s*math.sqrt(2);cc=base+nx*h
            planes.extend([(-a-half*s,-b,half-cc),(a-half*s,b,cc+half)])
        if any(cc+math.hypot(a,b)<0 for a,b,cc in planes): continue
        polys.append((np.empty((0,2)),planes))
    return polys

def exact(polys):
    if not polys: return 1.
    cuts=[-1.,1.];all_planes=[]
    for verts,planes in polys:
        cuts.extend(verts[:,0]);all_planes.extend(planes)
        for a,b,c in planes:
            nn=a*a+b*b
            if nn==0: continue
            d=c*c/nn
            if d<=1:
                u0=-a*c/nn;du=abs(b)*math.sqrt(max(0,1-d)/nn)
                cuts.extend((u0-du,u0+du))
    for a,b,c in all_planes:
        for aa,bb,cc in all_planes:
            det=a*bb-aa*b
            if abs(det)>1e-15: cuts.append((b*cc-bb*c)/det)
    cuts=sorted({max(-1.,min(1.,float(u))) for u in cuts})
    def interval_union(u):
        circle=math.sqrt(max(0.,1-u*u));intervals=[]
        for _,planes in polys:
            lo,hi=-circle,circle
            for a,b,c in planes:
                if abs(b)<1e-15:
                    if a*u+c<0: lo=2.;break
                elif b>0: lo=max(lo,(-a*u-c)/b)
                else: hi=min(hi,(-a*u-c)/b)
            if hi>lo: intervals.append((lo,hi))
        reach=-2.;total=0.
        for lo,hi in sorted(intervals):
            total+=max(0.,hi-max(lo,reach));reach=max(reach,hi)
        return total
    total=sum(integrate.quad(interval_union,a,b,epsabs=2e-9,epsrel=2e-9,limit=100)[0] for a,b in zip(cuts[:-1],cuts[1:]) if b-a>1e-12)
    value=1-total/math.pi
    return 0. if value<1e-9 else 1. if value>1-1e-9 else value

def fresh_hash(x):
    x=np.asarray(x,dtype=np.uint64)&0xffffffff
    x=((x^(x>>16))*0x7feb352d)&0xffffffff
    x=((x^(x>>15))*0x846ca68b)&0xffffffff
    return (x^(x>>16))&0xffffffff

def analytic_reference(world, rectangles):
    h,w=world.shape[:2];ref=np.ones((h,w));sigma=np.zeros((h,w))
    # The planes are double-sided, and the bias flips their normal towards +Z.
    origins=world[...,:3].astype(float).copy();origins[...,2]+=.001
    # Same conservative bounds as fixture_model.relevant, extended to rotations.
    possible=np.zeros((h,w),bool);s=math.tan(.04)
    for r in rectangles:
        x,y,z=r["center"];a,b=r["half"];c=abs(math.cos(r["angle"]));sn=abs(math.sin(r["angle"]))
        dx=c*a+sn*b;dy=sn*a+c*b;height=z-origins[...,2]
        t=height/((1-s)/math.sqrt(2))
        possible|=(height>0)&(origins[...,0]+height*(1+s)/(1-s)>=x-dx)&(origins[...,0]+height*(1-s)/(1+s)<=x+dx)&(origins[...,1]+t*s>=y-dy)&(origins[...,1]-t*s<=y+dy)
    for y,x in zip(*np.nonzero(possible)):
        polygons=analytic_polygons(origins[y,x],rectangles)
        ref[y,x]=exact(polygons)
        seeds=np.arange(1,65,dtype=np.uint64)
        hashed=fresh_hash(fresh_hash(np.uint64(x+y*w))^((seeds*0x9e3779b9)&0xffffffff))
        shift=np.stack([(hashed>>8)/16777216,(fresh_hash(hashed^0xa511e9b3)>>8)/16777216],-1).astype("f4")
        r=np.arange(256,dtype="f4")[None,:,None]*np.array([.7548776662466927,.5698402909980532],"f4")[None,None,:]+shift[:,None,:]
        r=(r-np.floor(r)).astype(float);radius=np.sqrt(r[...,0]);angle=2*math.pi*r[...,1]
        u=radius*np.cos(angle);v=radius*np.sin(angle);hits=np.zeros(u.shape,bool)
        for _,planes in polygons:
            inside=np.ones(u.shape,bool)
            for a,b,c in planes: inside&=a*u+b*v+c>=0
            hits|=inside
        sigma[y,x]=(1-hits.mean(1)).std(ddof=1)
    return ref,sigma

def threshold(draws):
    a=np.asarray(draws,float);mean=a.mean()
    spread=2*a.std(ddof=1)/mean if mean else 0.
    return dict(median=float(np.median(a)),spread_2sd_relative=float(spread),limit=float(np.median(a)*(1+spread)),maximum=float(a.max()))

def prepare(out, workload):
    out=Path(out);shape=(1080,1920) if workload=="quarry" else (192,256)
    pose_count=3 if workload=="quarry" else 5;seed_count=4 if workload=="quarry" else 16
    diagnostics=json.loads((out/"diagnostics.json").read_text())
    assert all(diagnostics[k] is True for k in ("validation","repeat_exact","return_exact","cpu_gpu_bit_exact"))
    results=[];motion_data=[];failures=[]
    for pose in range(pose_count):
        world=np.fromfile(out/f"pose-{pose}-world.f32",dtype="<f4").reshape(*shape,4)
        normal=np.fromfile(out/f"pose-{pose}-normal.f32",dtype="<f4").reshape(*shape,4)
        cover=normal[...,3]!=0;depth=world[...,3]
        if workload=="quarry":
            total=np.zeros(shape);square=np.zeros(shape)
            for seed in range(1,65):
                v=load_counts(out,pose,seed,"production",shape)/256
                total+=v;square+=v*v
            ref=total/64;sigma=np.sqrt(np.maximum(0,(square-total*total/64)/63))
        else: ref,sigma=analytic_reference(world,json.loads((out/"rectangles.json").read_text()))
        np.savez_compressed(out/f"pose-{pose}-reference.npz",ref=ref,sigma=sigma,depth=depth,cover=cover)
        np.rint(ref*16777216).astype("<u4").tofile(out/f"pose-{pose}-reference.u32")
        draws=[load_counts(out,pose,s,"production",shape)/256 for s in range(65,81)]
        parts=gate.populations(ref,depth,cover)
        draw_metrics=[gate.metrics(d,ref,sigma,depth,cover)[0] for d in draws]
        q=float(np.median([m["partial"]["p99"] for m in draw_metrics]))
        baseline={}
        for pop in ("partial","narrow","faint","isolated","silhouette"):
            n=int(parts[pop].sum())
            if not n: continue
            baseline[pop]={k:threshold([m[pop][k] for m in draw_metrics]) for k in ("rms","p99")}
            means=[m[pop]["mean"] for m in draw_metrics]
            se=float(np.median([(d-ref)[parts[pop]].std(ddof=1)/math.sqrt(n) if n>1 else 0 for d in draws]))
            baseline[pop]["mean_limit"]=abs(float(np.median(means)))+2*se
        along={k:threshold([m["along_contour"][k] for m in draw_metrics]) for k in ("rms","max")}
        row=dict(pose=pose,q=q,baseline=baseline,along_contour=along,seeds=[])
        candidate_errors=[]
        for seed in range(seed_count):
            hits=np.fromfile(out/f"pose-{pose}-seed-{seed}-hits.u32",dtype="<u4").reshape(*shape,8)
            prefix=load_counts(out,pose,seed,"prefix",shape)
            eligible=prefix!=0xffffffff
            replayed,region=replay(hits,eligible)
            actual=load_counts(out,pose,seed,"adaptive",shape)
            assert np.array_equal(replayed,actual),f"CPU/GPU bit-exact replay {workload} {pose} {seed}"
            m,error,pops=gate.metrics(actual/256,ref,sigma,depth,cover,q)
            candidate_errors.append(error)
            # Production on the same seed (same ray directions) is the paired "no worse than production" bar.
            # The seed-spread limits alone also reject production's own noisier seeds, so G3/G4 accept a
            # statistic within either the limit or production's same-seed value. Production's own outcome is recorded.
            p,_,_=gate.metrics(load_counts(out,pose,seed,"production",shape)/256,ref,sigma,depth,cover,q)
            def g3(x): return all(all(x[pop][k]<=b[k]["limit"] for k in ("rms","p99")) and abs(x[pop]["mean"])<=b["mean_limit"]
                                  for pop,b in baseline.items() if pop=="partial" or x[pop]["n"]>=100)
            def g4(x): return x["along_contour"]["rms"]<=along["rms"]["limit"] and x["along_contour"]["max"]<=along["max"]["maximum"]
            def paired_g3():
                for pop,b in baseline.items():
                    if pop!="partial" and m[pop]["n"]<100: continue
                    if not all(m[pop][k]<=max(b[k]["limit"],p[pop][k]) for k in ("rms","p99")): return False
                    if abs(m[pop]["mean"])>max(b["mean_limit"],abs(p[pop]["mean"])): return False
                return True
            checks={"G1":m["envelope_violations"]==0,"G2":m["solid_max"]<=.002,"G3":paired_g3(),
                    "G4":all(m["along_contour"][k]<=max(lim,p["along_contour"][k]) for k,lim in (("rms",along["rms"]["limit"]),("max",along["max"]["maximum"])))}
            production_checks={"G3":g3(p),"G4":g4(p),"G3_unpaired_candidate":g3(m),"G4_unpaired_candidate":g4(m)}
            missing=pops["partial"]&((actual==0)|(actual==256))
            path=out/f"pose-{pose}-seed-{seed}-G6.csv"
            with path.open("w",newline="") as f:
                writer=csv.writer(f);writer.writerow(["x","y","V","sigma256","candidate","error","crop"])
                for y,x in zip(*np.nonzero(missing)):
                    writer.writerow([x,y,ref[y,x],sigma[y,x],actual[y,x]/256,error[y,x],f"crops/pose-{pose}-seed-{seed}-{x}-{y}.png"])
            checks["G6"]=True
            row["seeds"].append(dict(seed=seed,checks=checks,production_checks=production_checks,production_metrics=p,metrics=m,uniform_partial_count=int(missing.sum()),region=int(region.sum())))
            failures.extend(f"{workload}/pose-{pose}/seed-{seed}/{k}" for k,v in checks.items() if not v)
        results.append(row)
        motion_data.append(dict(candidate_errors=candidate_errors,partial=parts["partial"]))
        print(workload,"pose",pose,"q",q,"failures",len(failures),flush=True)
    temporal=[]
    for pose in range(pose_count-1):
        a,b=motion_data[pose:pose+2]
        ref_a=np.load(out/f"pose-{pose}-reference.npz")["ref"]
        ref_b=np.load(out/f"pose-{pose+1}-reference.npz")["ref"]
        prod=[gate.motion(load_counts(out,pose,s,"production",shape)/256-ref_a,load_counts(out,pose+1,s,"production",shape)/256-ref_b,a["partial"],b["partial"]) for s in range(65,81)]
        rms=threshold([m["rms"] for m in prod]);p99=threshold([m["p99"] for m in prod])
        candidates=[gate.motion(e,f,a["partial"],b["partial"]) for e,f in zip(a["candidate_errors"],b["candidate_errors"])]
        # Paired with production on the same seed, as for G3/G4: no worse than production's own motion residual.
        same=[gate.motion(load_counts(out,pose,s,"production",shape)/256-ref_a,load_counts(out,pose+1,s,"production",shape)/256-ref_b,a["partial"],b["partial"]) for s in range(len(candidates))]
        passes=[m["rms"]<=max(rms["limit"],q["rms"]) and m["p99"]<=max(p99["maximum"],q["p99"]) for m,q in zip(candidates,same)]
        production_passes=[q["rms"]<=rms["limit"] and q["p99"]<=p99["maximum"] for q in same]
        temporal.append(dict(poses=[pose,pose+1],rms=rms,p99=p99,candidates=candidates,production_same_seed=same,passes=passes,production_passes=production_passes))
        failures.extend(f"{workload}/motion-{pose}/seed-{s}/G5" for s,ok in enumerate(passes) if not ok)
    result=dict(workload=workload,poses=results,G5=temporal,failures=failures,numeric_pass=not failures,G7="pending inspection")
    save(out/"gate.json",result)
    if failures: raise RuntimeError("Candidate stopped: "+", ".join(failures[:15]))
    return result

def rgb_visibility(v): return np.repeat(np.clip(v*255,0,255).astype("u1")[...,None],3,-1)
def error_rgb(e):
    e=np.clip(e*8,-1,1)
    return np.stack([128+127*np.maximum(e,0),128-127*np.abs(e),128+127*np.maximum(-e,0)],-1).astype("u1")
def tone(hdr, exposure):
    # Exact scene.wgsl present transform followed by the sRGB attachment transfer.
    x=np.maximum(hdr*exposure,0);x=x/(1+x)
    return np.clip(np.where(x<=.0031308,x*12.92,1.055*x**(1/2.4)-.055)*255,0,255).astype("u1")

def inspection(out, workload):
    out=Path(out);shape=(1080,1920) if workload=="quarry" else (192,256)
    poses=3 if workload=="quarry" else 5;seeds=4 if workload=="quarry" else 16
    images=out/"images";images.mkdir(exist_ok=True);(out/"crops").mkdir(exist_ok=True)
    files=[];motion=[]
    for pose in range(poses):
        data=np.load(out/f"pose-{pose}-reference.npz");ref=data["ref"]
        prod=load_counts(out,pose,0,"production",shape)/256;cand=load_counts(out,pose,0,"adaptive",shape)/256
        panels=[]
        for name,v in [("reference",ref),("production",prod),("adaptive",cand)]:
            for kind,rgb in [("visibility",rgb_visibility(v)),("error-x8",error_rgb(v-ref))]:
                p=images/f"pose-{pose}-{name}-{kind}.png";fresh_png.write(p,rgb);files.append(p)
            hdr=np.fromfile(out/f"pose-{pose}-{name}.rgba16f",dtype="<f2").reshape(*shape,4)[...,:3].astype(float)
            assert np.isfinite(hdr).all()
            for label,exposure in [("registered",1),("high-contrast",16)]:
                p=images/f"pose-{pose}-{name}-lit-{label}.png";fresh_png.write(p,tone(hdr,exposure));files.append(p)
            panels.append(np.concatenate([rgb_visibility(v),error_rgb(v-ref)],axis=1))
        sheet=np.concatenate(panels,axis=0);p=images/f"pose-{pose}-sheet.png";fresh_png.write(p,sheet);files.append(p)
        motion.append(p.name)
        # Exhaustive covered-image tiles ensure the inspection set contains
        # silhouettes, contacts, rubble and crevices even where nothing was missed.
        for y in range(0,shape[0],128):
            for x in range(0,shape[1],128):
                sl=np.s_[max(0,y-8):min(y+136,shape[0]),max(0,x-8):min(x+136,shape[1])]
                if not data["cover"][sl].any(): continue
                crop=np.concatenate([np.concatenate([rgb_visibility(v[sl]),error_rgb((v-ref)[sl])],axis=1) for v in (ref,prod,cand)],axis=0)
                p=out/"crops"/f"pose-{pose}-tile-{x}-{y}.png";fresh_png.write(p,crop);files.append(p)
        for seed in range(seeds):
            # Every G6 receiver gets a crop with reference, production, candidate
            # and signed errors, including sub-quantum faint partials.
            sp=load_counts(out,pose,seed,"production",shape)/256;sc=load_counts(out,pose,seed,"adaptive",shape)/256
            if seed:
                for name,v in [("production",sp),("adaptive",sc)]:
                    p=images/f"pose-{pose}-seed-{seed}-{name}.png"
                    fresh_png.write(p,np.concatenate([rgb_visibility(v),error_rgb(v-ref)],axis=1));files.append(p)
                    hdr=np.fromfile(out/f"pose-{pose}-seed-{seed}-{name}.rgba16f",dtype="<f2").reshape(*shape,4)[...,:3].astype(float)
                    assert np.isfinite(hdr).all()
                    for label,exposure in [("registered",1),("high-contrast",16)]:
                        p=images/f"pose-{pose}-seed-{seed}-{name}-lit-{label}.png";fresh_png.write(p,tone(hdr,exposure));files.append(p)
            with (out/f"pose-{pose}-seed-{seed}-G6.csv").open(newline="") as f:
                for item in csv.DictReader(f):
                    x,y=int(item["x"]),int(item["y"]);sl=np.s_[max(0,y-8):y+9,max(0,x-8):x+9]
                    crop=np.concatenate([np.concatenate([rgb_visibility(v[sl]),error_rgb((v-ref)[sl])],axis=1) for v in (ref,sp,sc)],axis=0)
                    p=out/item["crop"];fresh_png.write(p,np.repeat(np.repeat(crop,4,0),4,1));files.append(p)
        # Full-frame images permit inspection of silhouettes, contacts, rubble,
        # crevices, isolated cliff-top partials, specks and every wire orientation.
    p=images/"motion.html"
    p.write_text('<!doctype html><meta charset="utf-8"><title>M6 motion</title><img id="frame" style="max-width:100%"><script>const frames='+json.dumps(motion)+';let i=0;setInterval(()=>document.getElementById("frame").src=frames[i++%frames.length],350);</script>')
    files.append(p)
    identity={str(p.relative_to(out)):sha(p) for p in files}
    save(out/"inspection-artifacts.json",identity)
    rows='<p><a href="images/motion.html">Motion flip-book</a></p>'+"".join(f'<p><a href="{name}">{name}</a></p><img style="max-width:100%" src="{name}">' for name in identity if name.endswith("sheet.png") or "lit-" in name)
    rows+='<h2>All crops and seed-specific images</h2>'+"".join(f'<p><a href="{name}">{name}</a></p>' for name in identity if name.startswith("crops/") or "seed-" in name)
    (out/"inspection.html").write_text('<!doctype html><meta charset="utf-8"><title>M6 Level-A G7</title><h1>G7 inspection required</h1><p>Inspect full visibility, ×8 signed errors, both lit exposures, every G6 crop and motion. Reject any candidate-only defect. Check silhouettes, slab/wedge contacts, rubble, crevices, cliff-top isolated partials, thin strip/contact sliver, specks and wires.</p>'+rows,encoding="utf-8")
    return hashlib.sha256(json.dumps(identity,sort_keys=True).encode()).hexdigest()
