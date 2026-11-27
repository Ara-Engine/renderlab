// ---- renderlab world radiance cache: emitters as lights (docs/design-gi-emitters.md) ----
// Composed after gi_cache.wgsl (src/gi_cache.rs kernel_source), whose gathers call it.
// A glowing surface reached a cell only when one of the cell's fixed directions happened to meet
// it, each standing for 2 pi / K sr: a thin bright strip (the hangar's ceiling lights) was met by
// some cells' directions and missed by their neighbours', a fixed mottle a cell wide wherever
// emitters light the scene. Emitters are therefore sampled as lights:
// - gi_emitters_build lists every triangle of a draw whose program emits (cell shading's probe,
//   gi_shade_entry.wgsl gs_draw_emission: one radiance per draw, as GI takes emission), in world
//   space, in the table after the slots, and the emitting draws after the triangles;
// - each gather adds every emitter's light to the cell's L1 SH in closed form: the triangle,
//   clipped to the cell's hemisphere, as a uniform polygon of its radiance (L0 its solid angle,
//   L1 its vector form factor), times its visibility from the cell along one ray to a fixed point
//   of it (the same point every frame for a cell, W1);
// - a gather ray whose hit is on an emitting draw adds the hit's reflected light only
//   (gi_hit_radiance subtracts the draw's emission), so no emitter is counted twice.
// Shading and the final gather read the same list through gi_word (the table): one list and one
// rule for every reader.
// GI_H_EMITTERS (57), GI_H_EMITTER_COUNT (58) and GI_EMITTER_WORDS: gi_cells.wgsl.
const GI_H_EMITTER_DRAWS:u32=59u;
const GI_H_EMITTERS_MAX:u32=60u;
// Maximum listed emitter triangle count for which the cache's own gathers evaluate emitters
// analytically (default 64; RENDERLAB_GI_EMITTERS=1 sets u32::MAX, =0 sets 0). Scenes above the
// limit fall back to stochastic bounce hits in the cache while final gather still uses NEE.
const GI_H_EMITTERS_LIMIT:u32=43u;
fn gi_cache_emitters_on() -> bool {
    let n=gi_word(GI_H_EMITTER_COUNT);
    return n>0u && n<=gi_word(GI_H_EMITTERS_LIMIT);
}
// Per emitting draw after the triangles: its stable id, its batch index, its probe radiance (the
// brightest of three sampled triangles: whether it emits, never an estimate), the index of its
// first listed triangle, its triangle count and its first placement (src/gi_cache.rs
// EMITTER_DRAW_WORDS).
const GI_EMITTER_DRAWS_MAX:u32=64u;
const GI_EMITTER_DRAW_WORDS:u32=8u;
const GI_W_EMITTERS:u32=200u;
const GI_W_EMITTER_DRAWS:u32=201u;
const GI_W_EMITTER_RAYS:u32=202u;
const GI_W_EMITTERS_DROPPED:u32=203u;
// Listed triangles whose mean cell shading had not evaluated (the draw's probe stands in), and
// listed triangles whose radiance is zero although their draw's probe emits.
const GI_W_EMITTERS_UNEVALUATED:u32=208u;
const GI_W_EMITTERS_ZERO:u32=209u;

fn gi_emitter_draws_at() -> u32 { return gi_word(GI_H_EMITTERS)+gi_word(GI_H_EMITTERS_MAX)*GI_EMITTER_WORDS; }
// After the draws: per listed triangle, the running sum of the triangles' power up to and
// including it (f32), for readers that pick emitters in proportion to power (gi_emitter_pick).
fn gi_emitter_cdf_at() -> u32 { return gi_emitter_draws_at()+GI_EMITTER_DRAWS_MAX*GI_EMITTER_DRAW_WORDS; }
fn gi_emitter_vec(at:u32) -> vec3<f32> {
    return vec3<f32>(bitcast<f32>(gi_word(at)),bitcast<f32>(gi_word(at+1u)),bitcast<f32>(gi_word(at+2u)));
}

