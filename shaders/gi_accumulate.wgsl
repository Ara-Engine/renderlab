// ---- renderlab world radiance cache: accumulation (RENDERLAB_GI_ACCUMULATE=1) ----
// docs/design-gi-accumulate.md. Composed after gi_cache.wgsl (src/gi_cache.rs kernel_source).
// A cell's value was the sum of one fixed set of 64 directions, replaced whole by every gather,
// and a small bright source (Outpost's hangar doorway: sky and sunlit ground) was met by some
// cells' fixed directions and missed by their neighbours': a fixed mottle a cell wide. Here a
// cell's SH words hold its whole incident radiance (sky and bounce, absolute), and each update
// traces a few rays in a fresh rotation and blends them into that history, so a cell converges
// on all directions over its updates; no update is an event, and a fixed budget of updates runs
// every frame whatever the camera does.
// - gi_acc_select lists this frame's updates: a cell with no history yet (finish just gave it its
//   first value, sky only), visible cells every GI_H_ACC_VISIBLE frames, others every
//   GI_H_ACC_OTHER, each at a phase its slot fixes (the integrate queue carries the list; the
//   integrate pass does not run while accumulating);
// - gi_accumulate, one workgroup per listed cell: GI_H_ACC_RAYS rays (all 64 lanes for a cell's
//   first update), stratified over the hemisphere's solid angle in a rotation drawn per update; a
//   miss adds the sky's radiance, a hit the hit cell's outgoing radiance (gi_hit_radiance, which
//   also requests the hit cell as gathers do). The L2 SH of the sum blends into the history:
//   the slow weight is 1 / updates (at most GI_H_ACC_HISTORY); a fast mean of the update's L0
//   luminance (weight 1/2) leaving the slow mean's band (twice its standard deviation) pulls the
//   history to the new update (weight 1/2) and restarts its count. That one test stands for every
//   change: lights, sky, movers, cuts.
const GI_W_ACC_UPDATES:u32=210u;
const GI_W_ACC_RAYS:u32=211u;
const GI_W_ACC_PULLS:u32=212u;
const GI_ACC_LUMA:vec3<f32>=vec3<f32>(0.2126,0.7152,0.0722);

// One class of this frame's updates (0: cells without history, 1: visible cells due, 2: the rest
// due), appended while the frame's rays (GI_H_ACC_BUDGET, 0: no cap) last. The classes run as three
// passes in that order, so a cell without history is never left for a cell that has one.
fn gi_acc_select(kind:u32, id:vec3<u32>, groups:vec3<u32>) {
    let slots=gi_frame.offsets.x;
    let frame=gi_word(GI_H_FRAME);
    let pv=max(gi_word(GI_H_ACC_VISIBLE),1u);
    let po=max(gi_word(GI_H_ACC_OTHER),1u);
    let budget=gi_word(GI_H_ACC_BUDGET);
    let k=min(max(gi_word(GI_H_ACC_RAYS),1u),64u);
    for(var slot=id.x;slot<slots;slot+=groups.x*64u) {
        let hot=gi_hot(slot);
        if (gi_table[hot]&GI_VALID)==0u { continue; }
        let cold=gi_cold(slot);
        if (gi_table[cold+GI_C_STATE]&(GI_TRACED|GI_ORPHAN))!=GI_TRACED { continue; }
        let fresh=gi_table[hot+GI_HOT_ACC_COUNT]==0u;
        let visible=gi_table[cold+GI_C_SEEN]+1u>=frame;
        let phase=gi_mix(slot)+frame;
        var take=false;
        if kind==0u { take=fresh; }
        if kind==1u { take=!fresh && visible && phase%pv==0u; }
        if kind==2u { take=!fresh && !visible && phase%po==0u; }
        if !take { continue; }
        let rays=select(k,64u,fresh);
        if budget!=0u {
            // GI_W_ACC_RAYS counts the selected rays here; gi_accumulate adds nothing to it.
            let before=atomicAdd(&gi_work[GI_W_ACC_RAYS],rays);
            if before>=budget { continue; }
        }
        let at=atomicAdd(&gi_work[GI_W_INTEGRATES],1u);
        if at<slots { atomicStore(&gi_work[gi_frame.offsets.w+at],slot); }
    }
}
@compute @workgroup_size(64) fn gi_acc_select_fresh(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) { gi_acc_select(0u,id,groups); }
@compute @workgroup_size(64) fn gi_acc_select_visible(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) { gi_acc_select(1u,id,groups); }
@compute @workgroup_size(64) fn gi_acc_select_other(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) { gi_acc_select(2u,id,groups); }

