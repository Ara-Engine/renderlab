// ---- U-RTVIS rl_trace (docs/design-reflection-pass.md) ----
// Appended to a traced module of the standard program (src/reflection_pass.rs); it uses the
// reflection block's helpers (rl_vndf, rl_basis, rl_hash, rl_sky, rl_cast, rl_oct_*) and no
// program code. After shading, the frame's glossy-pixel list (shaders/reflections_lobe.wgsl:
// every reporting program's glossy receiver that fit the composite's room) is walked one entry an
// invocation: the receiver's full strata set into its reserved slots of R (a miss's sky) and H (a
// hit's record for reflection_hits). The composite then sums each entry's slots. Every kernel
// sizes itself from the list's length, so a view with little to reflect costs little.
// L: its length, then three vec4<u32> an entry (reflections_lobe.wgsl documents the layout).
struct RlGlossyEntries { count:u32, header:array<u32,3>, entries:array<vec4<u32>> };
@group(0) @binding(56) var<storage,read> rl_glossy:RlGlossyEntries;
@group(0) @binding(58) var<uniform> rl_pass_frame:vec4<u32>;
// The list's indirect arguments (rl_list_args): rl_trace_pixels' dispatch (x, y, z), then the
// composite's draw (vertex count, instance count, first vertex, first instance). Their own group,
// bound only by rl_list_args: the trace's dispatch reads them as indirect arguments, which may not
// share a dispatch with a read-write binding of the same buffer.
@group(2) @binding(0) var<storage,read_write> rl_list_arguments:array<u32,8>;
// H: count, the three dispatch arguments of reflection_hits (RL_HIT_GROUP-wide groups), then
// 48-byte records (three vec4<u32>).
struct RlHits { count:atomic<u32>, args:array<u32,3>, records:array<vec4<u32>> };
@group(0) @binding(64) var<storage,read_write> rl_hits:RlHits;
@group(0) @binding(65) var rl_samples:texture_storage_2d<rgba16float,write>;
const RL_HIT_GROUP:u32=64u;
// rl_trace_pixels' entries a workgroup, and workgroups a dispatch row (wgpu skips larger
// dimensions).
const RL_LIST_GROUP:u32=64u;
const RL_LIST_ROW:u32=65535u;
// x glossy pixels traced, y slots traced, z strata requested (every glossy receiver's, summed at
// shading: rl_room_reserve), w hit records dropped for want of room (their strata stay out of the
// pixel's mean). The buffer's later words are the shading side's and the Mac thread's
// (reflections_lobe.wgsl); the probe reads them all. (The Mac thread's S4a composition rewrites
// this declaration's exact text, so it stays on one line.)
struct RlTraceStats { pixels:atomic<u32>, slots:atomic<u32>, requested:atomic<u32>, dropped:atomic<u32> };
@group(0) @binding(60) var<storage,read_write> rl_trace_stats:RlTraceStats;
// The receivers' ray epsilon per pixel (reflection_pass.rs RECEIVER_EPSILON).
@group(0) @binding(79) var rl_receiver_epsilon:texture_2d<f32>;
// The list's length (shading's appends; never past its capacity).
fn rl_list_length()->u32 { return min(rl_glossy.count,rl_pass_frame.z); }

// The trace's and the composite's indirect arguments from the list's length.
@compute @workgroup_size(1) fn rl_list_args(@builtin(local_invocation_index) lane:u32) {
    // `lane` is unused: every compute entry of the module names its lane (the Mac thread's
    // RENDERLAB_RS_STACK=workgroup rewrite sizes its stack slices from them).
    let entries=rl_list_length();
    let groups=(entries+RL_LIST_GROUP-1u)/RL_LIST_GROUP;
    rl_list_arguments[0]=min(groups,RL_LIST_ROW);
    rl_list_arguments[1]=(groups+RL_LIST_ROW-1u)/RL_LIST_ROW;
    rl_list_arguments[2]=1u;
    rl_list_arguments[3]=0u;
    rl_list_arguments[4]=entries;
    rl_list_arguments[5]=1u;
    rl_list_arguments[6]=0u;
    rl_list_arguments[7]=0u;
}

// One list entry an invocation: its receiver's full strata set (L's count: today's faded set,
// directions and cones) into its reserved slots, a miss's sky into R and a hit's record into H.
@compute @workgroup_size(64) fn rl_trace_pixels(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let entry=(group.x+group.y*RL_LIST_ROW)*RL_LIST_GROUP+lane;
    if entry>=rl_list_length() {return;}
    let a=rl_glossy.entries[entry*3u];let b=rl_glossy.entries[entry*3u+1u];let c=rl_glossy.entries[entry*3u+2u];
    let p=vec2<u32>(c.x&0xffffu,c.x>>16u);
    let first=c.y;
    let count=(b.w>>16u)&31u;
    let samples=textureDimensions(rl_samples);
    // A safeguard only: the reservations fit the room.
    let capacity=min(samples.x*samples.y,arrayLength(&rl_hits.records)/3u);
    if count==0u || first+count>capacity {return;}
    atomicAdd(&rl_trace_stats.pixels,1u);
    atomicAdd(&rl_trace_stats.slots,count);
    let origin=bitcast<vec3<f32>>(a.xyz);
    let ws=unpack2x16float(a.w);
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
