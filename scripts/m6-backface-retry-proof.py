"""Classify all changed centre rays using exact rational intersections; CPU only."""
import argparse
import importlib.util
import json
import os
from pathlib import Path

os.environ.update(OPENBLAS_NUM_THREADS="2", OMP_NUM_THREADS="2")
import numpy as np


def module(name, filename):
    spec=importlib.util.spec_from_file_location(name,Path(__file__).with_name(filename))
    value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value)
    return value


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("manifest",type=Path)
    args=parser.parse_args();m=json.loads(args.manifest.read_text());out=Path(m["output"])
    proof=module("backface_exact","m6-receiver-ray-proof.py")
    source=module("backface_source","quarry-shadow-probe.py").Source(Path(m["asset"]))
    result=[]
    for ray in json.loads((out/"difference-rays.json").read_text()):
        pose=ray["pose"];x,y=ray["pixel"]
        records={name:np.memmap(out/f"pose-{pose}-hard-{name}.hits",dtype="<u4",mode="r",shape=(1080,1920,4))[y,x].tolist() for name in ("adjacency","retry")}
        assert records["adjacency"]==ray["adjacency_record"],"diagnostic must reproduce the counted capture"
        records["raw"]=ray["record"]
        origin=[proof.f32(v) for v in ray["origin_bits"]];direction=[proof.f32(v) for v in ray["direction_bits"]]
        matrix=[[proof.Fraction.from_float(float(v)) for v in row] for row in ray["model"]]
        hits={};positions={}
        for name,record in records.items():
            slot,primitive=record[:2];assert slot==ray["record"][0]
            mesh,_=source.placements[slot-1];_,vertices,indices=source.meshes[mesh][0]
            local=vertices[indices[primitive],:3];positions[name]=local.tolist()
            world=[[matrix[k][3]+sum(matrix[k][j]*proof.Fraction.from_float(float(p[j])) for j in range(3)) for k in range(3)] for p in local]
            hit=proof.intersect(origin,direction,world)
            facing=proof.dot(proof.cross(proof.sub(world[1],world[0]),proof.sub(world[2],world[0])),direction)
            hits[name]=dict(placement=slot,primitive=primitive,front_facing=facing<0,intersection=hit)
        a,b=hits["adjacency"]["intersection"],hits["retry"]["intersection"]
        common=len(set(map(tuple,positions["adjacency"])) & set(map(tuple,positions["retry"])))
        valid_a=a is not None and a["valid"];valid_b=b is not None and b["valid"]
        gap=float(proof.Fraction(b["exact_t"])-proof.Fraction(a["exact_t"])) if a and b else None
        classification=("retry misses the exact triangle" if valid_a and not valid_b else
            "retry selects a farther exact intersection" if valid_a and valid_b and gap>0 else
            "retry selects a nearer exact intersection" if valid_a and valid_b and gap<0 else
            "same facet; exact ray outside triangle (coverage case)" if records["adjacency"][:2]==records["retry"][:2] and not valid_a else
            "same exact facet" if records["adjacency"][:2]==records["retry"][:2] else "boundary/coverage case")
        result.append(dict(pose=pose,pixel=[x,y],raw_hardware_front=ray["front_face"],hits=hits,
            shared_source_vertices=common,retry_minus_adjacency_exact_t=gap,classification=classification))
    (out/"difference-exact.json").write_text(json.dumps(result,indent=2))
    for row in result:print(row["pose"],row["pixel"],row["classification"],row["retry_minus_adjacency_exact_t"])


if __name__=="__main__":main()
