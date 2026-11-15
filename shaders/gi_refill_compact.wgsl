// The ray-item arena is unused until sky setup. Only capped candidate lists live here;
// the existing per-slot claim scratch records class/rank and deduplicates queue entries.
// First fill can serve the entire current inflow, even when it exceeds a refresh cap.
fn gi_priority_arena() -> u32 { return gi_frame.requests.z; }
fn gi_priority_list(kind:u32,rank:u32) -> u32 {
    if kind==0u { return gi_priority_arena()+16u+rank; }
    return gi_priority_arena()+16u+gi_frame.offsets.x+(kind-1u)*gi_word(32u)+rank;
}
fn gi_priority_snapshot(slot:u32) -> bool {
    loop {
        let old=atomicCompareExchangeWeak(&gi_work[GI_W_CLAIMS+slot],0u,0xffffffffu);
        if old.exchanged { return true; }
        if old.old_value!=0u { return false; }
    }
    return false;
}
@compute @workgroup_size(64) fn gi_priority_compact(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let count=min(atomicLoad(&gi_work[gi_frame.bounce.x]),gi_frame.offsets.x);
    for(var i=id.x;i<count;i+=groups.x*64u) {
        let slot=atomicLoad(&gi_work[gi_frame.bounce.x+1u+i]);
        let state=gi_table[gi_cold(slot)+GI_C_STATE];
        // Even a temporarily partial/ineligible queued cell must forget that it was
        // visible when it goes off screen, so its return counts as new inflow.
        if (state&GI_PRIORITY_PENDING_VISIBLE)!=0u
            && (gi_table[gi_cold(slot)+GI_C_SEEN]!=gi_word(GI_H_FRAME) || gi_priority_raw_bounce(slot)!=0u) {
            gi_table[gi_cold(slot)+GI_C_STATE]=state&~GI_PRIORITY_PENDING_VISIBLE;
        }
        if gi_table[gi_hot(slot)]==0u || (state&GI_BQUEUED)==0u
            || (state&(GI_TRACED|GI_ORPHAN|GI_PARTIAL))!=GI_TRACED { continue; }
        if !gi_priority_snapshot(slot) { continue; }
        let kind=gi_priority_class(slot);
        let visible=gi_table[gi_cold(slot)+GI_C_SEEN]==gi_word(GI_H_FRAME);
        let raw_empty=gi_priority_raw_bounce(slot)==0u;
        let raw_class=select(select(1u,0u,raw_empty),2u,!visible);
        atomicAdd(&gi_work[gi_priority_arena()+10u+raw_class],1u);
        if visible && raw_empty {
            if (state&GI_PRIORITY_PENDING_VISIBLE)==0u { atomicAdd(&gi_work[gi_priority_arena()+7u],1u); }
            gi_table[gi_cold(slot)+GI_C_STATE]=state|GI_PRIORITY_PENDING_VISIBLE;
            if gi_table[gi_hot(slot)+GI_HOT_BOUNCE]!=0u { atomicAdd(&gi_work[gi_priority_arena()+8u],1u); }
        } else {
            gi_table[gi_cold(slot)+GI_C_STATE]=state&~GI_PRIORITY_PENDING_VISIBLE;
        }
        let rank=atomicAdd(&gi_work[gi_priority_arena()+kind],1u);
        atomicStore(&gi_work[GI_W_CLAIMS+slot],(kind<<30u)|(rank+1u));
        if kind==0u || rank<gi_word(32u) { atomicStore(&gi_work[gi_priority_list(kind,rank)],slot); }
    }
}
@compute @workgroup_size(1) fn gi_priority_select() {
    let at=gi_priority_arena();let cap=gi_word(32u);
    if gi_word(41u)==0u {
        let visible=gi_priority_visible_limit(cap);
        let first=min(atomicLoad(&gi_work[at]),visible);
        let refresh=min(atomicLoad(&gi_work[at+1u]),visible-first);
        let background=min(atomicLoad(&gi_work[at+2u]),cap-first-refresh);
        atomicStore(&gi_work[at+3u],first);
        atomicStore(&gi_work[at+4u],refresh);
        atomicStore(&gi_work[at+5u],background);
        atomicStore(&gi_work[at+6u],first+refresh+background);
        return;
    }
    // A soft gather target from the previous frame's measured cost. First fill's floor
    // serves at least the current on-screen inflow; time overruns never starve that floor.
    let inflow=atomicLoad(&gi_work[at+7u]);
    let first_cap=max(gi_word(35u),inflow);
    let empty=min(atomicLoad(&gi_work[at]),first_cap);
    let first_ms=f32(empty)*bitcast<f32>(gi_word(36u));
    let left_ms=max(0.0,bitcast<f32>(gi_word(38u))-first_ms);
    let timed_cap=u32(min(floor(left_ms/max(bitcast<f32>(gi_word(37u)),1e-9)),f32(cap)));
    let refresh_cap=min(timed_cap,gi_word(40u));
    let visible=gi_priority_visible_limit(refresh_cap);
    let later=min(atomicLoad(&gi_work[at+1u]),visible);
    let background=min(atomicLoad(&gi_work[at+2u]),refresh_cap-later);
    atomicStore(&gi_work[at+3u],empty);
    atomicStore(&gi_work[at+4u],later);
    atomicStore(&gi_work[at+5u],background);
    atomicStore(&gi_work[at+6u],empty+later+background);
    atomicStore(&gi_work[at+9u],select(0u,1u,first_ms>bitcast<f32>(gi_word(38u))));
}
var<workgroup> gi_priority_appends:atomic<u32>;
var<workgroup> gi_priority_append_base:u32;
@compute @workgroup_size(64) fn gi_priority_defer(@builtin(workgroup_id) group:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    if lane==0u {gi_wg_count=min(atomicLoad(&gi_work[gi_frame.bounce.x]),gi_frame.offsets.x);}
    let count=workgroupUniformLoad(&gi_wg_count);
    for(var head=group.x*64u;head<count;head+=groups.x*64u) {
        if lane==0u {atomicStore(&gi_priority_appends,0u);}
        workgroupBarrier();
        var deferred=false;var rank=0u;var slot=0u;
        if head+lane<count {
            slot=atomicLoad(&gi_work[gi_frame.bounce.x+1u+head+lane]);
            let marker=atomicExchange(&gi_work[GI_W_CLAIMS+slot],0u);
            if marker!=0u {
                let kind=marker>>30u;
                let ordinal=(marker&0x3fffffffu)-1u;
                if ordinal>=atomicLoad(&gi_work[gi_priority_arena()+3u+kind]) {
                    let cold=gi_cold(slot);let state=gi_table[cold+GI_C_STATE];
                    let cleared=state&~GI_BQUEUED;
                    let gathers=max((state&GI_GATHER_MASK)>>GI_GATHER_SHIFT,1u);
                    let left=max((cleared&GI_GATHER_MASK)>>GI_GATHER_SHIFT,min(gathers,7u));
                    gi_table[cold+GI_C_STATE]=(cleared&~GI_GATHER_MASK)|(left<<GI_GATHER_SHIFT)|GI_BQUEUED;
                    rank=atomicAdd(&gi_priority_appends,1u);deferred=true;
                }
            }
        }
        workgroupBarrier();
        if lane==0u {
            let n=atomicLoad(&gi_priority_appends);
            gi_priority_append_base=atomicAdd(&gi_work[gi_frame.bounce.y],n);
            atomicAdd(&gi_work[GI_W_BQUEUE],n);
            atomicAdd(&gi_work[GI_W_GATHER_DEFERRED],n);
        }
        workgroupBarrier();
        if deferred {
            let index=gi_priority_append_base+rank;
            if index<gi_frame.offsets.x {atomicStore(&gi_work[gi_frame.bounce.y+1u+index],slot);}
        }
        workgroupBarrier();
    }
}
var<workgroup> gi_priority_compact_info:vec4<u32>;
fn gi_priority_gather_compacted(group:vec3<u32>,groups:vec3<u32>,lane:u32,first:bool) {
    let at=gi_priority_arena();
    if lane==0u {gi_priority_compact_info=vec4<u32>(atomicLoad(&gi_work[at+3u]),atomicLoad(&gi_work[at+4u]),atomicLoad(&gi_work[at+5u]),atomicLoad(&gi_work[at+6u]));}
    let info=workgroupUniformLoad(&gi_priority_compact_info);
    let begin=select(info.x,0u,first);
    let end=select(info.w,info.x,first);
    for(var i=begin+group.x;i<end;i+=groups.x) {
        var kind=0u;var ordinal=i;
        if i>=info.x {kind=1u;ordinal=i-info.x;}
        if i>=info.x+info.y {kind=2u;ordinal=i-info.x-info.y;}
        if lane==0u {gi_wg_n=atomicLoad(&gi_work[gi_priority_list(kind,ordinal)]);}
        let slot=workgroupUniformLoad(&gi_wg_n);
        gi_priority_gather_cell(slot,lane);
    }
}
@compute @workgroup_size(64) fn gi_priority_gather_first(@builtin(workgroup_id) group:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    gi_priority_gather_compacted(group,groups,lane,true);
}
@compute @workgroup_size(64) fn gi_priority_gather_refresh(@builtin(workgroup_id) group:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    gi_priority_gather_compacted(group,groups,lane,false);
}
// Defer the entire queue without reading cell records or casting any gather rays.
@compute @workgroup_size(64) fn gi_priority_hold_queue(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let count=min(atomicLoad(&gi_work[gi_frame.bounce.x]),gi_frame.offsets.x);
    if id.x==0u {
        atomicStore(&gi_work[gi_frame.bounce.y],count);
        atomicAdd(&gi_work[GI_W_BQUEUE],count);
        atomicAdd(&gi_work[GI_W_GATHER_DEFERRED],count);
    }
    for(var i=id.x;i<count;i+=groups.x*64u) {
        atomicStore(&gi_work[gi_frame.bounce.y+1u+i],atomicLoad(&gi_work[gi_frame.bounce.x+1u+i]));
    }
}