// One thread per (placement, triangle) of the emitting draws, every frame the cache gathers: the
// list follows moving emitters and draw changes in the frame they happen. Each draw's triangles
// take the places after the previous emitting draws' (a running base every thread computes the
// same), so the order does not depend on thread timing. A triangle's radiance is its own mean
// emission, which cell shading evaluated last frame (gi_emitter_mean_at); a triangle not yet
// evaluated (a new list place) takes its draw's probe for that frame.
@compute @workgroup_size(64) fn gi_emitters_build(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let at=gi_word(GI_H_EMITTERS);
    let most=gi_word(GI_H_EMITTERS_MAX);
    let stride=groups.x*64u;
    let slots=gi_word(GI_H_MASK)+1u;
    let draws=gi_draws[PRIMARY_BATCHES].x;
    var listed=0u;
    for(var b=0u;b<draws;b+=1u) {
        // Cell shading's probe of the draw's emission (written after the slots' radiance).
        let e=gi_outgoing[slots+b].rgb;
        if !(max(e.x,max(e.y,e.z))>0.0) { continue; }
        let d=gi_draws[b];
        if id.x==0u {
            let k=atomicAdd(&gi_work[GI_W_EMITTER_DRAWS],1u);
            if k<GI_EMITTER_DRAWS_MAX {
                let w=gi_emitter_draws_at()+k*GI_EMITTER_DRAW_WORDS;
                gi_table[w]=d.x;
                gi_table[w+1u]=b;
                gi_table[w+2u]=bitcast<u32>(e.x);gi_table[w+3u]=bitcast<u32>(e.y);gi_table[w+4u]=bitcast<u32>(e.z);
                gi_table[w+5u]=listed;
                gi_table[w+6u]=batch_triangles(primary_batch_info(b));
                gi_table[w+7u]=d.y;
            }
        }
        let triangles=batch_triangles(primary_batch_info(b));
        let total=d.z*triangles;
        for(var i=id.x;i<total;i+=stride) {
            let p=d.y+i/triangles;
            let t=i%triangles;
            let ia=gi_indices[b].data[t*3u];let ib=gi_indices[b].data[t*3u+1u];let ic=gi_indices[b].data[t*3u+2u];
            let a=world_position(gi_vertices[b].data[ia*3u].xyz,p).xyz;
            var e1=world_position(gi_vertices[b].data[ib*3u].xyz,p).xyz-a;
            var e2=world_position(gi_vertices[b].data[ic*3u].xyz,p).xyz-a;
            // Preserve true source-front outward winding under negative-determinant (mirrored)
            // placements so cross(e1, e2) always points along the mesh's front face.
            if normals[p].info.x<0.0 {
                let tmp=e1;e1=e2;e2=tmp;
            }
            // A degenerate triangle stays listed: its solid angle and power are zero.
            let k=listed+i;
            if k>=most { continue; }
            let r=at+k*GI_EMITTER_WORDS;
            gi_table[r]=bitcast<u32>(a.x);gi_table[r+1u]=bitcast<u32>(a.y);gi_table[r+2u]=bitcast<u32>(a.z);
            gi_table[r+3u]=bitcast<u32>(e1.x);gi_table[r+4u]=bitcast<u32>(e1.y);gi_table[r+5u]=bitcast<u32>(e1.z);
            gi_table[r+6u]=bitcast<u32>(e2.x);gi_table[r+7u]=bitcast<u32>(e2.y);gi_table[r+8u]=bitcast<u32>(e2.z);
            let mean=gi_outgoing[gi_emitter_mean_at(k)];
            let radiance=select(e,mean.rgb,mean.w>0.0);
            if !(mean.w>0.0) { atomicAdd(&gi_work[GI_W_EMITTERS_UNEVALUATED],1u); }
            if !(max(radiance.x,max(radiance.y,radiance.z))>0.0) { atomicAdd(&gi_work[GI_W_EMITTERS_ZERO],1u); }
            gi_table[r+9u]=bitcast<u32>(radiance.x);gi_table[r+10u]=bitcast<u32>(radiance.y);gi_table[r+11u]=bitcast<u32>(radiance.z);
            gi_table[r+12u]=b;gi_table[r+13u]=p;gi_table[r+14u]=t;
            // Cache each triangle's power directly into its CDF slot while e1, e2, radiance are
            // in registers, so gi_emitters_cdf scans a single contiguous array instead of 9-word strides.
            let area=0.5*length(cross(e1,e2));
            let power=max(dot(radiance,vec3<f32>(0.2126,0.7152,0.0722)),0.0)*3.141592653589793*area;
            gi_table[gi_emitter_cdf_at()+k]=bitcast<u32>(power);
        }
        listed+=total;
    }
    if id.x==0u {
        atomicStore(&gi_work[GI_W_EMITTERS],min(listed,most));
        atomicStore(&gi_work[GI_W_EMITTERS_DROPPED],listed-min(listed,most));
    }
}
// The counts into the header for this frame's readers (the CPU's header write leaves these two
// words alone, so they hold on frames the cache does not rebuild the list).
@compute @workgroup_size(1) fn gi_emitters_publish() {
    gi_table[GI_H_EMITTER_COUNT]=min(atomicLoad(&gi_work[GI_W_EMITTERS]),gi_word(GI_H_EMITTERS_MAX));
    gi_table[GI_H_EMITTER_DRAWS]=min(atomicLoad(&gi_work[GI_W_EMITTER_DRAWS]),GI_EMITTER_DRAWS_MAX);
}

