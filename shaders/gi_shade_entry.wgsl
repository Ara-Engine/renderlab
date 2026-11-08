// ---- generated cell shading entry (M7 G2), after the program ----
// The bounce light a world radiance cache cell sends back: one thread per cell queued this
// frame, which builds the cell's SurfaceInput at its fixed query point, runs the program's
// own `surface`, and returns one view-independent outgoing radiance: the cell reflects as a
// Lambertian surface whose reflectance is the program's own directional albedo toward the
// cell's normal n,
//   a = min(sum over sixteen fixed directions d of scatter(s, input, d, n) * 2 pi / 16, 1),
// so the radiance is the cosine-weighted mean of what the program sends over the hemisphere
// (its radiosity over pi), which conserves the reflected power:
//   emission(s, input)
//   + a / pi * (sum over lights the cell's visibility ray reached (gi_light) of intensity *
//     attenuation * max(dot(n, l), 0)  +  the cell's incident irradiance: occluded sky plus
//     bounce, from its own SH).
// Exact for Lambert. G2 evaluated the lights as scatter(s, input, l, n), the program's
// radiance toward n alone: a glossy or mirror cell facing the sun then sent its highlight
// (up to 10^4 times a white surface's radiance) in every direction, and the few gather rays
// that met such cells left bright speckles on the ground in API1's game scene
// (docs/m7-g2c.md). The cap at 1 per channel is a passive
// surface's energy bound: it only changes programs whose response integrates above 1, or a
// sharp lobe that one fixed direction meets head-on (a normal-mapped mirror).
// Composed only for programs that declare `scatter` or `emission` (src/program.rs
// Stage::Cell); src/program.rs defines rl_scatter and rl_emission (zero when absent).
// Writes gi_shade_out[slot] (gi_cache.wgsl gi_outgoing), which the next gathers read.
@group(0) @binding(31) var<storage,read> gi_shade_work:array<u32>;
@group(0) @binding(33) var<storage,read_write> gi_shade_out:array<vec4<f32>>;
@group(0) @binding(36) var<storage,read> gi_shade_draws:array<vec4<u32>>;
// gi_cache.wgsl GI_W_SHADES: the queue's length.
const GS_COUNT:u32=32u;
const GS_DIRECTIONS:u32=16u;

