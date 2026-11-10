// P-S4B (docs/p-s4b.md, docs/design-nonrt-sunvis-s4.md): S4, occluder-centric sun visibility for
// the Surface lookup's queued pixels. src/sunvis_s4.rs appends this file to the surface cache
// module (virtual_shadow.wgsl + virtual_shadow_cache.wgsl) as its own module, and dispatches
// s4_missed in place of vsc_missed on frames of 8 or more direction batches (cuts and history
// restarts) where src/sunvis_s4.rs `chosen` expects it to beat the march. Other frames keep the march.
//
// Per 32-lane chunk of the queue (one pixel a lane, Exact's receiver, origin and directions):
//   1. tile volume: the chunk's origins in the sun frame (q across, h toward the sun), their q box,
//      lowest h and the disk slope;
//   2. gather: the warp walks a light-space BVH2 over every caster triangle cooperatively, a node
//      surviving when its box can meet a ray of the tile's cone; surviving leaves go to a workgroup
//      candidate list, flushed to every lane (3) whenever it fills. A chunk whose traversal stack
//      would overflow lets each lane walk the BVH alone with its own cone afterwards (S4-R);
//   3. per lane and candidate: s4_classify rejects (no direction can hit), certifies (every
//      direction hits) or tests each still-open direction with s4_hits;
//   4. the count to the target and the queue word, as vsc_finish writes them.
// Every triangle in a direction's ray lies in its receiver's cone, and every cull is a
// conservative superset of it, so the count is each direction's exact hit test over all
// triangles (docs/p-s4b.md: equal to RT's counts up to numerical boundaries).
//
// Patent contract (docs/patent-via-8471853.md): candidates are plane records (supporting plane,
// three edge planes), not vertex triplets; s4_hits is a boolean half-line/half-space predicate.
// No coordinate weights, no weight-based consistency, no reconstructed occluder depth or hit
// distance, and nothing from vsm_hits_sheared. src/sunvis_s4_math.rs is the CPU twin.

// src/sunvis_s4.rs `pack`: header (16 words), BVH2 nodes (16 words), plane records (24 words).
@group(2) @binding(60) var<storage,read> s4_geo:array<u32>;
const S4_LEAF:u32=0x80000000u;
const S4_EMPTY:u32=0xffffffffu;
const S4_NODE_WORDS:u32=16u;
const S4_RECORD_WORDS:u32=24u;
const S4_STACK:u32=1024u;
const S4_CANDIDATES:u32=512u;
// Flush the candidate list before an expansion step could overrun it (32 lanes x 2 leaves).
const S4_FLUSH:u32=S4_CANDIDATES-64u;
// A lane's own traversal (S4-R): src/sunvis_s4.rs keeps the tree at most this deep.
const S4_DEPTH:u32=32u;
fn s4_f(i:u32)->f32 {return bitcast<f32>(s4_geo[i]);}
fn s4_v(i:u32)->vec3<f32> {return vec3<f32>(s4_f(i),s4_f(i+1u),s4_f(i+2u));}
fn s4_v4(i:u32)->vec4<f32> {return vec4<f32>(s4_f(i),s4_f(i+1u),s4_f(i+2u),s4_f(i+3u));}
fn s4_u()->vec3<f32> {return s4_v(4u);}
fn s4_vb()->vec3<f32> {return s4_v(8u);}
fn s4_l()->vec3<f32> {return s4_v(12u);}
fn s4_records()->u32 {return s4_geo[2];}

struct S4Planes {p:vec4<f32>,e0:vec4<f32>,e1:vec4<f32>,e2:vec4<f32>,h:vec2<f32>,pl:f32,l_planes:bool,c:vec3<f32>};
fn s4_planes(t:u32)->S4Planes {
    let i=s4_records()+t*S4_RECORD_WORDS;
    let x=s4_v4(i+16u);
    return S4Planes(s4_v4(i),s4_v4(i+4u),s4_v4(i+8u),s4_v4(i+12u),x.xy,x.z,(s4_geo[i+19u]&1u)!=0u,s4_v(i+20u));
}

