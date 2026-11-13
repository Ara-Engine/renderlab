// ---- Local-light visibility history (light_cluster.rs `local_shadow_budget`) ----
// U-FIXED's sun visibility history (shaders/primary_sun_history.wgsl) for local lights on
// camera receivers. Each receiver keeps the visibility of its first four ray-shadowed lights,
// reprojected through the previous unjittered camera: a visible fraction and a sample count.
// For each light whose path to the receiver no moving or skinned placement can cross (its
// bounds, grown by the light's radius, miss the segment to the light), outside a changed
// light's restart sphere, with a matching history: a long fully lit or fully shadowed history
// is held without rays (re-checked one frame in RL_LOCAL_HOLD); otherwise the receiver
// traces a probe, RL_LOCAL_PROBES rays of a soft light's rotating disk sequence or a point
// source's one ray. A uniform history whose probe agrees is kept; one whose probe disagrees
// traces the budget; an edge or penumbra adds the probe to its average (at most
// RL_LOCAL_HISTORY_MAX samples). Every other receiver and light traces the budget, and still
// ones start a history there. A frame that restarts every history (a cut, a TR1 restart, a
// first frame) has words.x 0.
// History texel (rl_local_history_*): x depth code (12) | frame stamp (4) << 12; y slot k's
// light (8) << 8k, by kind.w's index plus one (0 none); z slots 0 and 1, w slots 2 and 3,
// each a half (visible fraction (10) << 6 | count (6)) << 16 (k & 1). A pixel nothing shaded
// last frame keeps an older stamp, which no receiver accepts. With two slots, Outpost's third
// and fourth spots traced every frame and kept nearly every SIMD group tracing.
// previous, current: unjittered view-projections. words: x history continues, y moving
// boxes, z frame number (its low 4 bits the stamp). boxes: each moving box's min then max
// corner.
struct RlLocalRecord { previous:mat4x4<f32>, current:mat4x4<f32>, words:vec4<u32>, boxes:array<vec4<f32>,8> };
@group(3) @binding(31) var<uniform> rl_local_record:RlLocalRecord;
@group(3) @binding(32) var rl_local_history_in:texture_2d<u32>;
@group(3) @binding(33) var rl_local_history_out:texture_storage_2d<rgba32uint,write>;
const RL_LOCAL_PROBES:u32=1u;
// Measurement only (light_cluster.rs `budget_mode`): 1 `show`, 2 `moving`, 4 `found` and
// 5 `hold` shade a light 10^4 times over where the history serves it, the moving test sends it
// to the budget, a history was found, or it is held without rays; the history itself is
// unchanged. 3 `overhead` keeps the history but never serves from it; 6 `norays` traces no
// camera receiver's local rays at all (the most the history could save); 7 `unslotted`
// shades a light 10^4 times over where it finds no history slot; 8 `traced` where it traced
// a ray this frame.
const RL_LOCAL_SHOW:u32=0u;
const RL_LOCAL_HISTORY_MAX:u32=32u;
// A fully lit or fully shadowed history of at least RL_LOCAL_HOLD_COUNT samples is held
// without rays, re-checked by its probe one frame in RL_LOCAL_HOLD (rl_local_recheck): with
// every change the restart set names already restarting it, its visibility cannot change.
const RL_LOCAL_HOLD_COUNT:u32=16u;
const RL_LOCAL_HOLD:u32=32u;
// The camera receiver's state, set by rl_local_history_prepare: reflection hits, shaded
// before it, trace their budget. looked: its history was looked up, on its first
// ray-shadowed light (a receiver no such light reaches neither reads nor stores one; its
// pixel's older stamp turns the stale word away). found: its reprojected history word;
// next: the word it stores; cached: each slot's visibility this frame (negative before it
// is traced), so a program that asks a light twice gets one estimate.
var<private> rl_local_live:bool;
var<private> rl_local_looked:bool;
var<private> rl_local_found:vec4<u32>;
var<private> rl_local_next:vec4<u32>;
var<private> rl_local_cached:vec4<f32>;

