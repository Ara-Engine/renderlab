// U-FIXED sun visibility history (audit 3.3), appended to the reconstruction's held trace
// module in place of the held stop (temporal_reconstruction.rs `trace_source_history`). Each
// receiver keeps its own sun visibility, reprojected through the previous unjittered camera:
// a visible fraction and a sample count. A receiver nothing can change (a still, unskinned
// placement outside every moving caster's swept shadow and every restart zone, under an
// unturned sun) with a matching history traces SUN_HISTORY_PROBES new directions of TR1's
// rotating sequence. A fully lit or fully shadowed history whose probes agree is kept; one
// whose probes disagree traces the frame's whole budget; a penumbra adds the probes to its
// average. Every other receiver traces the budget as before (cold probes included) and
// starts its history from that estimate.
// History word (rtvis_history_*): visible fraction << 16 | count << 10 | depth code; 0 is none.
const SUN_HISTORY_PROBES:u32=4u;
const SUN_HISTORY_MAX:u32=32u;
// This receiver's next history word, written by primary_sun_store.
var<private> sun_history_next:u32;

// log2 of the camera-space w in 1/64 octaves, 10 bits: a reprojected texel within two codes
// (about 2% of the depth) is the same surface.
fn sun_history_depth(w:f32)->u32 {return u32(i32(round(log2(max(w,1e-6))*64.0)))&1023u;}
fn sun_history_word(visible:f32,count:u32,depth:u32)->u32 {
    return (u32(round(clamp(visible,0.0,1.0)*65535.0))<<16u)|(min(count,63u)<<10u)|depth;
}

// The receiver's sun visibility in 256ths (tr1_rays 256), or the budget's count over tr1_rays.
fn primary_sun(receiver:Receiver,slot:u32,pixel:vec2<u32>,width:u32)->u32 {
    sun_history_next=0u;
    let here=rtvis_held.current*vec4<f32>(receiver.world,1.0);
    let depth=sun_history_depth(here.w);
    let batch=primary_placements[slot]&0x1fffffffu;
    let still=!batch_skinned(primary_batch_info(batch)) && !rtvis_moved(slot)
        && !rtvis_swept(receiver.world)
        && tr1_zone_rays(receiver.world,primary_frame.sampling.x)==primary_frame.sampling.x;
    var found=0u;
    if rtvis_held.words.w!=0u && still {
        let c=rtvis_held.previous*vec4<f32>(receiver.world,1.0);
        let size=vec2<i32>(textureDimensions(rtvis_history_in));
        if c.w>0.0 {
            let q=vec2<i32>(floor(fma(c.xy/c.w,vec2<f32>(0.5,-0.5),vec2<f32>(0.5))*vec2<f32>(size)));
            if all(q>=vec2<i32>(0)) && all(q<size) {
                let h=textureLoad(rtvis_history_in,q,0).x;
                let d=(h-sun_history_depth(c.w))&1023u;
                if ((h>>10u)&63u)!=0u && (d<=2u || d>=1021u) {found=h;}
            }
        }
    }
    if found!=0u {
        let count=(found>>10u)&63u;
        let kept=f32(found>>16u)/65535.0;
        let v=sun_probe(receiver,pixel,width,primary_frame.sided.y,SUN_HISTORY_PROBES);
        if v!=FRESH_SKIP {
            let lit=(found>>16u)==65535u;let dark=(found>>16u)==0u;
            if (lit && v==SUN_HISTORY_PROBES) || (dark && v==0u) {
                tr1_rays=256u;
                sun_history_next=sun_history_word(kept,count+SUN_HISTORY_PROBES,depth);
                return select(0u,256u,lit);
            }
            if !lit && !dark {
                let n=f32(count);
                let next=(kept*n+f32(v))/(n+f32(SUN_HISTORY_PROBES));
                tr1_rays=256u;
                sun_history_next=sun_history_word(next,min(count+SUN_HISTORY_PROBES,SUN_HISTORY_MAX),depth);
                return u32(round(next*256.0));
            }
        }
    }
    // No usable history, or a uniform history its probes contradict: the frame's budget.
    let visible=fresh_estimate(receiver,pixel,width,0u);
    if still && visible!=FRESH_SKIP {
        sun_history_next=sun_history_word(f32(visible)/f32(tr1_rays),min(tr1_rays,SUN_HISTORY_MAX),depth);
    }
    return visible;
}
fn primary_sun_store(p:vec2<i32>,visible:u32) {textureStore(rtvis_history_out,p,vec4<u32>(sun_history_next));}