// sunvis_s4_math.rs `tolerance` and `hits`.
fn s4_tolerance(a:vec3<f32>)->f32 {return 2e-7*(abs(a.x)+abs(a.y)+abs(a.z))+1e-7;}
fn s4_plane(q:vec4<f32>,x:vec3<f32>)->f32 {return fma(q.x,x.x,fma(q.y,x.y,q.z*x.z))+q.w;}
fn s4_side(e:vec4<f32>,a:vec3<f32>,d:vec3<f32>,m:f32,g:f32,tol:f32)->bool {
    let ea=s4_plane(e,a);
    let ed=fma(e.x,d.x,fma(e.y,d.y,e.z*d.z));
    return fma(ea,m,-(g*ed))>=-tol;
}
fn s4_hits(t:S4Planes,o:vec3<f32>,d:vec3<f32>,eps:f32)->bool {
    let a=fma(d,vec3<f32>(eps),o);
    let sa=s4_plane(t.p,a);
    let sd=fma(t.p.x,d.x,fma(t.p.y,d.y,t.p.z*d.z));
    if sd==0.0 || (sa>0.0 && sd>0.0) || (sa<0.0 && sd<0.0) {return false;}
    let m=abs(sd);
    let g=select(-sa,sa,sd>0.0);
    let tol=s4_tolerance(a)*m;
    return s4_side(t.e0,a,d,m,g,tol) && s4_side(t.e1,a,d,m,g,tol) && s4_side(t.e2,a,d,m,g,tol);
}
// sunvis_s4_math.rs `classify`: 0 reject, 1 certify, 2 test each direction.
fn s4_classify(t:S4Planes,o:vec3<f32>,eps:f32,slope:f32,thin:bool)->u32 {
    let big=max(max(abs(o.x),abs(o.y)),abs(o.z))+1.0;
    let margin=4e-6*big+select(0.0,2.0*eps,thin);
    let l=s4_l();
    let ho=fma(l.x,o.x,fma(l.y,o.y,l.z*o.z));
    let hmax=t.h.y-ho;
    let hmin=t.h.x-ho;
    if hmax<0.5*eps-margin {return 0u;}
    let s=slope*1.001+2e-6;
    let reach=max(hmax,0.0)*(1.0+s*s)*1.0001+eps+margin;
    let v=vec3<f32>(s4_plane(t.e0,o),s4_plane(t.e1,o),s4_plane(t.e2,o));
    let r=reach*(t.c+vec3<f32>(s))+vec3<f32>(margin);
    if any(v+r<vec3<f32>(0.0)) {return 0u;}
    if !thin && all(v-r>vec3<f32>(margin)) && t.l_planes && t.pl>2.0*s+1e-3 && hmin>4.0*eps+margin {return 1u;}
    return 2u;
}

// ---- Per-lane state ----
// The pixel's receiver, its shared origin (opaque receivers), its sequence and the frame's
// directions (K = vsm_total(); direction j is batch j / n8e.x's j % n8e.x-th, as vsm_directions).
struct S4Pixel {valid:bool,r:VsmReceiver,o:vec3<f32>,level:u32,shift:vec2<f32>,l:vec3<f32>,u:vec3<f32>,v:vec3<f32>,slope:f32};
var<private> s4_px:S4Pixel;
// Open directions left (0: done) and the candidate tests this lane ran (the cache's work estimate).
var<private> s4_open:u32;
var<private> s4_tests:u32;
// Blocked directions per lane, bit j%32 of word j/32; directions at or above K start blocked.
var<workgroup> s4_mask:array<array<u32,32>,8>;

