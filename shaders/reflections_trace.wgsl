// ---- U-RTVIS rl_trace (docs/design-reflection-pass.md) ----
// Appended to a traced module of the standard program (src/reflection_pass.rs); it uses the
// reflection block's helpers (rl_vndf, rl_basis, rl_hash, rl_sky, rl_cast, rl_oct_*) and no
// program code. One invocation per slot, a 2x2 quad of the image: it picks one glossy pixel of
// the quad whose lobe this frame wrote (L), rotating with the frame, and one stratum of that
// pixel's fixed set, rotating with the frame from a per-pixel offset; traces the ray; writes a
// miss's sky radiance into R and appends a hit's record to H. Every slot clears its R texel
// first, so reflection_hits writes only real hits and record order never changes a result.
@group(0) @binding(56) var rl_lobe_a:texture_2d<f32>;
@group(0) @binding(57) var rl_lobe_b:texture_2d<u32>;
@group(0) @binding(58) var<uniform> rl_pass_frame:vec4<u32>;
@group(0) @binding(59) var rl_ray_distance:texture_storage_2d<r32float,write>;
// H: count, the three dispatch arguments of reflection_hits (RL_HIT_GROUP-wide groups), then
// 48-byte records (three vec4<u32>).
struct RlHits { count:atomic<u32>, args:array<u32,3>, records:array<vec4<u32>> };
@group(0) @binding(64) var<storage,read_write> rl_hits:RlHits;
@group(0) @binding(65) var rl_samples:texture_storage_2d<rgba16float,write>;
const RL_HIT_GROUP:u32=64u;
var<workgroup> rl_group_count:atomic<u32>;
var<workgroup> rl_group_base:u32;

@compute @workgroup_size(8,8) fn rl_trace(@builtin(global_invocation_id) id:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    if lane==0u {atomicStore(&rl_group_count,0u);}
    workgroupBarrier();
    let slots=textureDimensions(rl_samples);
    let size=textureDimensions(rl_lobe_b);
    let inside=all(id.xy<slots);
    var record=vec4<u32>(0u);var origin=vec3<f32>(0.0);var d=vec3<f32>(0.0);var cone=vec2<f32>(0.0);
    var pixel=vec2<u32>(0u);var stratum=0u;var strata=0u;var hit=false;var lobe_alpha=0.0;
    if inside {
        textureStore(rl_samples,vec2<i32>(id.xy),vec4<f32>(0.0));
        textureStore(rl_ray_distance,vec2<i32>(id.xy),vec4<f32>(-2.0));
        let tag=f32(rl_pass_frame.x%2048u);
        let start=(rl_pass_frame.x+rl_hash(id.x*0x9e3779b1u^rl_hash(id.y)))&3u;
        var found=false;
        for(var i=0u;i<4u && !found;i+=1u) {
            let k=(start+i)&3u;
            let p=id.xy*2u+vec2<u32>(k&1u,k>>1u);
            if any(p>=size) {continue;}
            let b=textureLoad(rl_lobe_b,p,0);
            let at=unpack2x16float(b.w);
            if at.y!=tag || !(at.x>0.0) {continue;}
            found=true;pixel=p;
            let a=textureLoad(rl_lobe_a,p,0);
            origin=a.xyz;
            let ws=unpack2x16float(bitcast<u32>(a.w));
            let n=rl_oct_decode(b.x);let v=rl_oct_decode(b.y);let side=rl_oct_decode(b.z);
            let alpha=at.x;let rough=sqrt(alpha);lobe_alpha=alpha;
            strata=clamp(u32(ceil(RL_RAYS_PER_ROUGHNESS*rough)),RL_GLOSSY_RAYS,RL_GLOSSY_MAX);
            stratum=(rl_pass_frame.x+(rl_hash((p.x*0x9e3779b1u)^rl_hash(p.y))&0xffffu))%strata;
            let cone_alpha=alpha*inverseSqrt(f32(strata));
            let spread=ws.y+2.0*cone_alpha;
            cone=vec2<f32>(ws.x,spread);
            let frame=primary_frame.sided.y;
            let seed=rl_hash((p.x*0x9e3779b1u)^rl_hash(p.y^(frame*0x85ebca6bu)));
            let rotation=f32(seed>>8u)*(1.0/16777216.0);
            let radial=select(0.5,fract(f32(frame)*0.6180339887),frame!=0u);
            let basis=rl_basis(n);
            let local_v=vec3<f32>(dot(v,basis[0]),dot(v,basis[1]),dot(v,basis[2]));
            let u=vec2<f32>((f32(stratum)+radial)/f32(strata),fract(rotation+f32(stratum)*0.6180339887));
            d=reflect(-v,basis*rl_vndf(local_v,max(alpha,1e-6),u));
            if dot(d,side)>0.0 {
                let epsilon=max(1e-4*length(origin),1e-5);
                let traced=rl_cast(origin,d,epsilon);
                textureStore(rl_ray_distance,vec2<i32>(id.xy),vec4<f32>(traced.t));
                if traced.record.x!=0u {
                    record=traced.record;hit=true;
                } else {
                    textureStore(rl_samples,vec2<i32>(id.xy),vec4<f32>(rl_sky(d,cone_alpha),1.0));
                }
            }
        }
    }
    // One atomic per workgroup on H's count, so the list keeps the trace tiles' order.
    var index=0u;
    if hit {index=atomicAdd(&rl_group_count,1u);}
    workgroupBarrier();
    if lane==0u {
        let n=atomicLoad(&rl_group_count);
        rl_group_base=select(0u,atomicAdd(&rl_hits.count,n),n>0u);
    }
    workgroupBarrier();
    if hit {
        let at=(workgroupUniformLoad(&rl_group_base)+index)*3u;
        let slot=id.y*slots.x+id.x;
        // Word 2: stratum | strata << 8 | the lobe's GGX alpha as f16 << 16 (strata <= RL_GLOSSY_MAX).
        let word2=stratum|(strata<<8u)|(pack2x16float(vec2<f32>(lobe_alpha,0.0))<<16u);
        rl_hits.records[at]=vec4<u32>(pixel.x|(pixel.y<<16u),slot,word2,record.x);
        rl_hits.records[at+1u]=vec4<u32>(record.y,record.z,record.w,bitcast<u32>(origin.x));
        rl_hits.records[at+2u]=vec4<u32>(bitcast<u32>(origin.y),bitcast<u32>(origin.z),rl_oct_encode(d),pack2x16float(cone));
    }
}
// reflection_hits' indirect arguments from H's count: RL_HIT_GROUP records a group, rows of
// at most 65535 groups (wgpu skips larger dimensions).
@compute @workgroup_size(1) fn rl_hit_args() {
    let groups=(atomicLoad(&rl_hits.count)+RL_HIT_GROUP-1u)/RL_HIT_GROUP;
    rl_hits.args[0]=min(groups,65535u);
    rl_hits.args[1]=(groups+65534u)/65535u;
    rl_hits.args[2]=1u;
}
