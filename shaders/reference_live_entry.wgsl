// ---- generated live reference material entry (docs/design-reference-live.md), after the program ----
// One thread per queued path hit. The hit's SurfaceInput is rebuilt from the engine's own buffers
// as the cell entry builds a cell's (gi_shade_entry.wgsl), with the path ray's origin as the eye and
// no footprint: the reference integrates each pixel by jitter and samples textures at level 0, as
// src/reference.rs does. Then the program's own `surface` runs, and the hit's material record is
// stored: `material_out` for programs with the record mapping, else the engine's material factors
// and textures (src/program.rs rf_record, the view modes' fallback). Emission is the program's own
// (`rl_emission`, zero without `emission`). Each module stores only the hits whose draw runs its
// program (rf_job.y); the shading kernel (shaders/reference_live.wgsl) reads every record.
// Must match src/reference_live.rs RfHit and RfRecord.
struct RfHit { placement:u32, primitive:u32, bary:vec2<f32>, origin:vec3<f32>, flags:u32 };
// geometric.w: the IOR of a see-through material (0: opaque).
struct RfRecord { normal_alpha:vec4<f32>, base_metal:vec4<f32>, emission_rough:vec4<f32>, world_model:vec4<f32>, geometric:vec4<f32> };
// rf_hits[i].flags: the hit is valid (a miss or a finished path stores none).
const RF_HIT:u32=1u;
@group(0) @binding(90) var<storage,read> rf_hits:array<RfHit>;
@group(0) @binding(91) var<storage,read_write> rf_records:array<RfRecord>;
// x: queued hits; y: this module's program id.
@group(0) @binding(92) var<uniform> rf_job:vec4<u32>;
// Each draw's (batch's) program id.
@group(0) @binding(93) var<storage,read> rf_programs:array<u32>;

fn rf_input(batch:u32,index:u32,slot:u32)->VertexInput {
    let a=primary_vertices[batch].data[index*3u];let b=primary_vertices[batch].data[index*3u+1u];
    let c=primary_vertices[batch].data[index*3u+2u];
    return VertexInput(a.xyz,vec3<f32>(a.w,b.xy),b.zw,c,slot);
}
// The cell entry's camera-free vertex (gi_shade_entry.wgsl gs_vertex): world attributes only.
fn rf_vertex(tangent:bool,batch:u32,input:VertexInput,index:u32)->VertexOutput {
    let nt=normals[input.slot];
    var o:VertexOutput;
    let world=world_position(input.position,input.slot);
    o.position=vec4<f32>(0.0);
    o.world=world.xyz;
    o.receiver_vertex=world.xyz;
    o.color=input.color;
    o.uv=input.uv;
    o.normal=unit_normal(mat3x3<f32>(nt.x.xyz,nt.y.xyz,nt.z.xyz)*unit_normal(input.normal));
    o.winding=nt.info.x;
    o.tangent=vec3<f32>(0.0);
    o.handedness=0.0;
    if tangent {
        let t4=primary_tangents[batch].data[index];
        let m=models[input.slot];
        let a=mat3x3<f32>(m[0].xyz,m[1].xyz,m[2].xyz);
        let scale=max(max(max(abs(m[0].x),abs(m[0].y)),max(abs(m[0].z),abs(m[1].x))),max(max(abs(m[1].y),abs(m[1].z)),max(max(abs(m[2].x),abs(m[2].y)),abs(m[2].z))));
        let t=unit_normal((a*(1.0/max(scale,1e-30)))*t4.xyz);
        o.tangent=unit_normal(t-o.normal*dot(o.normal,t));
        o.handedness=t4.w*o.winding;
    }
    return o;
}
// The hit's receiver, seen from the ray origin. The side the ray arrives on stands in for the
// camera's: a hit on the source-back side mirrors the frame and is source_front, so a
// single-sided material reflects toward the ray as from its front (the cell entry's rule, and
// src/reference.rs shades back faces alike); the trace has already culled the source-back faces
// of single-sided draws (shaders/reference_live.wgsl, as primary_trace.wgsl).
fn rf_surface_input(hit:RfHit)->SurfaceInput {
    let p=hit.placement;
    let info=primary_placements[p];
    let batch=info&0x1fffffffu;
    let tangent=(info>>31u)!=0u;
    let i=hit.primitive*3u;
    let ia=primary_indices[batch].data[i];let ib=primary_indices[batch].data[i+1u];let ic=primary_indices[batch].data[i+2u];
    let ta=rf_input(batch,ia,p);let tb=rf_input(batch,ib,p);let tc=rf_input(batch,ic,p);
    let a=rf_vertex(tangent,batch,ta,ia);let b=rf_vertex(tangent,batch,tb,ib);let c=rf_vertex(tangent,batch,tc,ic);
    let w=vec3<f32>(1.0-hit.bary.x-hit.bary.y,hit.bary);
    let q=world_position(receiver_point(ta.position,tb.position,tc.position,w),p).xyz;
    let geometric=primary_geometric(ta.position,tb.position,tc.position,p);
    let s=select(-1.0,1.0,dot(geometric,hit.origin-q)>=0.0);
    let material=primary_materials[batch].value;
    let receiver=primary_receiver(q,array<vec3<f32>,3>(ta.position,tb.position,tc.position),p,vec2<i32>(-1));
    let zero=vec3<f32>(0.0);
    return SurfaceInput(q,zero,zero,s*geometric,s*(a.normal*w.x+b.normal*w.y+c.normal*w.z),zero,zero,
        a.tangent*w.x+b.tangent*w.y+c.tangent*w.z,s*a.handedness,a.uv*w.x+b.uv*w.y+c.uv*w.z,vec2<f32>(0.0),vec2<f32>(0.0),
        a.color*w.x+b.color*w.y+c.color*w.z,true,hit.origin,vec2<u32>(0u),material,receiver,decal_none());
}
@compute @workgroup_size(64) fn rf_materials(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    for(var i=id.x;i<rf_job.x;i+=groups.x*64u) {
        let hit=rf_hits[i];
        if (hit.flags&RF_HIT)==0u {continue;}
        let batch=primary_placements[hit.placement]&0x1fffffffu;
        if rf_programs[batch]!=rf_job.y {continue;}
        let input=decal_prepare(rf_surface_input(hit));
        rl_load_parameters(batch);
        let s=surface(input);
        let m=rf_record(s,input);
        let e=rl_emission(s,input);
        // geometric.w: a see-through material's IOR (TP1: 1 a blended film, above 1 glass), 0 when opaque.
        let ior=select(0.0,transparent_ior(input.material),transparent_material(input.material));
        rf_records[i]=RfRecord(vec4<f32>(m.normal,m.alpha),vec4<f32>(m.base,m.metallic),vec4<f32>(e,m.roughness),
            vec4<f32>(input.world,bitcast<f32>(m.model)),vec4<f32>(input.geometric,ior));
    }
}