// vsm_prepare without the receiver triangle read or the direction tables.
fn s4_prepare(id:vec2<u32>)->S4Pixel {
    var out:S4Pixel;
    let size=textureDimensions(vsm_world);
    let p=vec2<i32>(id);
    let record=textureLoad(vsm_hit,p,0);
    let normal=textureLoad(vsm_normal,p,0);
    let light=lights[0];
    if record.x==0u || normal.w==0.0 || light.kind.z==0u {return out;}
    let world=textureLoad(vsm_world,p,0);
    let depth=dot(shadow_views[light.kind.y].camera_depth,vec4<f32>(world.xyz,1.0));
    var layer=VSM_NONE;
    for(var i=0u;i<light.kind.z;i+=1u) {
        let coverage=shadow_views[light.kind.y+i].coverage_bias;
        if depth>=coverage.x && depth<=coverage.y {layer=light.kind.y+i;break;}
    }
    if layer==VSM_NONE {return out;}
    let view=shadow_views[layer];
    out.r=VsmReceiver(world.xyz,normal.xyz,world.w,normal.w<0.0,record.x-1u,0u,record.y);
    let l=-light.travel_outer.xyz;
    let rows=transpose(view.world_to_clip);
    out.u=normalize(rows[0].xyz);out.v=normalize(rows[1].xyz);out.l=l;
    let seed=vsm_hash(id.x+id.y*size.x);
    out.shift=vec2<f32>(f32(seed>>8u),f32(vsm_hash(seed^0xa511e9b3u)>>8u))*(1.0/16777216.0);
    out.slope=view.texel.z;
    out.o=vsm_origin(out.r,l);
    out.level=textureLoad(vsm_level_in,p,0).x;
    out.valid=true;
    return out;
}
fn s4_direction(j:u32)->vec3<f32> {
    let per=max(vsm.n8e.x,1u);
    let index=vsm.nsq.y+8u*(j/per)+j%per;
    return vsm_direction(index,s4_px.shift,s4_px.l,s4_px.u,s4_px.v,s4_px.slope);
}

// One candidate triangle against this lane's receiver.
fn s4_pair(t:u32) {
    if s4_open==0u {return;}
    let planes=s4_planes(t);
    let thin=s4_px.r.thin;
    let eps=s4_px.r.epsilon;
    let verdict=s4_classify(planes,s4_px.o,eps,s4_px.slope,thin);
    if verdict==0u {return;}
    s4_tests+=1u;
    let k=vsm_total();
    if verdict==1u {
        for(var w=0u;w<8u;w+=1u) {s4_mask[w][vsm_lane]=0xffffffffu;}
        s4_open=0u;
        return;
    }
    for(var w=0u;w<(k+31u)/32u;w+=1u) {
        var word=s4_mask[w][vsm_lane];
        if word==0xffffffffu {continue;}
        var free=~word;
        loop {
            if free==0u {break;}
            let b=firstTrailingBit(free);
            free&=free-1u;
            let d=s4_direction(32u*w+b);
            var o=s4_px.o;
            if thin {o=vsm_origin(s4_px.r,d);}
            if s4_hits(planes,o,d,eps) {word|=1u<<b;s4_open-=1u;}
        }
        s4_mask[w][vsm_lane]=word;
        if s4_open==0u {return;}
    }
}
fn s4_leaf(child:u32) {
    let first=(child&~S4_LEAF)>>3u;
    let count=(child&7u)+1u;
    for(var i=0u;i<count;i+=1u) {s4_pair(first+i);}
}

// ---- The tile's cone and the cooperative gather ----
// x: the chunk's q box (min.xy, max.xy) widened by thin origins' offsets; y: lowest origin height
// less those, the cone's lateral reach per unit height, 1 when any lane is valid.
var<workgroup> s4_lanes:array<vec4<f32>,32>;
var<workgroup> s4_pads:array<f32,32>;
var<workgroup> s4_tile:array<vec4<f32>,2>;
var<workgroup> s4_stack:array<u32,S4_STACK>;
var<workgroup> s4_sp:atomic<u32>;
var<workgroup> s4_cand:array<u32,S4_CANDIDATES>;
var<workgroup> s4_nc:atomic<u32>;
var<workgroup> s4_overflow:atomic<u32>;
var<workgroup> s4_over:u32;
// The loop's uniform state: stack size | candidates << 16; the chunk.
var<workgroup> s4_state:u32;
var<workgroup> s4_chunk:u32;

