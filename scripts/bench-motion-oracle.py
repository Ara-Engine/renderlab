"""Independent S1 double-precision integrator at S2 moving-time witnesses."""
import argparse
import importlib.util
import json
from pathlib import Path

import numpy as np
from numba import set_num_threads
from scipy.stats import qmc


def load(name):
    spec=importlib.util.spec_from_file_location(name.replace("-","_"),Path(__file__).with_name(name+".py"))
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--root",type=Path,required=True);args=parser.parse_args()
    set_num_threads(2);root=args.root;scene=json.loads((root/"fixture/scene.json").read_text())
    if scene["bench"]!="S2":raise ValueError("S1's independent oracle has a fixed 45-degree FOV; S3 uses the full-dispatch equality control")
    oracle=load("bench-contact-fold-oracle");results=[]
    track=scene["tracks"][0]
    for radius in scene["radii"]:
        case=f"{track['name']}-r{radius:g}"
        for index in (0,17):
            pose=track["frames"][index];triangles=[];normals=[];placements=[];thin=[]
            for mi,mesh in enumerate(scene["meshes"]):
                points=np.array(mesh["positions"],float)
                if mesh["name"] in ("reveal-plate","shadow-blocker"):points[:,0]+=pose["plate_x"] if mesh["name"]=="reveal-plate" else pose["blocker_x"]
                for indices in np.array(mesh["indices"]).reshape(-1,3):
                    triangles.append(points[indices]);normals.append(mesh["normals"][0]);placements.append(mi);thin.append(mesh["thin"])
            ref=np.load(root/"renderlab"/case/"reference"/f"pose-{index}.npz")
            yy,xx=np.nonzero(ref["cover"]&(ref["ref"]>.01)&(ref["ref"]<.99));select=np.linspace(0,len(xx)-1,2).astype(int)
            local=np.stack([xx[select],yy[select]],-1);coords=local+np.array(scene["crop"][:2])
            eye=np.array(pose["eye"]);forward=np.array(pose["target"])-eye;forward/=np.linalg.norm(forward)
            right=np.cross(forward,[0,1,0]);right/=np.linalg.norm(right);up=np.cross(right,forward)
            light=np.array(pose["to_light"]);basis=np.cross(light,[0,1,0]);basis/=np.linalg.norm(basis);other=np.cross(light,basis)
            disk=qmc.Sobol(2,scramble=True,seed=7143).random_base2(10)
            directions=light+np.tan(radius)*np.sqrt(disk[:,0,None])*(np.cos(2*np.pi*disk[:,1,None])*basis+np.sin(2*np.pi*disk[:,1,None])*other)
            directions/=np.linalg.norm(directions,axis=-1,keepdims=True)
            if radius==0:directions=light[None,:]
            cpu,n=oracle.integrate(coords.astype(np.int64),np.array(triangles),np.array(placements),np.array(normals),np.array(thin),eye,forward,right,up,light,directions,1e-5)
            gpu=ref["ref"][local[:,1],local[:,0]];sigma=ref["sigma"][local[:,1],local[:,0]]
            source_variance=cpu*(1-cpu)/len(directions) if radius>0 else 0
            bound=5*np.sqrt(sigma*sigma+source_variance)+1/256
            row=dict(case=case,frame=index,pixels=coords.tolist(),cpu=cpu.tolist(),gpu=gpu.tolist(),accepted=n.tolist(),bound=bound.tolist(),maximum_error=float(np.max(np.abs(cpu-gpu))),passed=bool(np.all(np.abs(cpu-gpu)<=bound)))
            results.append(row);print(json.dumps(row),flush=True)
    (root/"oracle.json").write_text(json.dumps(dict(method="S1 f64 Moller-Trumbore, matching 256 footprint strata, independent Sobol disk; sampled S2 checks only",results=results),indent=2))
    if not all(row["passed"] for row in results):raise SystemExit(1)


if __name__=="__main__":main()
