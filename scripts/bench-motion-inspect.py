"""Whole-frame motion sheets, with actual common-transform images from both engines."""
import argparse
import importlib.util
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont


def compact(root, scene, case, film_tools):
    """Keep a readable eight-panel comparison, including the per-time film crop."""
    index = 16 if scene["bench"] == "S2" else 352
    directory = root / "images"
    source = directory / f"{case}-f{index:04}.png"
    if not source.exists():
        return
    film_path = directory/f"{case}-film-f{index:04}.png"
    if not film_path.exists():
        with np.load(root/"renderlab"/case/"reference"/f"pose-{index}.npz") as reference:
            film_tools.tone(reference["hdr"]).save(film_path)
    with Image.open(film_path) as image:
        film = image.copy()
    rect = (128,32,384,224) if scene["bench"] == "S2" else (96,40,288,184)
    panels = [("Film", film.crop(rect))]
    x0,y0,_,_ = scene["crop"]
    absolute = tuple(v+(x0 if i%2==0 else y0) for i,v in enumerate(rect))
    for mode in ("hard","Exact","Interactive","Temporal","VSM","HRT1","HRT256"):
        native = mode in ("VSM","HRT1","HRT256")
        path = root/"unreal"/case/mode/f"{index:04}"/"final-radiance.png" if native else root/"renderlab"/case/mode/f"{index:04}-full.png"
        with Image.open(path) as image:
            panels.append((f"{'UE' if native else 'RL'} {mode}", image.crop(absolute)))
    sheet = Image.new("RGB", (960,404), (20,27,35))
    draw = ImageDraw.Draw(sheet)
    font = ImageFont.truetype("C:/Windows/Fonts/segoeui.ttf", 16)
    for i,(label,image) in enumerate(panels):
        x,y = i%4*240,i//4*202
        draw.text((x+4,y+1),label,font=font,fill="white")
        sheet.paste(image.resize((240,180),Image.Resampling.NEAREST),(x,y+22))
    sheet.save(directory/f"{case}-compact-f{index:04}.png")


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--root",type=Path,required=True);args=parser.parse_args();root=args.root
    scene=json.loads((root/"fixture/scene.json").read_text())
    spec=importlib.util.spec_from_file_location("s1_score",Path(__file__).with_name("bench-contact-fold-score.py"));s1=importlib.util.module_from_spec(spec);spec.loader.exec_module(s1)
    frames=(0,16,32,95,159) if scene["bench"]=="S2" else (31,191,319,352,359,384)
    for case in sorted(p.name for p in (root/"renderlab").iterdir() if p.is_dir()):
        panels=[]
        for mode in ("hard","Exact","Interactive","Temporal","VSM","HRT1","HRT256"):
            for index in frames:
                path=root/"renderlab"/case/mode/f"{index:04}-full.png" if mode in ("hard","Exact","Interactive","Temporal") else root/"unreal"/case/mode/f"{index:04}"/"final-radiance.png"
                if path.exists():
                    with Image.open(path) as image:panels.append((f"{'RL' if mode in ('hard','Exact','Interactive','Temporal') else 'UE'} {mode} f{index}",image.copy()))
        if panels:s1.film.sheet(panels,columns=len(frames),width=384).save(root/"images"/f"{case}-whole-motion.png")
        compact(root, scene, case, s1.film)
    print("whole motion sheets",root/"images")


if __name__=="__main__":main()
