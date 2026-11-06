"""Generate S1's shared metre/Y-up triangle manifest and private glTF transport.

No engine launch. All geometry is authored here and freely redistributable.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct


def scene():
    meshes = []

    def quad(name, points, thin=True):
        a, b, c, _ = points
        u = [b[i] - a[i] for i in range(3)]
        v = [c[i] - a[i] for i in range(3)]
        n = [u[1]*v[2]-u[2]*v[1], u[2]*v[0]-u[0]*v[2], u[0]*v[1]-u[1]*v[0]]
        norm = math.sqrt(sum(x*x for x in n))
        meshes.append(dict(name=name, positions=points, normals=[[x/norm for x in n]]*4,
                           indices=[0, 1, 2, 0, 2, 3], thin=thin))

    quad("floor", [(-1, 0, 1), (1, 0, 1), (1, 0, -1), (-1, 0, -1)], False)
    for gi, gap in enumerate((.0002, .001, .005)):
        for wi, width in enumerate((.001, .005, .020)):
            z = -.82 + (gi*3+wi)*.055
            quad(f"strip-{width*1000:g}mm-gap-{gap*1000:g}mm",
                 [(-.65, gap, z+width/2), (.35, gap, z+width/2),
                  (.35, gap, z-width/2), (-.65, gap, z-width/2)])
    for label, y, x, z, tilt in (("stack-low", .02, -.45, -.02, .12),
                                ("stack-high", .10, -.38, .08, -.18)):
        quad(label, [(x+dx, y+dx*math.tan(tilt), z+dz)
                     for dx, dz in ((-.1,.1),(.1,.1),(.1,-.1),(-.1,-.1))])
    height = .12 * math.tan(math.radians(60))
    for label, x, z, concave in (("concave", .57, .10, True), ("convex", .57, .60, False)):
        heights = (height, .00001, height) if concave else (.00001, height, .00001)
        for side in range(2):
            xa, xb = x + (side-1)*.12, x + side*.12
            quad(f"{label}-{side}", [(xa, heights[side], z+.16), (xb, heights[side+1], z+.16),
                                     (xb, heights[side+1], z-.16), (xa, heights[side], z-.16)], False)
    quad("shell-top", [(-.65,.12,.77),(-.25,.12,.77),(-.25,.12,.52),(-.65,.12,.52)])
    quad("shell-side", [(-.65,.00001,.77),(-.65,.12,.77),(-.65,.12,.52),(-.65,.00001,.52)])
    quad("shell-back", [(-.65,.00001,.77),(-.25,.00001,.77),(-.25,.12,.77),(-.65,.12,.77)])
    eye, target = [1.8,1.2,2.4], [0,.05,0]
    direction = [eye[i]-target[i] for i in range(3)]
    length = math.sqrt(sum(x*x for x in direction))
    poses = [dict(eye=eye, target=target),
             dict(eye=[eye[i]+.5*direction[i]/length for i in range(3)],
                  target=[target[i]+.5*direction[i]/length for i in range(3)])]
    elevation, azimuth = map(math.radians, (20,35))
    to_light = [math.cos(elevation)*math.sin(azimuth), math.sin(elevation), math.cos(elevation)*math.cos(azimuth)]
    track = []
    for frame in range(224):
        t = 0 if frame < 32 else min(1,(frame-31)/64) if frame < 96 else 1 if frame < 128 else max(0,1-(frame-127)/64) if frame < 192 else 0
        track.append(dict(frame=frame, seconds=frame/60,
                          eye=[eye[i]+.5*t*direction[i]/length for i in range(3)],
                          target=[target[i]+.5*t*direction[i]/length for i in range(3)]))
    return dict(schema=1, units="metres, Y up; azimuth measured from +Z towards +X", size=[1920,1080],
                fov_y_degrees=45, poses=poses, track=track, to_light=to_light, sun_radius=.04,
                sun_lux=[1000]*3, exposure_stops=-8, environment=[0,0,0], radiance_environment=[1,1,1],
                material=dict(base_color=[.5,.5,.5,1], roughness=.6, metallic=0, double_sided=True),
                meshes=meshes, triangles=sum(len(m["indices"])//3 for m in meshes),
                registration=dict(contact_lost="contact & candidate-film > .5", leaks="umbra & candidate-film > .1",
                                  solid_flips="(lit & candidate-film < -.5) | (umbra & candidate-film > .5)",
                                  populations="unchanged fresh_level_b_metrics.populations on film; physical view depth",
                                  static_frames=128, repeats=8, sequence_scored=False))


def write_gltf(s, out):
    blob = bytearray(); views=[]; accessors=[]; meshes=[]
    def accessor(values, code, component, kind):
        while len(blob)%4: blob.append(0)
        start = len(blob)
        flat = [v for row in values for v in row] if kind == "VEC3" else values
        blob.extend(struct.pack("<"+code*len(flat), *flat))
        views.append(dict(buffer=0,byteOffset=start,byteLength=len(blob)-start))
        entry=dict(bufferView=len(views)-1,componentType=component,count=len(values),type=kind)
        if kind == "VEC3":
            entry.update(min=[min(v[i] for v in values) for i in range(3)],max=[max(v[i] for v in values) for i in range(3)])
        accessors.append(entry)
        return len(accessors)-1
    for m in s["meshes"]:
        # The installed glTF importer maps (x,y,z) to UE (x,z,y) centimetres.
        # Bake (-source.z,source.y,source.x), as the retained quarry transport does.
        convert=lambda values:[[-v[2],v[1],v[0]] for v in values]
        p=accessor(convert(m["positions"]),"f",5126,"VEC3"); n=accessor(convert(m["normals"]),"f",5126,"VEC3")
        idx=accessor(m["indices"],"I",5125,"SCALAR")
        meshes.append(dict(name=m["name"],primitives=[dict(attributes=dict(POSITION=p,NORMAL=n),indices=idx,material=0)]))
    document=dict(asset=dict(version="2.0",generator="Renderlab S1 procedural fixture"),
                  buffers=[dict(uri="contact-fold.bin",byteLength=len(blob))],bufferViews=views,accessors=accessors,
                  meshes=meshes,nodes=[dict(name=m["name"],mesh=i) for i,m in enumerate(s["meshes"])],
                  scenes=[dict(nodes=list(range(len(meshes))))],scene=0,
                  materials=[dict(name="S1Grey",doubleSided=True,pbrMetallicRoughness=dict(baseColorFactor=s["material"]["base_color"],roughnessFactor=.6,metallicFactor=0))])
    (out/"contact-fold.bin").write_bytes(blob)
    (out/"contact-fold.gltf").write_text(json.dumps(document,indent=2))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--output",type=Path,required=True)
    out=parser.parse_args().output;out.mkdir(parents=True,exist_ok=True)
    s=scene();write_gltf(s,out)
    n=16
    header=[124,0x100f,n,n,n*16,0,1]+[0]*11+[32,4,int.from_bytes(b"DX10","little"),0,0,0,0,0]+[0x1008,0xfe00,0,0,0]
    (out/"constant.dds").write_bytes(b"DDS "+struct.pack("<31I",*header)+struct.pack("<5I",2,3,4,1,0)+struct.pack("<4f",1,1,1,1)*(6*n*n))
    (out/"scene.json").write_text(json.dumps(s,indent=2))
    hashes={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in out.iterdir() if p.is_file() and p.name!="identities.json"}
    (out/"identities.json").write_text(json.dumps(hashes,indent=2))
    print(json.dumps(dict(triangles=s["triangles"],sha256=hashes)))


if __name__ == "__main__": main()