// The lateral drift per unit height of a ray within `slope` of the build's axis.
fn s4_drift(slope:f32)->f32 {let s=slope*1.001+2e-6;return s*(1.0+s*s)*1.0001;}
// Whether a light-space box (q.xy, h) can hold a hit of a ray from origins in q box [a, b] at
// heights from h0 up, drifting `drift` per unit height.
fn s4_box(lo:vec3<f32>,hi:vec3<f32>,a:vec2<f32>,b:vec2<f32>,h0:f32,drift:f32)->bool {
    if hi.z<h0 {return false;}
    let r=drift*(hi.z-h0)+1e-5*(1.0+max(abs(h0),max(max(abs(a.x),abs(a.y)),max(abs(b.x),abs(b.y)))));
    return all(lo.xy<=b+vec2<f32>(r)) && all(hi.xy>=a-vec2<f32>(r));
}
fn s4_light(x:vec3<f32>)->vec3<f32> {return vec3<f32>(dot(x,s4_u()),dot(x,s4_vb()),dot(x,s4_l()));}

// This lane's own traversal with its own cone (S4-R), after a stack overflow.
fn s4_alone() {
    if !s4_px.valid || s4_open==0u {return;}
    let pad=select(0.0,2.0*s4_px.r.epsilon,s4_px.r.thin);
    let q=s4_light(s4_px.o);
    let a=q.xy-vec2<f32>(pad);let b=q.xy+vec2<f32>(pad);
    let h0=q.z-pad;
    let drift=s4_drift(s4_px.slope);
    var stack:array<u32,S4_DEPTH>;
    var sp=1u;
    stack[0]=0u;
    while sp>0u && s4_open!=0u {
        sp-=1u;
        let n=16u+stack[sp]*S4_NODE_WORDS;
        for(var c=0u;c<2u;c+=1u) {
            let child=s4_geo[n+12u+c];
            if child==S4_EMPTY {continue;}
            if !s4_box(s4_v(n+6u*c),s4_v(n+6u*c+3u),a,b,h0,drift) {continue;}
            if (child&S4_LEAF)!=0u {s4_leaf(child);} else if sp<S4_DEPTH {stack[sp]=child;sp+=1u;}
        }
    }
}