var<workgroup> gi_wg_acc:array<f32,27u*64u>;
var<workgroup> gi_wg_acc_count:u32;
@compute @workgroup_size(64) fn gi_accumulate(@builtin(workgroup_id) group:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    if lane==0u { gi_wg_acc_count=min(atomicLoad(&gi_work[GI_W_INTEGRATES]),gi_frame.offsets.x); }
    let count=workgroupUniformLoad(&gi_wg_acc_count);
    let requests=(gi_frame.bounce.z&GI_RELIGHT_REQUESTS)!=0u;
    for(var at=group.x;at<count;at+=groups.x) {
        let slot=atomicLoad(&gi_work[gi_frame.offsets.w+at]);
        let hot=gi_hot(slot);
        let cold=gi_cold(slot);
        let state=gi_table[cold+GI_C_STATE];
        let updates=gi_table[hot+GI_HOT_ACC_COUNT];
        let rays=select(min(max(gi_word(GI_H_ACC_RAYS),1u),64u),64u,updates==0u);
        for(var i=0u;i<9u;i+=1u) { gi_cell_sh[i]=vec3<f32>(0.0); }
        if lane<rays {
            let c=gi_cell(slot);
            let n=gi_unpack_normal(gi_table[cold+GI_C_NORMAL]);
            let q=gi_point(cold);
            let frame=gi_basis(n);
            let origin=q+n*gi_origin_offset(q,c.edge);
            let secondary=(state&GI_SECONDARY)!=0u;
            let tertiary=(state&GI_TERTIARY)!=0u;
            let band=select(select(GI_BAND_SECONDARY,GI_BAND_TERTIARY,secondary),0u,!requests || tertiary);
            // A rotation and a jitter drawn per update: over its updates a cell meets every direction.
            let seed=gi_mix(slot^(gi_word(GI_H_FRAME)*0x9e3779b9u));
            let turn=f32(seed>>8u)*(6.283185307179586/16777216.0);
            let jitter=f32(gi_mix(seed^lane)>>8u)*(1.0/16777216.0);
            let z=1.0-(f32(lane)+jitter)/f32(rays);
            let r=sqrt(max(1.0-z*z,0.0));
            let phi=f32(lane)*2.399963229728653+turn;
            let d=frame*vec3<f32>(r*cos(phi),r*sin(phi),z);
            gi_hit_spread=gi_header_f32(GI_H_HIT_SPREAD);
            let h=gi_closest(origin,d,0.0,1e20);
            var radiance=vec3<f32>(0.0);
            if !h.hit {
                radiance=gi_sky_absolute(gi_sky(d));
            } else {
                let o=gi_hit_radiance(origin,d,h,band,tertiary);
                if o.w>GI_HIT_NONE { radiance=o.rgb; }
            }
            gi_sh_basis(d);
            let w=6.283185307179586/f32(rays);
            for(var i=0u;i<9u;i+=1u) { gi_cell_sh[i]=radiance*(w*gi_y[i]); }
        }
        // The lanes' sums into lane 0 in a fixed tree order.
        for(var i=0u;i<9u;i+=1u) {
            gi_wg_acc[(i*3u)*64u+lane]=gi_cell_sh[i].x;
            gi_wg_acc[(i*3u+1u)*64u+lane]=gi_cell_sh[i].y;
            gi_wg_acc[(i*3u+2u)*64u+lane]=gi_cell_sh[i].z;
        }
        workgroupBarrier();
        for(var stride=32u;stride>0u;stride/=2u) {
            if lane<stride {
                for(var j=0u;j<27u;j+=1u) { gi_wg_acc[j*64u+lane]+=gi_wg_acc[j*64u+lane+stride]; }
            }
            workgroupBarrier();
        }
        if lane==0u {
            let scale=bitcast<f32>(gi_table[hot+1u]);
            var sample:array<vec3<f32>,9>;
            for(var i=0u;i<9u;i+=1u) {
                sample[i]=vec3<f32>(gi_wg_acc[(i*3u)*64u],gi_wg_acc[(i*3u+1u)*64u],gi_wg_acc[(i*3u+2u)*64u]);
            }
            let l=max(dot(sample[0],GI_ACC_LUMA),0.0);
            var fast=bitcast<f32>(gi_table[hot+GI_HOT_ACC_FAST]);
            var mean=bitcast<f32>(gi_table[hot+GI_HOT_ACC_MEAN]);
            var moment=bitcast<f32>(gi_table[hot+GI_HOT_ACC_MOMENT]);
            var n=updates;
            var alpha=1.0;
            if n==0u {
                fast=l; mean=l; moment=l*l; n=1u;
            } else {
                let history=f32(max(gi_word(GI_H_ACC_HISTORY),1u));
                alpha=1.0/min(f32(n)+1.0,history);
                fast=mix(fast,l,0.5);
                let spread=2.0*sqrt(max(moment-mean*mean,0.0))+0.02*mean+1e-12;
                if abs(fast-mean)>spread {
                    alpha=max(alpha,0.5);
                    n=1u;
                    atomicAdd(&gi_work[GI_W_ACC_PULLS],1u);
                }
                mean=mix(mean,l,alpha);
                moment=mix(moment,l*l,alpha);
                n=min(n+1u,u32(history));
            }
            for(var i=0u;i<9u;i+=1u) {
                let old=vec3<f32>(gi_half(hot,3u*i),gi_half(hot,3u*i+1u),gi_half(hot,3u*i+2u))*scale;
                gi_cell_sh[i]=mix(old,sample[i],alpha);
            }
            gi_cell_sh[9]=vec3<f32>(0.0);
            gi_store_sh(slot);
            gi_table[hot+GI_HOT_BOUNCE]=0u;
            gi_table[hot+GI_HOT_ACC_FAST]=bitcast<u32>(fast);
            gi_table[hot+GI_HOT_ACC_MEAN]=bitcast<u32>(mean);
            gi_table[hot+GI_HOT_ACC_MOMENT]=bitcast<u32>(moment);
            gi_table[hot+GI_HOT_ACC_COUNT]=n;
            gi_queue_shade(slot);
            atomicAdd(&gi_work[GI_W_ACC_UPDATES],1u);
            if gi_word(GI_H_ACC_BUDGET)==0u { atomicAdd(&gi_work[GI_W_ACC_RAYS],rays); }
        }
        workgroupBarrier();
    }
}
