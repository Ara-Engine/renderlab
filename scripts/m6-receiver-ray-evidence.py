"""Independent f64 witness intersection, replica statistics and native-image crops."""
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont

OUT = Path("E:/renderlab-work/receiver-adoption/followup")
PROBE = OUT / "probe"
X, Y = 1295, 513


def main():
    all_rays = np.fromfile(PROBE / "rays.f32", "<f4").reshape(256, 12, 4)
    a = all_rays[154]
    origin = a[0, :3].astype(float); direction = a[1, :3].astype(float)
    matrix = a[7:11, :3].astype(float).T
    vertices = a[4:7].astype(float) @ matrix.T
    edge1 = vertices[1]-vertices[0]; edge2 = vertices[2]-vertices[0]
    cross = np.cross(direction, edge2); det = edge1 @ cross
    delta = origin-vertices[0]; u = delta @ cross/det
    q = np.cross(delta, edge1); v = direction @ q/det
    distance = edge2 @ q/det
    assert distance > a[0, 3] and u > 0 and v > 0 and u+v < 1
    masks = np.fromfile(PROBE / "masks.u32", "<u4").reshape(-1, 8)
    blocked = np.array([sum(int(v).bit_count() for v in row) for row in masks])
    assert blocked[0] == 1 and blocked[1:65].sum() == 0
    replicas = 1-blocked[1:]/256
    hit = np.fromfile(PROBE / "hit.u32", "<u4").reshape(1080, 1920, 4)[Y, X]
    result = dict(pixel=[X,Y], reference_16384=1., sigma_16384=0., exact=255/256,
        mask_words=masks[0].tolist(), blocked_ray=154,
        receiver_placement=int(hit[0]-1), receiver_primitive=int(hit[1]),
        blocking_ids=a[3].view("<u4").tolist(), origin=origin.tolist(), direction=direction.tolist(),
        epsilon=float(a[0,3]), t_max=float(a[1,3]), gpu_t=float(a[2,0]),
        gpu_bary=[float(1-a[2,1]-a[2,2]),float(a[2,1]),float(a[2,2])],
        f64_t=float(distance), f64_bary=[float(1-u-v),float(u),float(v)],
        n_dot_direction=float(a[11,:3] @ direction),
        world_vertices_f64=vertices.tolist(), replicas=len(replicas), total_rays=len(replicas)*256,
        blocked_rays=int(blocked[1:].sum()), replicas_with_hits=int((blocked[1:]>0).sum()),
        reference_high=float(replicas.mean()), sigma_high=float(replicas.std(ddof=1)))
    (OUT / "ray-evidence.json").write_text(json.dumps(result, indent=2))
    images = OUT / "images"; images.mkdir(exist_ok=True)
    bounds = (X-10,Y-10,X+11,Y+11)
    ref = np.load(OUT.parent / "primary/pose-0-reference.npz")["ref"]
    exact = np.fromfile(PROBE / "visibility.u32", "<u4").reshape(1080,1920)/256
    panels=[]
    for name, data in [("Full image, native pixels",None),("16,384-ray visibility",ref),("Exact 256-ray visibility",exact),("Error vs ref (1 ray = red)",np.maximum(ref-exact,0))]:
        if data is None:
            image=Image.open(OUT.parent/"images/pose-0-new-exact-full-0.png").convert("RGB")
        elif name.startswith("Error"):
            amount=np.clip(data*256,0,1);rgb=np.zeros((*data.shape,3),dtype=np.uint8)
            rgb[...,0]=np.rint(amount*255).astype(np.uint8);image=Image.fromarray(rgb)
        else:
            image=Image.fromarray(np.rint(np.clip(data,0,1)*255).astype(np.uint8)).convert("RGB")
        image=image.crop(bounds).resize((210,210),Image.Resampling.NEAREST)
        panel=Image.new("RGB",(230,258),(30,30,30));panel.paste(image,(10,35))
        draw=ImageDraw.Draw(panel);draw.text((10,8),name,fill="white")
        draw.rectangle((109,134,120,145),outline=(0,220,255),width=1)
        panels.append(panel)
    sheet=Image.new("RGB",(460,516))
    for i,panel in enumerate(panels):sheet.paste(panel,((i%2)*230,(i//2)*258))
    sheet.save(images/"g2-crops.png")
    print(json.dumps(result,indent=2))


if __name__ == "__main__": main()