// The mean emission of triangle `primitive` of placement `placement` of the draw at batch `batch`,
// when that draw is listed, else zero: what a hit on it should take away when its emitter is
// sampled. Readers that know the hit's triangle (hardware rays, the final gather's records).
fn gi_triangle_emission(batch:u32, placement:u32, primitive:u32) -> vec3<f32> {
    let n=gi_word(GI_H_EMITTER_DRAWS);
    let base=gi_emitter_draws_at();
    for(var j=0u;j<n;j+=1u) {
        let w=base+j*GI_EMITTER_DRAW_WORDS;
        if gi_word(w+1u)!=batch { continue; }
        let k=gi_word(w+5u)+(placement-gi_word(w+7u))*gi_word(w+6u)+primitive;
        if placement<gi_word(w+7u) || k>=gi_word(GI_H_EMITTER_COUNT) { return vec3<f32>(0.0); }
        return gi_emitter_vec(gi_word(GI_H_EMITTERS)+k*GI_EMITTER_WORDS+9u);
    }
    return vec3<f32>(0.0);
}
// Listed triangle k's emitted power (W): its radiance's luminance times pi times its area.
fn gi_emitter_power(k:u32) -> f32 {
    let r=gi_word(GI_H_EMITTERS)+k*GI_EMITTER_WORDS;
    let area=0.5*length(cross(gi_emitter_vec(r+3u),gi_emitter_vec(r+6u)));
    return max(dot(gi_emitter_vec(r+9u),vec3<f32>(0.2126,0.7152,0.0722)),0.0)*3.141592653589793*area;
}
const GI_EMITTER_CDF_LANES:u32=256u;
var<workgroup> gi_wg_emitter_sums:array<f32,256>;
// The power CDF (gi_emitter_cdf_at): 256 lanes each compute their chunk's powers once (caching
// each triangle's power in the CDF slot so pass 2 avoids re-reading 9 strided geometry words),
// then prefix-sum the 256 chunk totals and write the running sums in parallel.
@compute @workgroup_size(256) fn gi_emitters_cdf(@builtin(local_invocation_index) lane:u32) {
    let n=gi_word(GI_H_EMITTER_COUNT);
    let chunk=(n+GI_EMITTER_CDF_LANES-1u)/GI_EMITTER_CDF_LANES;
    let first=min(lane*chunk,n);
    let last=min(first+chunk,n);
    let at=gi_emitter_cdf_at();
    var sum=0.0;
    for(var k=first;k<last;k+=1u) { sum+=bitcast<f32>(gi_table[at+k]); }
    gi_wg_emitter_sums[lane]=sum;
    workgroupBarrier();
    if lane==0u {
        var run=0.0;
        for(var i=0u;i<GI_EMITTER_CDF_LANES;i+=1u) { let s=gi_wg_emitter_sums[i]; gi_wg_emitter_sums[i]=run; run+=s; }
    }
    workgroupBarrier();
    var run=gi_wg_emitter_sums[lane];
    for(var k=first;k<last;k+=1u) { run+=bitcast<f32>(gi_table[at+k]); gi_table[at+k]=bitcast<u32>(run); }
}
// An emitter picked in proportion to power by u in [0, 1): its index and the probability it was
// picked with (its power over the total), or probability 0 when nothing emits.
struct GiEmitterPick { index:u32, probability:f32 };
fn gi_emitter_pick(u:f32) -> GiEmitterPick {
    let n=gi_word(GI_H_EMITTER_COUNT);
    if n==0u { return GiEmitterPick(0u,0.0); }
    let at=gi_emitter_cdf_at();
    let total=bitcast<f32>(gi_word(at+n-1u));
    if !(total>0.0) { return GiEmitterPick(0u,0.0); }
    let goal=u*total;
    var lo=0u; var hi=n-1u;
    while lo<hi { let mid=(lo+hi)/2u; if bitcast<f32>(gi_word(at+mid))<=goal { lo=mid+1u; } else { hi=mid; } }
    let below=select(0.0,bitcast<f32>(gi_word(at+lo-1u)),lo>0u);
    return GiEmitterPick(lo,(bitcast<f32>(gi_word(at+lo))-below)/total);
}
// The emitted radiance of the draw a material identity + 1 names (gi_hit_identity), or zero.
fn gi_identity_emission(identity:u32) -> vec3<f32> {
    let n=gi_word(GI_H_EMITTER_DRAWS);
    if identity==0u || n==0u || !gi_cache_emitters_on() { return vec3<f32>(0.0); }
    let id=(identity-1u)>>16u;
    let base=gi_emitter_draws_at();
    for(var k=0u;k<n;k+=1u) {
        let w=base+k*GI_EMITTER_DRAW_WORDS;
        if gi_word(w)==id { return gi_emitter_vec(w+2u); }
    }
    return vec3<f32>(0.0);
}
// The emitted radiance of the draw at batch index `batch` (primary_decode(record).batch), or zero:
// for readers bound to the table but not to the cache's draws and geometry (the final gather).
fn gi_batch_emission(batch:u32) -> vec3<f32> {
    let n=gi_word(GI_H_EMITTER_DRAWS);
    if n==0u { return vec3<f32>(0.0); }
    let base=gi_emitter_draws_at();
    for(var k=0u;k<n;k+=1u) {
        let w=base+k*GI_EMITTER_DRAW_WORDS;
        if gi_word(w+1u)==batch { return gi_emitter_vec(w+2u); }
    }
    return vec3<f32>(0.0);
}

