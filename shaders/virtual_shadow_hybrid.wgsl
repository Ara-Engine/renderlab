// N8k hybrid (docs/non-rt-n8k.md), `VirtualLookup::Hybrid`: the exact lookup of
// shaders/virtual_shadow.wgsl for the receivers the map lookup (virtual_shadow_lookup.wgsl)
// queued because one of its blocks was ambiguous. virtual_shadow_gpu.rs appends this file to
// virtual_shadow.wgsl, so it calls that module's per-pixel functions as vsm_lookup does.
// The queue is the lookup's read-write scratch binding (vsm_scratch, N8m): word 0 (`queue`) the
// count, word 1 (`spread`) the lanes per receiver (N8m; vmap_queue_args, which sizes the dispatch),
// then from word 4 (`words`) two words per receiver: x | y << 16, and the directions whose map
// block was ambiguous (bits 0-7) | every direction the map blocked (bits 8-15).
// VSM_QUEUED_ALL (default) tests every direction again; off, only the ambiguous ones are, and
// the map's other decisions stand, which was no cheaper and lost quality (docs/non-rt-n8k.md).
override VSM_QUEUED_ALL:bool=true;
// VSM_QUEUED_RING: only the exact near test (the receiver triangle and its vertex 1-ring, bounded
// fan walks, no march) for the ambiguous directions; an ambiguous map block stands only when it
// confirms it, every other map decision stands. Its blocks are near-field ones (facing, an own
// surface near the receiver), which the ring decides exactly; it skips the march's long walks.
override VSM_QUEUED_RING:bool=false;
// N8l keeps per-lane direction tables in workgroup memory (vsm_lane); 32 lanes, as vsm_lookup.
@compute @workgroup_size(32) fn vsm_lookup_queued(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    vsm_lane=li;
    let spread=max(vsm_scratch.spread,1u);
    if gid.x%spread!=0u {return;}
    let k=gid.x/spread;
    if k>=atomicLoad(&vsm_scratch.queue) {return;}
    let word=vec2<u32>(atomicLoad(&vsm_scratch.words[2u*k]),atomicLoad(&vsm_scratch.words[2u*k+1u]));
    let id=vec2<u32>(word.x&0xffffu,word.x>>16u);
    let px=vsm_prepare(id);
    if !px.valid {textureStore(vsm_output,vec2<i32>(id),vec4<u32>(256u));return;}
    let r=px.r;
    let frame=vsm_active;
    if VSM_QUEUED_RING {
        // With the map's VMAP_RING, bits 16-23 hold its far-field blocks of those directions.
        let ambiguous=word.y&0xffu&frame;
        let mapped=(word.y>>8u)&0xffu&frame;
        let far=(word.y>>16u)&0xffu;
        vsm_active=ambiguous;
        vsm_visits=0u;
        let ring=vsm_ring(r,(vsm.misc.w&1u)!=0u && !vsm_beyond_horizon(r,px.l));
        vsm_active=frame;
        vsm_visible(id,countOneBits(frame&~((mapped&~ambiguous)|((ring.blocked|far)&ambiguous))));
        return;
    }
    let ambiguous=select(word.y&0xffu&frame,frame,VSM_QUEUED_ALL);
    let kept=select((word.y>>8u)&0xffu&~ambiguous&frame,0u,VSM_QUEUED_ALL);
    vsm_active=ambiguous;
    vsm_visits=0u;
    let ring=vsm_ring(r,(vsm.misc.w&1u)!=0u && !vsm_beyond_horizon(r,px.l));
    var blocked=ring.blocked;
    if VSM_FAR && px.level!=VSM_NONE && blocked!=0xffu {
        let open=vsm_march(r,ring.radius,px.level,0xffu&~blocked);
        blocked=0xffu&~open;
    }
    vsm_active=frame;
    vsm_visible(id,countOneBits(frame&~((blocked&ambiguous)|kept)));
}
