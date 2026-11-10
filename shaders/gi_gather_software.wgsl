// P-NRT3: this step's live cells, the deferred cells' appends, and their broadcasts.
var<workgroup> gi_wg_list:array<u32,GI_GATHER_LANES>;
var<workgroup> gi_wg_listed:atomic<u32>;
var<workgroup> gi_wg_appends:atomic<u32>;
var<workgroup> gi_wg_n:u32;
var<workgroup> gi_drain_base:u32;

@compute @workgroup_size(64) fn gi_gather_batched(@builtin(workgroup_id) group:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    if lane==0u { gi_wg_count=min(atomicLoad(&gi_work[gi_frame.bounce.x]),gi_frame.offsets.x); }
    let count=workgroupUniformLoad(&gi_wg_count);
    let requests=(gi_frame.bounce.z&GI_RELIGHT_REQUESTS)!=0u;
    let period=gi_gather_period(count);
    let phase=gi_word(GI_H_FRAME)%period;
    // P-NRT3: `width` lanes screen that many consecutive queue entries at once, one each, so a
    // capped drain's deferred cells cost one parallel step instead of one workgroup iteration
    // each. A workgroup still covers about count / groups entries, as one entry a step did.
    // The live cells it finds are then gathered one after another by all lanes, as before;
    // each cell's rays, sums and writes are unchanged, and queue order carries no meaning.
    let width=clamp(count/groups.x,1u,GI_GATHER_LANES);
    for(var head=group.x*width;head<count;head+=groups.x*width) {
        if lane==0u {
            atomicStore(&gi_wg_listed,0u);
            atomicStore(&gi_wg_appends,0u);
        }
        workgroupBarrier();
        let at=head+lane;
        var slot=0u;
        var deferred=false;
        var mine=0u;
        if lane<width && at<count {
            slot=atomicLoad(&gi_work[gi_frame.bounce.x+1u+at]);
            let cold=gi_cold(slot);
            let state=gi_table[cold+GI_C_STATE];
            let queued=gi_table[gi_hot(slot)]!=0u && (state&GI_BQUEUED)!=0u
                && (state&(GI_TRACED|GI_ORPHAN|GI_PARTIAL))==GI_TRACED;
            if queued && slot%period==phase {
                // This frame's slot group: gathered below by the whole workgroup.
                gi_wg_list[atomicAdd(&gi_wg_listed,1u)]=slot;
            } else if queued {
                // Outside this frame's slot group: next frame's queue, the schedule unchanged.
                // As clearing GI_BQUEUED and then gi_schedule(slot, max(left, 1)), with the
                // workgroup's appends to the next queue (and their counters) added once.
                let cleared=state&~GI_BQUEUED;
                let gathers=max((state&GI_GATHER_MASK)>>GI_GATHER_SHIFT,1u);
                let left=max((cleared&GI_GATHER_MASK)>>GI_GATHER_SHIFT,min(gathers,7u));
                gi_table[cold+GI_C_STATE]=(cleared&~GI_GATHER_MASK)|(left<<GI_GATHER_SHIFT)|GI_BQUEUED;
                deferred=true;
                mine=atomicAdd(&gi_wg_appends,1u);
            }
        }
        workgroupBarrier();
        if lane==0u {
            let appends=atomicLoad(&gi_wg_appends);
            var base=0u;
            if appends!=0u {
                atomicAdd(&gi_work[GI_W_GATHER_DEFERRED],appends);
                base=atomicAdd(&gi_work[gi_frame.bounce.y],appends);
                atomicAdd(&gi_work[GI_W_BQUEUE],appends);
            }
            gi_drain_base=base;
            gi_wg_n=atomicLoad(&gi_wg_listed);
        }
        let listed=workgroupUniformLoad(&gi_wg_n);
        if deferred {
            let a=gi_drain_base+mine;
            if a<gi_frame.offsets.x { atomicStore(&gi_work[gi_frame.bounce.y+1u+a],slot); }
        }
        for(var j=0u;j<listed;j+=1u) {
        let slot=gi_wg_list[j];
        let cold=gi_cold(slot);
        let state=gi_table[cold+GI_C_STATE];
        if lane==0u {
            atomicStore(&gi_wg_missing,0u);
            atomicStore(&gi_wg_changing,0u);
            atomicStore(&gi_wg_rays,0u);
        }
        workgroupBarrier();
        for(var i=0u;i<4u;i+=1u) { gi_gather_sh[i]=vec3<f32>(0.0); }
        {
            let c=gi_cell(slot);
            let n=gi_unpack_normal(gi_table[cold+GI_C_NORMAL]);
            let q=gi_point(cold);
            let frame=gi_basis(n);
            let origin=q+n*gi_origin_offset(q,c.edge);
            // Cells pixels made request in the secondary band, cells their gathers made in the
            // tertiary band, and tertiary cells request none: two generations of hit cells
            // beyond what pixels see, all below the pixels' band (gi_miss_claim).
            let secondary=(state&GI_SECONDARY)!=0u;
            let tertiary=(state&GI_TERTIARY)!=0u;
            let band=select(select(GI_BAND_SECONDARY,GI_BAND_TERTIARY,secondary),0u,!requests || tertiary);
            var missing=0u;
            var changing=0u;
            var rays=0u;
            let stride=max(gi_word(GI_H_GATHER_STRIDE),1u);
            for(var k=lane;k<GI_DIRECTIONS;k+=GI_GATHER_LANES) {
                if k%stride!=0u || (gi_table[cold+GI_C_BITS+k/32u]&(1u<<(k%32u)))!=0u { continue; }
                let d=gi_direction(frame,c.key,k);
                let h=gi_closest(origin,d,0.0,1e20);
                rays+=1u;
                if !h.hit { continue; }
                let o=gi_hit_radiance(origin,d,h,band,tertiary);
                if o.w==GI_HIT_MISSING || o.w==0.0 { missing+=1u; }
                if o.w==GI_HIT_CHANGING { changing+=1u; }
                if o.w<=GI_HIT_NONE { continue; }
                gi_sh_basis(d);
                let radiance=o.rgb*(6.283185307179586*f32(stride)/f32(GI_DIRECTIONS));
                for(var i=0u;i<4u;i+=1u) { gi_gather_sh[i]+=radiance*gi_y[i]; }
            }
            if missing!=0u { atomicAdd(&gi_wg_missing,missing); }
            if changing!=0u { atomicAdd(&gi_wg_changing,changing); }
            if rays!=0u { atomicAdd(&gi_wg_rays,rays); }
        }
        // The lanes' sums into lane 0 in a fixed tree order (no atomics on values).
        for(var i=0u;i<4u;i+=1u) {
            gi_wg_bounce[(i*3u)*GI_GATHER_LANES+lane]=gi_gather_sh[i].x;
            gi_wg_bounce[(i*3u+1u)*GI_GATHER_LANES+lane]=gi_gather_sh[i].y;
            gi_wg_bounce[(i*3u+2u)*GI_GATHER_LANES+lane]=gi_gather_sh[i].z;
        }
        workgroupBarrier();
        for(var stride=GI_GATHER_LANES/2u;stride>0u;stride/=2u) {
            if lane<stride {
                for(var j=0u;j<12u;j+=1u) {
                    gi_wg_bounce[j*GI_GATHER_LANES+lane]+=gi_wg_bounce[j*GI_GATHER_LANES+lane+stride];
                }
            }
            workgroupBarrier();
        }
        if lane==0u {
            atomicAdd(&gi_work[GI_W_GATHERS],1u);
            atomicAdd(&gi_work[GI_W_GATHER_RAYS],atomicLoad(&gi_wg_rays));
            let missing=atomicLoad(&gi_wg_missing);
            if missing!=0u { atomicAdd(&gi_work[GI_W_GATHER_MISSES],missing); }
            var m=0.0;
            for(var j=0u;j<12u;j+=1u) { m=max(m,abs(gi_wg_bounce[j*GI_GATHER_LANES])); }
            let hot=gi_hot(slot);
            if m<=0.0 {
                gi_table[hot+GI_HOT_BOUNCE]=GI_BOUNCE_NONE;
            } else {
                for(var w=0u;w<6u;w+=1u) {
                    let a=gi_wg_bounce[(2u*w)*GI_GATHER_LANES];
                    let b=gi_wg_bounce[(2u*w+1u)*GI_GATHER_LANES];
                    gi_table[hot+GI_HOT_BOUNCE+1u+w]=pack2x16float(vec2<f32>(a,b)/m);
                }
                gi_table[hot+GI_HOT_BOUNCE]=bitcast<u32>(m);
            }
            // The unfiltered value, the filter's source (G3).
            if gi_filter_on(GI_FILTER_BOUNCE) {
                for(var w=GI_HOT_BOUNCE;w<GI_HOT_BOUNCE+7u;w+=1u) { gi_raw[gi_raw_at(slot,w)]=gi_table[hot+w]; }
            }
            // The schedule: one gather done. A gather that met hit cells without a value yet
            // (new this frame, or requested now) is repeated once more.
            var left=(state&GI_GATHER_MASK)>>GI_GATHER_SHIFT;
            left=select(0u,left-1u,left>0u);
            // GC1: a gather whose every hit value was final ends the schedule. The cell's fixed
            // rays meet the same cells, whose values no longer change, so a further gather would
            // replace this value with itself (W1 form 1); an event that changes them schedules
            // its cells again. Cells reached while their hits still change keep the G gathers.
            // Moving cameras create cells among settled ones, which then gather once, not G
            // times (docs/m7-gc1.md).
            if missing==0u && atomicLoad(&gi_wg_changing)==0u { left=0u; }
            var retry=state&GI_RETRY;
            if missing!=0u {
                if left==0u && retry==0u { left=1u; retry=GI_RETRY; }
            } else {
                retry=0u;
            }
            gi_table[cold+GI_C_STATE]=(state&~(GI_GATHER_MASK|GI_BQUEUED|GI_RETRY))|retry;
            gi_queue_shade(slot);
            gi_schedule(slot,left);
        }
        workgroupBarrier();
        }
    }
}