// The L1 SH, in gi_sh_basis's order and constants, of a uniform polygon of unit radiance seen
// from q, clipped to the hemisphere about n: x its solid angle (Van Oosterom and Strackee, over
// a fan) times Y00, yzw its vector form factor (Lambert: half the sum over its edges of the
// edge's angle times the unit normal of the edge's plane through q) times the Y1 constant.
// Exact for any visible emitter, whatever its size or distance.
fn gi_emitter_sh(q:vec3<f32>, n:vec3<f32>, a:vec3<f32>, b:vec3<f32>, c:vec3<f32>) -> vec4<f32> {
    let v0=a-q;let v1=b-q;let v2=c-q;
    let h0=dot(v0,n);let h1=dot(v1,n);let h2=dot(v2,n);
    if h0<=0.0 && h1<=0.0 && h2<=0.0 { return vec4<f32>(0.0); }
    // Sutherland-Hodgman against the cell's plane: at most four corners.
    var p:array<vec3<f32>,4>;
    var m=0u;
    if h0>0.0 { p[m]=v0; m+=1u; }
    if (h0>0.0)!=(h1>0.0) { p[m]=v0+(v1-v0)*(h0/(h0-h1)); m+=1u; }
    if h1>0.0 { p[m]=v1; m+=1u; }
    if (h1>0.0)!=(h2>0.0) { p[m]=v1+(v2-v1)*(h1/(h1-h2)); m+=1u; }
    if h2>0.0 { p[m]=v2; m+=1u; }
    if (h2>0.0)!=(h0>0.0) { p[m]=v2+(v0-v2)*(h2/(h2-h0)); m+=1u; }
    if m<3u { return vec4<f32>(0.0); }
    var f=vec3<f32>(0.0);
    var omega=0.0;
    var centre=vec3<f32>(0.0);
    let u0=normalize(p[0]);
    for(var i=0u;i<m;i+=1u) {
        let ui=normalize(p[i]);
        let uj=normalize(p[(i+1u)%m]);
        centre+=ui;
        let edge=cross(ui,uj);
        let l=length(edge);
        if l>1e-12 { f+=edge*(atan2(l,dot(ui,uj))/l); }
        if i>0u && i+1u<m {
            omega+=2.0*atan2(dot(u0,edge),1.0+dot(u0,ui)+dot(ui,uj)+dot(uj,u0));
        }
    }
    // The winding sets both signs; the polygon lies in front of q.
    omega=abs(omega);
    f*=0.5;
    if dot(f,centre)<0.0 { f=-f; }
    return vec4<f32>(0.282095*omega,0.488603*f.y,0.488603*f.z,0.488603*f.x);
}