fn gs_input(batch:u32,index:u32,slot:u32)->VertexInput {
    let a=primary_vertices[batch].data[index*3u];let b=primary_vertices[batch].data[index*3u+1u];
    let c=primary_vertices[batch].data[index*3u+2u];
    return VertexInput(a.xyz,vec3<f32>(a.w,b.xy),b.zw,c,slot);
}
// vertex_output's attributes (and vs_tangent's tangent) without the camera projection, which
// a cell has no use for: cell shading binds no camera.
fn gs_vertex(tangent:bool,batch:u32,input:VertexInput,index:u32)->VertexOutput {
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
// The placement a material identity names this frame, or 0xffffffff when its draw is absent.
fn gs_placement(identity:u32) -> u32 {
    let id=identity>>16u;
    let inst=identity&0xffffu;
    for(var b=0u;b<PRIMARY_BATCHES;b+=1u) {
        let d=gi_shade_draws[b];
        if d.x==id && inst<d.z { return d.y+inst; }
    }
    return 0xffffffffu;
}
// Footprint gradients on a triangle's plane: an attribute's change along world offsets dx
// and dy, given its change uv1, uv2 along the edges e1, e2 (the traced entry's dual basis).
fn gs_gradients(n:vec3<f32>,e1:vec3<f32>,e2:vec3<f32>,uv1:vec2<f32>,uv2:vec2<f32>,dx:vec3<f32>,dy:vec3<f32>)->mat2x2<f32> {
    let scale=max(max(max(abs(e1.x),abs(e1.y)),abs(e1.z)),max(max(abs(e2.x),abs(e2.y)),abs(e2.z)));
    let a=e1/max(scale,1e-30);let b=e2/max(scale,1e-30);
    let area=dot(cross(a,b),n);
    if abs(area)<1e-20 {return mat2x2<f32>();}
    let u=cross(b,n)/area;let v=cross(n,a)/area;
    let sx=dx/max(scale,1e-30);let sy=dy/max(scale,1e-30);
    return mat2x2<f32>(uv1*dot(u,sx)+uv2*dot(v,sx),uv1*dot(u,sy)+uv2*dot(v,sy));
}
// A cell's SurfaceInput at its query point q, seen from its normal side nc, with a footprint
// of one cell edge (the cell's ray cone, as programs have no derivatives here). With the hit
// triangle known (hardware rays) every attribute is the triangle's; without it (software
// rays) the placement's material is read over its whole texture.
fn gs_surface_input(slot:u32, p:u32, q:vec3<f32>, nc:vec3<f32>, edge:f32) -> SurfaceInput {
    let cold=gi_cold(slot);
    let info=primary_placements[p];
    let batch=info&0x1fffffffu;
    let primitive=gi_word(cold+GI_C_PRIMITIVE);
    let frame=gi_basis(nc);
    let dx=frame[0]*edge;let dy=frame[1]*edge;
    let material=primary_materials[batch].value;
    let eye=q+nc;
    if primitive==GI_NO_PRIMITIVE {
        let receiver=Receiver(q,nc,vec2<i32>(-1),batch,1e-3*edge,(info&0x20000000u)!=0u);
        return SurfaceInput(q,dx,dy,nc,nc,vec3<f32>(0.0),vec3<f32>(0.0),vec3<f32>(0.0),1.0,
            vec2<f32>(0.5),vec2<f32>(1e4,0.0),vec2<f32>(0.0,1e4),vec4<f32>(1.0),true,eye,vec2<u32>(0u),material,receiver,decal_none());
    }
    let tangent=(info>>31u)!=0u;
    let i=primitive*3u;
    let ia=primary_indices[batch].data[i];let ib=primary_indices[batch].data[i+1u];let ic=primary_indices[batch].data[i+2u];
    let ta=gs_input(batch,ia,p);let tb=gs_input(batch,ib,p);let tc=gs_input(batch,ic,p);
    let a=gs_vertex(tangent,batch,ta,ia);let b=gs_vertex(tangent,batch,tb,ib);let c=gs_vertex(tangent,batch,tc,ic);
    let bary=unpack2x16unorm(gi_word(cold+GI_C_BARY));
    let w=vec3<f32>(1.0-bary.x-bary.y,bary);
    let geometric=primary_geometric(ta.position,tb.position,tc.position,p);
    let m=models[p];let linear=mat3x3<f32>(m[0].xyz,m[1].xyz,m[2].xyz);
    let e1=linear*(tb.position-ta.position);let e2=linear*(tc.position-ta.position);
    let uv=gs_gradients(geometric,e1,e2,tb.uv-ta.uv,tc.uv-ta.uv,dx,dy);
    let bw=gs_gradients(geometric,e1,e2,vec2<f32>(1.0,0.0),vec2<f32>(0.0,1.0),dx,dy);
    // The cell's side of the triangle stands in for the camera's. A cell on the source-back
    // side (the cache's rays block on both faces) sees that side as a front: the frame is
    // mirrored and source_front set, so even a single-sided material reflects toward the
    // cell as it would from its front, as the reference path tracer shades back faces.
    // Without this the program's normal faces away from every direction the cell receives
    // light from, and back faces under slabs and overhangs reflect nothing (docs/m7-g2.md).
    let s=select(-1.0,1.0,dot(geometric,nc)>=0.0);
    let normal_dx=s*((b.normal-a.normal)*bw[0].x+(c.normal-a.normal)*bw[0].y);
    let normal_dy=s*((b.normal-a.normal)*bw[1].x+(c.normal-a.normal)*bw[1].y);
    let receiver=primary_receiver(q,array<vec3<f32>,3>(ta.position,tb.position,tc.position),p,vec2<i32>(-1));
    return SurfaceInput(q,dx,dy,s*geometric,s*(a.normal*w.x+b.normal*w.y+c.normal*w.z),normal_dx,normal_dy,
        a.tangent*w.x+b.tangent*w.y+c.tangent*w.z,s*a.handedness,a.uv*w.x+b.uv*w.y+c.uv*w.z,uv[0],uv[1],
        a.color*w.x+b.color*w.y+c.color*w.z,true,eye,vec2<u32>(0u),material,receiver,decal_none());
}
// One cell's outgoing radiance: rgb, w 1 (written), or 0 for a cell without a value.
fn gs_shade(slot:u32) -> vec4<f32> {
    let cold=gi_cold(slot);
    let hot=gi_hot(slot);
    if (gi_word(hot)&GI_VALID)==0u || (gi_word(cold+GI_C_STATE)&GI_ORPHAN)!=0u { return vec4<f32>(0.0); }
    let identity=gi_word(cold+GI_C_MATERIAL);
    // A point whose placement is unknown or not drawn this frame reflects nothing.
    if identity==0u { return vec4<f32>(0.0,0.0,0.0,1.0); }
    let p=gs_placement(identity-1u);
    if p==0xffffffffu { return vec4<f32>(0.0,0.0,0.0,1.0); }
    let q=vec3<f32>(bitcast<f32>(gi_word(cold+GI_C_POINT)),bitcast<f32>(gi_word(cold+GI_C_POINT+1u)),bitcast<f32>(gi_word(cold+GI_C_POINT+2u)));
    let nc=gi_unpack_normal(gi_word(cold+GI_C_NORMAL));
    let edge=gi_edge(gi_word(cold+GI_C_DEF)&0xffu);
    let input=decal_prepare(gs_surface_input(slot,p,q,nc,edge));
    rl_load_parameters(input.receiver.slot);
    let s=surface(input);
    let frame=gi_basis(nc);
    var albedo=vec3<f32>(0.0);
    for(var k=0u;k<GS_DIRECTIONS;k+=1u) {
        let z=1.0-(f32(k)+0.5)/f32(GS_DIRECTIONS);
        let r=sqrt(max(1.0-z*z,0.0));
        let phi=f32(k)*2.399963229728653;
        albedo+=rl_scatter(s,input,frame*vec3<f32>(r*cos(phi),r*sin(phi),z),nc);
    }
    // Nonnegative and finite (NaN fails both comparisons), then the energy bound.
    albedo=select(vec3<f32>(0.0),albedo*(6.283185307179586/f32(GS_DIRECTIONS)),albedo>=vec3<f32>(0.0));
    albedo=min(albedo,vec3<f32>(1.0));
    var incident=gi_sky_scale(gi_cell_sky_irradiance(hot,nc))+gi_cell_bounce_irradiance(hot,nc);
    let bits=gi_word(cold+GI_C_LIGHTS);
    for(var i=0u;i<min(light_count(),32u);i+=1u) {
        if (bits&(1u<<i))==0u { continue; }
        let light=light_sample(i,input);
        incident+=light.intensity*(light.attenuation*max(dot(nc,light.direction),0.0));
    }
    var out=rl_emission(s,input)+albedo*incident*(1.0/PI);
    // Finite and nonnegative, as rl_finite but without the HDR store's 16-bit ceiling.
    out=min(select(vec3<f32>(0.0),out,out>=vec3<f32>(0.0)),vec3<f32>(3.4028235e38));
    return vec4<f32>(out,1.0);
}
@compute @workgroup_size(64) fn gi_shade_cells(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let count=min(gi_shade_work[GS_COUNT],gi_word(GI_H_MASK)+1u);
    let queue=gi_word(GI_H_SHADE_QUEUE);
    for(var i=id.x;i<count;i+=groups.x*64u) {
        let slot=gi_shade_work[queue+i];
        // GC1 (one line, shared with G2c's cell shading): a value whose cell still has gathers
        // scheduled is marked as changing (w 0.75), so gathers that read it repeat (gi_gather).
        gi_shade_out[slot]=gs_shade(slot)*select(vec4<f32>(1.0),vec4<f32>(1.0,1.0,1.0,0.75),(gi_word(gi_cold(slot)+GI_C_STATE)&(GI_GATHER_MASK|GI_BQUEUED))!=0u);
    }
}
