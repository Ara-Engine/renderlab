// ---- measurement: the cache's software traversal against the reflection pass's (RENDERLAB_GI_BVH_BENCH) ----
// Composed after gi_rays_software.wgsl and the reflection pass's BVH4 trace (rs_trace, over
// rl_geometry at binding 39) only while the switch names a ray count (src/gi_cache.rs). Both
// kernels trace the same rays: ray i leaves the query point of the first valid traced cell at or
// after a slot hashed from i, in a cosine-weighted direction hashed from i, closest hit, no
// distance limit. Each kernel counts its hits (work words 224 and 225), so the two structures
// can be checked to agree; the pass log's marks gi_bench_cache and gi_bench_rs give their GPU time.
@group(0) @binding(39) var<storage,read> rl_geometry:array<vec4<u32>>;
const GI_BENCH_RAYS:u32=1048576u;
const GI_W_BENCH:u32=224u;
fn gi_bench(first:u32, stride:u32, rs:bool) {
    let mask=gi_word(GI_H_MASK);
    var hits=0u;
    for(var i=first;i<GI_BENCH_RAYS;i+=stride) {
        var slot=gi_mix(i*0x9e3779b9u+1u)&mask;
        var found=false;
        for(var t=0u;t<8u;t+=1u) {
            if (gi_table[gi_hot(slot)]&GI_VALID)!=0u && (gi_table[gi_cold(slot)+GI_C_STATE]&GI_TRACED)!=0u { found=true; break; }
            slot=(slot+97u)&mask;
        }
        if !found { continue; }
        let cold=gi_cold(slot);
        let n=gi_unpack_normal(gi_table[cold+GI_C_NORMAL]);
        let q=gi_point(cold);
        let origin=q+n*gi_origin_offset(q,gi_cell(slot).edge);
        let h1=gi_mix(i^0x68bc21ebu);
        let h2=gi_mix(h1^0x2c1b3c6du);
        let u1=f32(h1>>8u)*(1.0/16777216.0);
        let u2=f32(h2>>8u)*(1.0/16777216.0);
        let r=sqrt(u1);
        let phi=6.283185307179586*u2;
        let d=gi_basis(n)*vec3<f32>(r*cos(phi),r*sin(phi),sqrt(max(1.0-u1,0.0)));
        if rs {
            if rs_trace(origin,d,0.0,1e20,false).record.x!=0u { hits+=1u; }
        } else {
            if gi_closest(origin,d,0.0,1e20).hit { hits+=1u; }
        }
    }
    atomicAdd(&gi_work[GI_W_BENCH+select(0u,1u,rs)],hits);
}
@compute @workgroup_size(64) fn gi_bench_cache(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) { gi_bench(id.x,groups.x*64u,false); }
@compute @workgroup_size(64) fn gi_bench_rs(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) { gi_bench(id.x,groups.x*64u,true); }