// One lane's share of a cell's emitters (emitter e on lane e % GI_GATHER_LANES), added to
// gi_gather_sh. Returns the visibility rays traced.
fn gi_emitter_faces(r:u32, a:vec3<f32>, e1:vec3<f32>, e2:vec3<f32>, q:vec3<f32>) -> bool {
    let batch=gi_word(r+12u);
    let place=gi_word(r+13u);
    let tris=batch_triangles(primary_batch_info(batch));
    let flags=primary_placements[place];
    let single_sided=(flags&0x40000000u)!=0u;
    let thin=(flags&0x20000000u)!=0u;
    if !single_sided && (thin || tris<=2u) { return true; }
    return dot(cross(e1,e2),q-a)>0.0;
}
fn gi_emitter_gather(lane:u32, q:vec3<f32>, origin:vec3<f32>, n:vec3<f32>, key:vec2<u32>) -> u32 {
    let count=gi_word(GI_H_EMITTER_COUNT);
    if !gi_cache_emitters_on() { return 0u; }
    let base=gi_word(GI_H_EMITTERS);
    var rays=0u;
    for(var e=lane;e<count;e+=GI_GATHER_LANES) {
        let r=base+e*GI_EMITTER_WORDS;
        let a=gi_emitter_vec(r);
        let e1=gi_emitter_vec(r+3u);
        let e2=gi_emitter_vec(r+6u);
        let radiance=gi_emitter_vec(r+9u);
        if all(radiance<=vec3<f32>(0.0)) { continue; }
        if !gi_emitter_faces(r,a,e1,e2,q) { continue; }
        let sh=gi_emitter_sh(q,n,a,a+e1,a+e2);
        // Below a microsteradian an emitter adds nothing a cell can show.
        if sh.x<=2.8e-7 { continue; }
        let f_vec=vec3<f32>(sh.w,sh.y,sh.z)*(1.0/0.488603);
        let irrad=max(dot(f_vec,n),0.0);
        if irrad<=1e-7 { continue; }
        // Visibility along one ray to a fixed point of the triangle, from the cell's own hash
        // (the same point every frame for this cell); a point under the cell's horizon moves to
        // the triangle's centroid, or its highest vertex above the horizon when clipped across it.
        let hash=gi_mix(key.y^(e*0x9e3779b9u));
        var uv=vec2<f32>(f32(hash&0xffffu),f32(hash>>16u))*(1.0/65536.0);
        if uv.x+uv.y>1.0 { uv=vec2<f32>(1.0)-uv; }
        var x=a+e1*uv.x+e2*uv.y;
        if dot(x-q,n)<=1e-5 {
            x=a+(e1+e2)*(1.0/3.0);
            if dot(x-q,n)<=1e-5 {
                let h0=dot(a-q,n);let h1=dot(a+e1-q,n);let h2=dot(a+e2-q,n);
                x=select(select(a+e2,a+e1,h1>=h2),a,h0>=h1 && h0>=h2);
            }
        }
        let to=x-origin;
        let dist=length(to);
        if dist<=1e-6 { continue; }
        rays+=1u;
        if !gi_visible_within(origin,to/dist,dist*0.999) { continue; }
        let e_dir=radiance*irrad;
        let f_unit=select(n,normalize(f_vec),dot(f_vec,f_vec)>1e-12);
        let dir_l1=mix(n,f_unit,clamp(dot(f_unit,n)*2.0,0.0,1.0));
        let f_eff=dir_l1/max(dot(dir_l1,n),0.5);
        gi_gather_sh[0]+=e_dir*0.56418958;
        gi_gather_sh[1]+=e_dir*(0.48860251*f_eff.y);
        gi_gather_sh[2]+=e_dir*(0.48860251*f_eff.z);
        gi_gather_sh[3]+=e_dir*(0.48860251*f_eff.x);
    }
    if rays!=0u { atomicAdd(&gi_work[GI_W_EMITTER_RAYS],rays); }
    return rays;
}