// log2 of the camera-space w in 1/256 octaves, as primary_sun_history.wgsl's depth code.
fn rl_local_depth_code(w:f32)->u32 {return u32(i32(round(log2(max(w,1e-6))*256.0)))&4095u;}
fn rl_local_half(visible:f32,count:u32)->u32 {
    return (u32(round(clamp(visible,0.0,1.0)*1023.0))<<6u)|min(count,63u);
}

// rl_lights_prepare: the camera receiver's lights may use the history.
fn rl_local_history_prepare(world:vec3<f32>) {
    rl_local_live=true;
    rl_local_looked=false;
}
// The receiver at `world` finds its history.
fn rl_local_lookup(world:vec3<f32>) {
    rl_local_looked=true;
    rl_local_cached=vec4<f32>(-1.0);
    rl_local_found=vec4<u32>(0u);
    let stamp=rl_local_record.words.z&15u;
    let here=rl_local_record.current*vec4<f32>(world,1.0);
    rl_local_next=vec4<u32>(rl_local_depth_code(here.w)|(stamp<<12u),0u,0u,0u);
    if rl_local_record.words.x==0u || rl_zone_relit(world) {return;}
    let c=rl_local_record.previous*vec4<f32>(world,1.0);
    if !(c.w>0.0) {return;}
    let size=vec2<i32>(textureDimensions(rl_local_history_in));
    let q=vec2<i32>(floor(fma(c.xy/c.w,vec2<f32>(0.5,-0.5),vec2<f32>(0.5))*vec2<f32>(size)));
    if any(q<vec2<i32>(0)) || any(q>=size) {return;}
    let h=textureLoad(rl_local_history_in,q,0);
    let d=(h.x-rl_local_depth_code(c.w))&4095u;
    if ((h.x>>12u)&15u)==((stamp+15u)&15u) && (d<=2u || d>=4093u) {rl_local_found=h;}
}
// The entry's last step for a shaded camera pixel.
fn rl_local_history_store(pixel:vec2<i32>) {
    if rl_local_looked {textureStore(rl_local_history_out,pixel,rl_local_next);}
}
// Light `id`'s slot in the next texel, claiming a free one, or 4 when all hold other lights.
fn rl_local_slot(id:u32)->u32 {
    for(var k=0u;k<4u;k+=1u) {
        let held=(rl_local_next.y>>(8u*k))&255u;
        if held==id {return k;}
        if held==0u {rl_local_next.y|=id<<(8u*k);return k;}
    }
    return 4u;
}
// Slot k's half (fraction << 6 | count) of a texel.
fn rl_local_half_of(texel:vec4<u32>, k:u32)->u32 {
    return (select(texel.z,texel.w,k>=2u)>>(16u*(k&1u)))&0xffffu;
}
// Light `id`'s half of the found texel, 0 when it has none.
fn rl_local_found_half(id:u32)->u32 {
    for(var k=0u;k<4u;k+=1u) {
        if ((rl_local_found.y>>(8u*k))&255u)==id {return rl_local_half_of(rl_local_found,k);}
    }
    return 0u;
}
// Whether some moving box, grown by the light's radius, meets the segment from x to the
// light's centre: every disk ray stays within the radius of that segment, so no other
// receiver's view of the light can change. x inside a box (a moving receiver) counts.
fn rl_local_moving(light:Light, x:vec3<f32>)->bool {
    let d=light.position_range.xyz-x;
    let rcp=vec3<f32>(1.0)/select(d,vec3<f32>(1e-6),abs(d)<vec3<f32>(1e-6));
    let grow=vec3<f32>(light.source.x*1.01+1e-3);
    for(var k=0u;k<rl_local_record.words.y;k+=1u) {
        let a=(rl_local_record.boxes[2u*k].xyz-grow-x)*rcp;
        let b=(rl_local_record.boxes[2u*k+1u].xyz+grow-x)*rcp;
        let t0=min(a,b);let t1=max(a,b);
        if min(min(t1.x,t1.y),min(t1.z,1.0))>=max(max(max(t0.x,t0.y),t0.z),0.0) {return true;}
    }
    return false;
}
// Whether this frame re-checks a held light at this receiver: one frame in RL_LOCAL_HOLD,
// staggered per 8x8 pixel block and light so the re-checks spread over the frames while
// each SIMD group's lanes skip their rays together (a per-pixel stagger left some lane in
// almost every group tracing every frame).
fn rl_local_recheck(receiver:Receiver, id:u32)->bool {
    let block=vec2<u32>(receiver.pixel)>>vec2<u32>(3u);
    let seed=rl_local_hash(block.x^rl_local_hash(block.y^(id*0x9e3779b9u)));
    return ((rl_local_record.words.z+seed)%RL_LOCAL_HOLD)==0u;
}
// Slot k's cached visibility (negative before it is traced), read and written through
// select chains: a store into a private vector at a runtime index can silently do nothing on
// NVIDIA through naga, which would keep every pixel tracing.
fn rl_local_cached_at(k:u32)->f32 {
    return select(select(select(rl_local_cached.w,rl_local_cached.z,k==2u),rl_local_cached.y,k==1u),rl_local_cached.x,k==0u);
}
fn rl_local_cache(k:u32, visible:f32) {
    rl_local_cached=select(rl_local_cached,vec4<f32>(visible),vec4<u32>(k)==vec4<u32>(0u,1u,2u,3u));
}
fn rl_local_visibility(light:Light, receiver:Receiver, l:vec3<f32>) -> f32 {
    let id=light.kind.w&0xffffu;
    if RL_LOCAL_SHOW==6u && rl_local_live {return 1.0;}
    if !rl_local_live || id>255u {return rl_local_rays_visibility(light,receiver,l);}
    if !rl_local_looked {rl_local_lookup(receiver.world);}
    let slot=rl_local_slot(id);
    if slot==4u {
        if RL_LOCAL_SHOW==7u {return 1e4;}
        return rl_local_rays_visibility(light,receiver,l);
    }
    let cached=rl_local_cached_at(slot);
    if cached>=0.0 {return cached;}
    let soft=light.source.x>0.0;
    let still=!rl_local_moving(light,receiver.world);
    let found=select(0u,rl_local_found_half(id),still);
    let count=select(found&63u,0u,RL_LOCAL_SHOW==3u);
    var visible=-1.0;
    var half=0u;
    var served=true;
    var held=false;
    // A point source's probe is its one ray, which a contradicted history keeps.
    var probe=-1.0;
    if count!=0u {
        let kept=f32(found>>6u)/1023.0;
        let lit=(found>>6u)==1023u;let dark=(found>>6u)==0u;
        if (lit || dark) && count>=RL_LOCAL_HOLD_COUNT && !rl_local_recheck(receiver,id) {
            visible=kept;
            half=found;
            held=true;
        } else {
            // `if`, not `select`: select evaluates both sides, tracing both.
            if soft {
                probe=rl_local_disk_rays(light,receiver,l,rl_light_frame.info.z&1023u,RL_LOCAL_PROBES);
            } else {
                probe=rl_local_rays_visibility(light,receiver,l);
            }
            let probes=select(1u,RL_LOCAL_PROBES,soft);
            if (lit && probe==1.0) || (dark && probe==0.0) {
                visible=kept;
                half=(found&0xffc0u)|min(count+probes,63u);
            } else if !lit && !dark {
                let n=f32(count);
                visible=(kept*n+probe*f32(probes))/(n+f32(probes));
                half=rl_local_half(visible,min(count+probes,RL_LOCAL_HISTORY_MAX));
            }
        }
    }
    // No usable history, or a uniform one its probe contradicts: the budget.
    if visible<0.0 {
        served=false;
        if !soft && probe>=0.0 {
            visible=probe;
        } else {
            visible=rl_local_rays_visibility(light,receiver,l);
        }
        let samples=select(1u,max(rl_local_rays,4u),soft);
        if still {half=rl_local_half(visible,min(samples,RL_LOCAL_HISTORY_MAX));}
    }
    if slot>=2u {rl_local_next.w|=half<<(16u*(slot&1u));} else {rl_local_next.z|=half<<(16u*(slot&1u));}
    rl_local_cache(slot,visible);
    if (RL_LOCAL_SHOW==1u && served) || (RL_LOCAL_SHOW==2u && !still) || (RL_LOCAL_SHOW==4u && count!=0u) || (RL_LOCAL_SHOW==5u && held) || (RL_LOCAL_SHOW==8u && !held) {return 1e4;}
    return visible;
}
