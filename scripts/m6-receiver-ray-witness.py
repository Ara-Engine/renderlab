"""Emit an offscreen forced-neighbour test from actual GPU ray bit readback.

The generated module contains private asset coordinates: keep it in ignored E: evidence.
The three targets per pose are the original returned hit, that hit repaired, and the
exact same-ray nearer neighbour repaired. `corner` counts fully blocked samples;
`edge` counts selection of the exact nearer primitive, `interior` the other primitive.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import struct


def bits(value):
    return struct.unpack("<I", struct.pack("<f", value))[0]


def vector(values, words=False):
    return "bitcast<vec3<f32>>(vec3<u32>(" + ",".join(f"{int(v) if words else bits(v)}u" for v in values) + "))"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("rays", "proof", "asset", "module", "out"):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location("source_probe", Path(__file__).with_name("quarry-shadow-probe.py"))
    source_probe = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(source_probe)
    source = source_probe.Source(args.asset)
    rays, proof = json.loads(args.rays.read_text()), json.loads(args.proof.read_text())
    cases = []
    for ray, exact in zip(rays, proof):
        slot, primitive, u, v = ray["record"]
        mesh, _ = source.placements[slot - 1]
        _, vertices, indices = source.meshes[mesh][0]
        nearer = next(hit for hit in exact["intersections"] if hit["primitive"] == exact["exact_closest"])
        good = nearer["primitive"]
        def triangle(identity):
            return "array<vec3<f32>,3>(" + ",".join(vector(p[:3]) for p in vertices[indices[identity]]) + ")"
        cases.append(f"""case {ray['pose']}u: {{
            origin={vector(ray['origin_bits'], True)}; direction={vector(ray['direction_bits'], True)};
            good={good}u; record=vec4<u32>({slot}u,{primitive}u,{u}u,{v}u); positions={triangle(primitive)};
            if index%3u==2u {{record=vec4<u32>({slot}u,{good}u,{bits(nearer['barycentrics'][1])}u,{bits(nearer['barycentrics'][2])}u);positions={triangle(good)};}}
        }}"""
        )
    module = args.module.read_text()
    start = module.index("@compute @workgroup_size(8,4) fn film_footprint(")
    module = module[:start] + """
@compute @workgroup_size(8,4) fn film_footprint(@builtin(global_invocation_id) id:vec3<u32>) {
    let grid=footprint_params.x;if any(id.xy>=vec2<u32>(grid)) {return;}
    let index=footprint_params.y;let witness=footprint_targets[index];
    film_rotation=0u;film_sample=vec2<f32>(0.5);film_footprint_seed=1u+id.x+id.y*grid;
    var origin=vec3<f32>(0.0);var direction=vec3<f32>(0.0);var good=0u;
    var record=vec4<u32>(0u);var positions=array<vec3<f32>,3>();
    let pose=select(select(2u,1u,witness.pixel.x==1119u),0u,witness.pixel.x==1347u);
    switch pose {
""" + "\n".join(cases) + """
        default: {return;}
    }
    let bary=bitcast<vec2<f32>>(record.zw);let weights=vec3<f32>(1.0-bary.x-bary.y,bary);
    let point=world_position(receiver_point(positions[0],positions[1],positions[2],weights),record.x-1u).xyz;
    var trace=PrimaryTrace(record,point,positions);
    if index%3u!=0u {trace=primary_repair(trace,origin,direction);}
    let receiver=primary_receiver(trace.point,trace.positions,record.x-1u,vec2<i32>(witness.pixel));
    var visible=0u;
    for(var lane=0u;lane<8u;lane+=1u) {let v=fresh_estimate(receiver,witness.pixel,witness.size.x,lane);visible+=select(v,4u,v==FRESH_SKIP);}
    atomicAdd(&footprint_sums[index].accepted,1u);atomicAdd(&footprint_sums[index].visible,visible);
    if visible<32u {atomicAdd(&footprint_sums[index].opposite,1u);}
    if visible==0u {atomicAdd(&footprint_sums[index].corner,1u);}
    if trace.record.y==good {atomicAdd(&footprint_sums[index].edge,1u);} else {atomicAdd(&footprint_sums[index].interior,1u);}
}
"""
    args.out.write_text(module)


if __name__ == "__main__":
    main()
