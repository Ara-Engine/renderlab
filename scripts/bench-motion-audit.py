"""Export registered matrices and audit actual native cameras/time steps and T0 resets."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re

import numpy as np


def camera(pose,scene):
    eye=np.array(pose["eye"],dtype=float);forward=np.array(pose["target"],dtype=float)-eye;forward/=np.linalg.norm(forward)
    right=np.cross(forward,[0,1,0]);right/=np.linalg.norm(right);up=np.cross(right,forward)
    view=np.eye(4);view[:3,:3]=np.stack([right,up,-forward]);view[:3,3]=-view[:3,:3]@eye
    h=1/math.tan(math.radians(scene["fov_y_degrees"])/2);near=.01;far=100
    projection=np.array([[h*1080/1920,0,0,0],[0,h,0,0],[0,0,near/(far-near),near*far/(far-near)],[0,0,-1,0]])
    return view,projection,projection@view


def matrices(root,scene):
    tracks=[];displacements=[]
    for track in scene["tracks"]:
        frames=[]
        for pose in track["frames"]:
            view,projection,matrix=camera(pose,scene)
            frames.append(dict(frame=pose["frame"],seconds=pose["seconds"],view=view.tolist(),projection=projection.tolist(),world_to_clip=matrix.tolist()))
        if "pixels_per_frame" in track:
            pixels=[]
            for frame in frames[:2]:
                clip=np.array(frame["world_to_clip"])@np.array([0,0,0,1]);pixels.append((clip[0]/clip[3]+1)*960)
            displacements.append(dict(track=track["name"],floor_delta_x_pixels=pixels[1]-pixels[0],expected=-track["pixels_per_frame"]))
        tracks.append(dict(name=track["name"],frames=frames))
    result=dict(scope="Registered f64 matrices; Renderlab uses f32 evaluation, finite reversed Z, near .01/far 100; Unreal camera/FOV validated separately and uses native reversed depth.",
                source_sha256=hashlib.file_digest((root/"fixture/scene.json").open("rb"),"sha256").hexdigest(),tracks=tracks,floor_displacement=displacements)
    (root/"camera-matrices.json").write_text(json.dumps(result,indent=2))
    if any(abs(r["floor_delta_x_pixels"]-r["expected"])>1e-9 for r in displacements):raise RuntimeError("floor displacement registration failed")
    return result


def audit(root,scene,renderlab_only=False,native_only=False):
    failures=[];resets=[];world_deltas=[];cameras=[];returns=[]
    for track in ([] if native_only else scene["tracks"]):
        for radius in scene["radii"]:
            case=f"{track['name']}-r{radius:g}";path=root/"renderlab"/case/"Temporal"
            if all(track["frames"][0][key]==track["frames"][-1][key] for key in ("eye","target","to_light","plate_x","blocker_x")):
                for mode in ("hard","Exact","Interactive","Temporal"):
                    folder=root/"renderlab"/case/mode
                    a=hashlib.file_digest((folder/"0000.rgba16f").open("rb"),"sha256").hexdigest()
                    b=hashlib.file_digest((folder/f"{len(track['frames'])-1:04}.rgba16f").open("rb"),"sha256").hexdigest()
                    returns.append(dict(case=case,mode=mode,first_sha256=a,last_sha256=b,equal=a==b))
                    if a!=b:failures.append(dict(kind="return identity",case=case,mode=mode))
            for pose in track["frames"]:
                i=pose["frame"];info=json.loads((path/f"{i:04}.json").read_text())
                if radius>0 and info["reset"]:
                    a=(path/f"{i:04}.rgba16f").read_bytes();b=(root/"renderlab"/case/"Interactive"/f"{i:04}.rgba16f").read_bytes()
                    equal=hashlib.sha256(a).digest()==hashlib.sha256(b).digest()
                    detail={}
                    if not equal:
                        av=np.frombuffer(a,dtype="<f2").reshape(-1,4)[:,:3].astype(float)
                        bv=np.frombuffer(b,dtype="<f2").reshape(-1,4)[:,:3].astype(float)
                        detail=dict(differing_pixels=int(np.any(av!=bv,axis=1).sum()),maximum_rgb_difference=float(np.max(np.abs(av-bv))),rgb_rms_difference=float(np.sqrt(np.mean((av-bv)**2))))
                    resets.append(dict(case=case,frame=i,equal=equal,**detail))
                    if not equal:failures.append(dict(kind="T0 identity",case=case,frame=i))
    if renderlab_only:
        result=dict(temporal_reset_frames=len(resets),all_resets_equal_interactive=all(r["equal"] for r in resets),returns=returns,failures=failures)
        result["reset_witnesses"]=resets
        (root/"audit-renderlab.json").write_text(json.dumps(result,indent=2));print(json.dumps(dict(temporal_reset_frames=len(resets),all_resets_equal_interactive=result["all_resets_equal_interactive"],return_checks=len(returns),all_returns_equal=all(r["equal"] for r in returns),failure_count=len(failures),first_failures=failures[:5]),indent=2))
        if failures:raise SystemExit(1)
        return
    logs=sorted([*(root/"unreal").glob("*/*/camera-native.log"),*(root/"unreal").glob("*/*/camera-hdr.log")])
    log="\n".join(p.read_text(encoding="utf-8-sig",errors="replace") for p in logs)
    pattern=r"Location=\(X=([^,]+),Y=([^,]+),Z=([^\)]+)\),Rotation=\(Pitch=([^,]+),Yaw=([^,]+),Roll=([^\)]+)\),FOV=([^,]+)"
    for block in log.split("MOTION_VIEW ")[1:]:
        directory=Path(block.splitlines()[0].strip());recipe=json.loads((directory/"recipe.json").read_text());pose=recipe["pose"]
        line=next((line for line in block.splitlines() if "CameraCachePrivate =" in line),None)
        match=re.search(pattern,line or "")
        if match is None:failures.append(dict(kind="missing native camera",directory=str(directory)));continue
        x,y,z,pitch,yaw,roll,fov=map(float,match.groups());eye=np.array([y,z,-x])/100
        a,b=np.radians([pitch,yaw]);ue=np.array([np.cos(a)*np.cos(b),np.cos(a)*np.sin(b),np.sin(a)])
        forward=ue[[1,2,0]]*[1,1,-1];expected=np.array(pose["target"],dtype=float)-pose["eye"];expected/=np.linalg.norm(expected)
        expected_fov=math.degrees(2*math.atan(math.tan(math.radians(scene["fov_y_degrees"])/2)*1920/1080))
        errors=dict(eye_m=float(np.max(np.abs(eye-pose["eye"]))),direction=float(np.max(np.abs(forward-expected))),fov_degrees=abs(fov-expected_fov),roll_degrees=abs(roll))
        cameras.append(dict(directory=str(directory),native_eye_m=eye.tolist(),native_forward=forward.tolist(),native_horizontal_fov_degrees=fov,errors=errors))
        if errors["eye_m"]>1e-6 or errors["direction"]>2e-6 or errors["fov_degrees"]>1e-5 or errors["roll_degrees"]>1e-6:failures.append(dict(kind="native camera",directory=str(directory),errors=errors))
        world_deltas.append(recipe["world_delta_seconds"])
    expected=sum(len(t["frames"])*len(scene["radii"])*3*2 for t in scene["tracks"])
    if len(cameras)!=expected:failures.append(dict(kind="native camera coverage",actual=len(cameras),expected=expected))
    if world_deltas and max(abs(v-1/60) for v in world_deltas)>1e-6:failures.append(dict(kind="world time step",minimum=min(world_deltas),maximum=max(world_deltas)))
    result=dict(scope="native only" if native_only else "Renderlab and native",temporal_reset_frames=len(resets),all_resets_equal_interactive=None if native_only else all(r["equal"] for r in resets),native_camera_count=len(cameras),expected_native_camera_count=expected,
        maximum_camera_errors={key:max(r["errors"][key] for r in cameras) for key in cameras[0]["errors"]} if cameras else None,
        world_delta_seconds=[min(world_deltas),max(world_deltas)] if world_deltas else [],failures=failures,reset_witnesses=resets,cameras=cameras,returns=returns)
    (root/"audit.json").write_text(json.dumps(result,indent=2))
    print(json.dumps({key:value for key,value in result.items() if key not in ("reset_witnesses","cameras","returns","failures")}|dict(failure_count=len(failures),first_failures=failures[:5]),indent=2))
    if failures:raise SystemExit(1)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--root",type=Path,required=True);parser.add_argument("--matrices-only",action="store_true");parser.add_argument("--renderlab-only",action="store_true");parser.add_argument("--native-only",action="store_true");args=parser.parse_args()
    scene=json.loads((args.root/"fixture/scene.json").read_text());matrices(args.root,scene)
    if args.native_only and args.renderlab_only:parser.error("choose one audit scope")
    if not args.matrices_only:audit(args.root,scene,args.renderlab_only,args.native_only)


if __name__=="__main__":main()