// One chunk: lane `li` takes queue entry g (of n).
fn s4_run(g:u32,n:u32,li:u32) {
    s4_px.valid=false;
    var k=VSM_NONE;
    var id=vec2<u32>(0u);
    if g<n {
        k=vsm_scratch.pad0+g;
        let word=atomicLoad(&vsm_scratch.words[2u*k]);
        id=vec2<u32>(word&0xffffu,word>>16u);
        s4_px=s4_prepare(id);
        if !s4_px.valid {textureStore(vsm_output,vec2<i32>(id),vec4<u32>(256u));k=VSM_NONE;}
    }
    let total=vsm_total();
    s4_open=select(0u,total,s4_px.valid);
    s4_tests=0u;
    for(var w=0u;w<8u;w+=1u) {
        let first=32u*w;
        var word=0xffffffffu;
        if s4_px.valid && first<total {word=select(~0u<<((total-first)&31u),0u,total-first>=32u);}
        s4_mask[w][li]=word;
    }
    // 1. The tile's cone.
    var lane=vec4<f32>(0.0,0.0,3.4e38,-1.0);
    let pad=select(0.0,2.0*s4_px.r.epsilon,s4_px.valid && s4_px.r.thin);
    if s4_px.valid {
        let q=s4_light(s4_px.o);
        lane=vec4<f32>(q.xy,q.z-pad,s4_px.slope);
    }
    s4_lanes[li]=lane;
    s4_pads[li]=pad;
    workgroupBarrier();
    if li==0u {
        var a=vec2<f32>(3.4e38);var b=vec2<f32>(-3.4e38);var h0=3.4e38;var slope=-1.0;
        var pad=0.0;
        for(var i=0u;i<32u;i+=1u) {
            let x=s4_lanes[i];
            if x.w<0.0 {continue;}
            a=min(a,x.xy);b=max(b,x.xy);h0=min(h0,x.z);slope=max(slope,x.w);
        }
        // Thin origins move up to 2 epsilon from the shared one, also across.
        for(var i=0u;i<32u;i+=1u) {pad=max(pad,s4_pads[i]);}
        let any=slope>=0.0;
        s4_tile[0]=vec4<f32>(a-vec2<f32>(pad),b+vec2<f32>(pad));
        s4_tile[1]=vec4<f32>(h0,s4_drift(max(slope,0.0)),select(0.0,1.0,any),0.0);
        s4_stack[0]=0u;
        atomicStore(&s4_sp,1u);atomicStore(&s4_nc,0u);atomicStore(&s4_overflow,0u);
        s4_state=select(0u,1u,any);
    }
    // 2-3. Gather, flushing candidates to every lane.
    loop {
        let state=workgroupUniformLoad(&s4_state);
        let sp=state&0xffffu;
        let nc=state>>16u;
        if nc>S4_FLUSH || (sp==0u && nc>0u) {
            for(var c=0u;c<nc;c+=1u) {s4_leaf(s4_cand[c]);}
            workgroupBarrier();
            if li==0u {atomicStore(&s4_nc,0u);s4_state=sp;}
            continue;
        }
        if sp==0u {break;}
        let take=min(sp,32u);
        var node=S4_EMPTY;
        if li<take {node=s4_stack[sp-1u-li];}
        if li==0u {atomicStore(&s4_sp,sp-take);}
        workgroupBarrier();
        if node!=S4_EMPTY {
            let box=s4_tile[0];let cone=s4_tile[1];
            let n=16u+node*S4_NODE_WORDS;
            for(var c=0u;c<2u;c+=1u) {
                let child=s4_geo[n+12u+c];
                if child==S4_EMPTY {continue;}
                if !s4_box(s4_v(n+6u*c),s4_v(n+6u*c+3u),box.xy,box.zw,cone.x,cone.y) {continue;}
                if (child&S4_LEAF)!=0u {
                    s4_cand[atomicAdd(&s4_nc,1u)]=child;
                } else {
                    let at=atomicAdd(&s4_sp,1u);
                    if at<S4_STACK {s4_stack[at]=child;} else {atomicStore(&s4_overflow,1u);}
                }
            }
        }
        workgroupBarrier();
        if li==0u {s4_state=min(atomicLoad(&s4_sp),S4_STACK)|(atomicLoad(&s4_nc)<<16u);}
    }
    // An overflowed stack dropped subtrees: each lane walks the whole tree alone (exact, as the
    // blocked bits only accumulate).
    if li==0u {s4_over=atomicLoad(&s4_overflow);}
    if workgroupUniformLoad(&s4_over)!=0u {s4_alone();}
    // 4. The count and the queue word (vsc_finish's).
    if k!=VSM_NONE {
        var visible=0u;
        for(var w=0u;w<8u;w+=1u) {visible+=countOneBits(~s4_mask[w][li]);}
        vsm_visible(id,visible);
        // vsc_finish's verdict code: the count of one batch, or solidity over several.
        var code=visible;
        if vsm.nsq.x>1u {code=select(select(1u,vsm.n8e.x,visible==total),0u,visible==0u);}
        let start=u32((i32(vsm.levels[0].info.y)+i32(min(s4_px.level,15u)))&15);
        // The walk order's estimate for the frames that march again: heavy where the pixel is
        // partly shadowed (penumbrae walk longest), light elsewhere.
        let work=select(2u,5u,visible!=0u && visible!=total);
        atomicStore(&vsm_scratch.words[2u*k+1u],0x80000000u|code|(start<<4u)|(work<<8u));
    }
}

// The queued pixels, 32 a chunk, chunks taken from the walk's cursor until the queue is covered.
@compute @workgroup_size(32) fn s4_missed(@builtin(local_invocation_index) li:u32) {
    vsm_lane=li;
    let n=atomicLoad(&vsm_scratch.queue);
    loop {
        if li==0u {s4_chunk=atomicAdd(&vsm_scratch.words[vsc_cursor()],1u);}
        let t=workgroupUniformLoad(&s4_chunk);
        if t*32u>=n {break;}
        s4_run(t*32u+li,n,li);
        // The next chunk's tile writes wait for this chunk's last reads.
        workgroupBarrier();
    }
}
