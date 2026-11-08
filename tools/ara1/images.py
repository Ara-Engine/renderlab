"""CPU-only side-by-side sheets, aspect crops and reference scores.

No resizing or exposure fitting is performed for scoring. PNG scores measure the
displayed result. The physical subcommand delegates to Renderlab's existing U0b
split-half scorer for native, calibrated HDR captures (never reconstructed from PNG).
"""
import argparse
import importlib.util
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw
from capture import digest

REPO=Path(__file__).resolve().parents[2]
PANELS=(("ara","Ara"),("rt","Renderlab RT"),("nonrt","Renderlab non-RT"),("reference","Path-traced reference"))


def load_script(name,path):
    spec=importlib.util.spec_from_file_location(name,path)
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module


def checked_manifest(path):
    value=json.loads(path.read_text())
    base=path.parent
    scene=Path(value["scene"])
    if not scene.is_absolute():scene=base/scene
    if digest(scene)!=value["scene_sha256"]:raise ValueError("comparison scene changed")
    spec=json.loads(scene.read_text())
    for row in value["frames"]:
        if not 0<=row["index"]<len(spec["frames"]):raise ValueError("unregistered frame index")
        for key,_ in PANELS:
            if row.get(key):
                p=Path(row[key]);row[key]=p if p.is_absolute() else base/p
                with Image.open(row[key]) as image:
                    if list(image.size)!=spec["size"]:raise ValueError("image extent differs from registered scene")
        if row.get("reference"):
            identity=json.loads(Path(str(row["reference"])+".json").read_text())
            expected=dict(scene_sha256=value["scene_sha256"],frame_index=row["index"],image_sha256=digest(row["reference"]),
                          display_transform=value["display_transform"])
            if any(identity.get(k)!=v for k,v in expected.items()):raise ValueError("reference scene/frame/display identity mismatch")
            if identity.get("accepted_for_scoring") is not True:raise ValueError("reference has no explicit scoring qualification")
    return value,spec


def crop_box(rect,size):
    if len(rect)!=4 or not 0<=rect[0]<rect[2]<=1 or not 0<=rect[1]<rect[3]<=1:raise ValueError("crops use normalized x0,y0,x1,y1")
    w,h=size
    box=tuple(round(v*(w if i%2==0 else h)) for i,v in enumerate(rect))
    if box[0]>=box[2] or box[1]>=box[3]:raise ValueError("crop is empty at this resolution")
    return box


def sheet(row,spec,rect=None):
    size=spec["size"];box=crop_box(rect,size) if rect else (0,0,*size)
    w,h=box[2]-box[0],box[3]-box[1]
    cell=max(w,160)
    result=Image.new("RGB",(4*cell,h+46),(22,24,28));draw=ImageDraw.Draw(result)
    for n,(key,label) in enumerate(PANELS):
        draw.text((n*cell+8,7),row.get("ara_label",label) if key=="ara" else label,fill="white")
        draw.text((n*cell+8,25),f'frame {row["index"]}: {spec["frames"][row["index"]]["phase"]}',fill=(180,185,190))
        if row.get(key):
            with Image.open(row[key]) as im:result.paste(im.convert("RGB").crop(box),(n*cell+(cell-w)//2,46))
        else:draw.text((n*cell+8,65),"Unavailable",fill=(240,170,120))
    return result


def compose(manifest,out):
    value,spec=checked_manifest(manifest);out.mkdir(parents=True,exist_ok=True)
    for row in value["frames"]:
        sheet(row,spec).save(out/f'frame-{row["index"]:04}.png')
        for aspect,rect in value.get("crops",{}).items():
            if Path(aspect).name!=aspect:raise ValueError("aspect name must be a filename")
            sheet(row,spec,rect).save(out/f'frame-{row["index"]:04}-{aspect}.png')


def score(manifest,out):
    value,spec=checked_manifest(manifest)
    if spec.get("startup_minimal"):raise ValueError("startup-minimal smoke frames are not quality/performance comparison inputs")
    scan=load_script("ara1_scan_metrics",REPO/"scripts/scan-quality-compare.py")
    metrics=load_script("ara1_gate_metrics",REPO/"support/fresh_level_b_metrics.py")
    rows=[];previous={}
    for frame in value["frames"]:
        i=frame["index"];pose=spec["frames"][i]
        if pose["cut"]:previous.clear()
        if not frame.get("reference"):
            rows.append(dict(index=i,status="no matched qualified reference; no accuracy score"));continue
        ref=scan.linear(scan.load(frame["reference"]))
        regions={"whole_image":np.ones(ref.shape[:2],bool)}
        for name,rect in value.get("crops",{}).items():
            x0,y0,x1,y1=crop_box(rect,spec["size"]);mask=np.zeros(ref.shape[:2],bool);mask[y0:y1,x0:x1]=True;regions[name]=mask
        for key,_ in PANELS[:3]:
            if not frame.get(key):continue
            cand=scan.linear(scan.load(frame[key]));error=cand-ref
            stats={name:metrics.summary(error[mask].reshape(-1)) for name,mask in regions.items()}
            record=dict(index=i,engine=key,phase=pose["phase"],reference_sha256=digest(frame["reference"]),candidate_sha256=digest(frame[key]),
                        linear_display_rgb=stats,display_mae=scan.statistics(error,regions))
            if key in previous and previous[key][0]+1==i:
                # A screen-coordinate residual-change diagnostic; not a motion-warped
                # estimator, and never compared across a cut or a missing frame.
                record["screen_residual_change"]=metrics.summary((error-previous[key][1]).reshape(-1))
            previous[key]=(i,error);rows.append(record)
    out.parent.mkdir(parents=True,exist_ok=True)
    out.write_text(json.dumps(dict(scope="final displayed-image error; includes shading and display differences; no HDR/visibility inference",
        manifest_sha256=digest(manifest),metric_sources=["scripts/scan-quality-compare.py","support/fresh_level_b_metrics.py"],rows=rows),indent=2,allow_nan=False)+"\n")


def main():
    p=argparse.ArgumentParser(description=__doc__)
    sub=p.add_subparsers(dest="command",required=True)
    for name in ("compose","score"):
        s=sub.add_parser(name);s.add_argument("manifest",type=Path);s.add_argument("out",type=Path)
    s=sub.add_parser("physical");s.add_argument("reference",type=Path);s.add_argument("candidate",type=Path)
    s.add_argument("--pre-exposure",type=float,required=True);s.add_argument("--stops",type=float,required=True);s.add_argument("--out",type=Path,required=True)
    a=p.parse_args()
    if a.command=="compose":compose(a.manifest,a.out)
    elif a.command=="score":score(a.manifest,a.out)
    else:load_script("ara1_u0b",REPO/"scripts/m7-u0b-score.py").score(a)


if __name__=="__main__":main()
