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
        let tag=rl_pass_frame.x%2048u;
        let start=(rl_pass_frame.x+rl_hash(id.x*0x9e3779b1u^rl_hash(id.y)))&3u;
        var found=false;
        for(var i=0u;i<4u && !found;i+=1u) {
            let k=(start+i)&3u;
            let p=id.xy*2u+vec2<u32>(k&1u,k>>1u);
            if any(p>=size) {continue;}
            let b=textureLoad(rl_lobe_b,p,0);
            let at=vec2<f32>(unpack2x16float(b.w&0xffffu).x,0.0);
            if ((b.w>>16u)&0x7ffu)!=tag || !(at.x>0.0) {continue;}
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
// ---- the deferred composite's trace (RENDERLAB_RL_DEFERRED=1): every glossy pixel's full set ----
// One invocation per pixel: a glossy pixel whose lobe this frame wrote takes a contiguous range of
// sample slots for its strata (L's count: today's faded set, directions and cones), records it in
// the range image, clears its slots, writes a miss's sky into its slot and appends a hit's record
// (slot = its slot) for reflection_hits. reflections_composite.wgsl sums the range.
// x glossy pixels, y slots taken, z strata requested (every glossy receiver's, summed at shading:
// rl_room_reserve), w hit records dropped for want of room (their strata stay out of the pixel's
// mean; the probe reads all four).
struct RlTraceStats { pixels:atomic<u32>, slots:atomic<u32>, requested:atomic<u32>, dropped:atomic<u32> };
@group(0) @binding(60) var<storage,read_write> rl_trace_stats:RlTraceStats;
// Per pixel: its first slot << 5 | its slot count (at most 31; 0: no traced reflection).
@group(0) @binding(61) var rl_ranges:texture_storage_2d<r32uint,write>;
// The receivers' ray epsilon per pixel (reflection_pass.rs RECEIVER_EPSILON).
@group(0) @binding(79) var rl_receiver_epsilon:texture_2d<f32>;
var<workgroup> rl_group_slots:atomic<u32>;
var<workgroup> rl_group_first:u32;
// A pixel's strata count from L this frame (0: no traced reflection; a receiver that did not fit
// the room wrote no lobe).
fn rl_pixel_count(p:vec2<u32>)->u32 {
    let b=textureLoad(rl_lobe_b,p,0);
    let alpha=unpack2x16float(b.w&0xffffu).x;
    if ((b.w>>16u)&0x7ffu)!=rl_pass_frame.x%2048u || !(alpha>0.0) {return 0u;}
    return b.w>>27u;
}
@compute @workgroup_size(8,8) fn rl_trace_pixels(@builtin(global_invocation_id) id:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    if lane==0u {atomicStore(&rl_group_slots,0u);}
    workgroupBarrier();
    let size=textureDimensions(rl_lobe_b);
    let inside=all(id.xy<size);
    var count=0u;var b=vec4<u32>(0u);
    let samples=textureDimensions(rl_samples);
    // The room: R's slots and H's records (a hit needs one of each). The lobes shading stored all
    // fit it (rl_room_reserve), so every pixel traces its whole set.
    let capacity=min(samples.x*samples.y,arrayLength(&rl_hits.records)/3u);
    if inside {
        b=textureLoad(rl_lobe_b,id.xy,0);
        count=rl_pixel_count(id.xy);
    }
    // One atomic per workgroup on the frame's slot count.
    let offset=atomicAdd(&rl_group_slots,count);
    workgroupBarrier();
    if lane==0u {
        let n=atomicLoad(&rl_group_slots);
        rl_group_first=select(0u,atomicAdd(&rl_trace_stats.slots,n),n>0u);
    }
    let first=workgroupUniformLoad(&rl_group_first)+offset;
    // A safeguard only: the stored lobes fit the room.
    if count>0u && first+count>capacity {count=select(0u,capacity-first,first<capacity);}
    if !inside {return;}
    textureStore(rl_ranges,vec2<i32>(id.xy),vec4<u32>((first<<5u)|count,0u,0u,0u));
    if count==0u {return;}
    atomicAdd(&rl_trace_stats.pixels,1u);
    let p=id.xy;
    let a=textureLoad(rl_lobe_a,p,0);
    let origin=a.xyz;
    let ws=unpack2x16float(bitcast<u32>(a.w));
    let n=rl_oct_decode(b.x);let v=rl_oct_decode(b.y);let side=rl_oct_decode(b.z);
    let alpha=unpack2x16float(b.w&0xffffu).x;
    let cone_alpha=alpha*inverseSqrt(f32(count));
    let cone=vec2<f32>(ws.x,ws.y+2.0*cone_alpha);
    // The inline loop's directions: the frame's radial offset and the pixel's rotation.
    let frame=primary_frame.sided.y;
    let seed=rl_hash((p.x*0x9e3779b1u)^rl_hash(p.y^(frame*0x85ebca6bu)));
    let rotation=f32(seed>>8u)*(1.0/16777216.0);
    let radial=select(0.5,fract(f32(frame)*0.6180339887),frame!=0u);
    let basis=rl_basis(n);
    let local_v=vec3<f32>(dot(v,basis[0]),dot(v,basis[1]),dot(v,basis[2]));
    let width=samples.x;
    // The receiver's own epsilon (its triangle's, as the inline loop's tmin).
    let epsilon=textureLoad(rl_receiver_epsilon,vec2<i32>(p),0).x;
    for(var k=0u;k<count;k+=1u) {
        let slot=first+k;
        let at=vec2<i32>(vec2<u32>(slot%width,slot/width));
        textureStore(rl_samples,at,vec4<f32>(0.0));
        let u=vec2<f32>((f32(k)+radial)/f32(count),fract(rotation+f32(k)*0.6180339887));
        let d=reflect(-v,basis*rl_vndf(local_v,max(alpha,1e-6),u));
        if dot(d,side)<=0.0 {continue;}
        let traced=rl_cast(origin,d,epsilon);
        if traced.record.x==0u {
            textureStore(rl_samples,at,vec4<f32>(rl_sky(d,cone_alpha),1.0));
            continue;
        }
        // A hit before its H append (the Mac's S4a hit reuse replaces this line under
        // RENDERLAB_RL_HIT_REUSE; keep it here, once, through S2).
        // ---- rl trace hit reuse seam ----
        let index=atomicAdd(&rl_hits.count,1u);
        // No record: the slot stays unwritten (a = 0) and out of the pixel's mean, never black.
        if index>=(arrayLength(&rl_hits.records)/3u) {atomicAdd(&rl_trace_stats.dropped,1u);continue;}
        let r=index*3u;
        let word2=k|(count<<8u)|(pack2x16float(vec2<f32>(alpha,0.0))<<16u);
        rl_hits.records[r]=vec4<u32>(p.x|(p.y<<16u),slot,word2,traced.record.x);
        rl_hits.records[r+1u]=vec4<u32>(traced.record.y,traced.record.z,traced.record.w,bitcast<u32>(origin.x));
        rl_hits.records[r+2u]=vec4<u32>(bitcast<u32>(origin.y),bitcast<u32>(origin.z),rl_oct_encode(d),pack2x16float(cone));
    }
}
// reflection_hits' indirect arguments from H's count: RL_HIT_GROUP records a group, rows of
// at most 65535 groups (wgpu skips larger dimensions).
@compute @workgroup_size(1) fn rl_hit_args(@builtin(local_invocation_index) lane:u32) {
    // `lane` is unused: every compute entry of the module names its lane (the Mac thread's
    // RENDERLAB_RS_STACK=workgroup rewrite sizes its stack slices from them).
    let written=min(atomicLoad(&rl_hits.count),arrayLength(&rl_hits.records)/3u);
    atomicStore(&rl_hits.count,written);
    let groups=(written+RL_HIT_GROUP-1u)/RL_HIT_GROUP;
    rl_hits.args[0]=min(groups,65535u);
    rl_hits.args[1]=(groups+65534u)/65535u;
    rl_hits.args[2]=1u;
}
