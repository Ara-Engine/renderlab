// Ordered dispatches reserve 80% for pixels; the background can use unused visible tickets.
fn gi_priority_visible_limit(cap:u32) -> u32 { return cap-cap/5u; }
fn gi_priority_ticket(limit:u32) -> bool {
    return atomicAdd(&gi_work[47u],1u)<limit;
}
@compute @workgroup_size(1) fn gi_priority_gather_clamp() {
    atomicMin(&gi_work[47u],gi_priority_visible_limit(gi_word(32u)));
}
@compute @workgroup_size(1) fn gi_priority_claim_reset() {
    atomicStore(&gi_work[47u],atomicLoad(&gi_work[GI_W_CREATED]));
}
fn gi_priority_claim_admit(band:u32) -> bool {
    let cap=gi_word(33u);
    var admitted=true;
    if cap!=0u { admitted=gi_priority_ticket(select(cap,gi_priority_visible_limit(cap),band>=GI_BAND_PIXEL)); }
    if admitted { atomicAdd(&gi_work[gi_priority_arena()+13u],1u); }
    return admitted;
}
var<private> gi_priority_phase:u32;
const GI_PRIORITY_PENDING_VISIBLE:u32=2048u;
fn gi_priority_raw_bounce(slot:u32) -> u32 {
    if gi_filter_on(GI_FILTER_BOUNCE) { return gi_raw[gi_raw_at(slot,GI_HOT_BOUNCE)]; }
    return gi_table[gi_hot(slot)+GI_HOT_BOUNCE];
}
fn gi_priority_class(slot:u32) -> u32 {
    if gi_table[gi_cold(slot)+GI_C_SEEN]!=gi_word(GI_H_FRAME) { return 2u; }
    // A gathered zero has the nonzero GI_BOUNCE_NONE sentinel; zero means no value yet.
    let bounce=select(gi_table[gi_hot(slot)+GI_HOT_BOUNCE],gi_priority_raw_bounce(slot),gi_word(41u)!=0u);
    return select(1u,0u,bounce==0u);
}
fn gi_priority_gather_admit() -> bool {
    let cap=gi_word(32u);
    return gi_priority_ticket(select(gi_priority_visible_limit(cap),cap,gi_priority_phase==2u));
}
@compute @workgroup_size(64) fn gi_priority_claim_visible(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    gi_priority_claim_body(id,groups,true);
}
@compute @workgroup_size(64) fn gi_priority_claim_background(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    gi_priority_claim_body(id,groups,false);
}
@compute @workgroup_size(64) fn gi_priority_gather_empty(@builtin(workgroup_id) group:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    gi_priority_phase=0u;
    gi_priority_gather_body(group,groups,lane);
}
@compute @workgroup_size(64) fn gi_priority_gather_later(@builtin(workgroup_id) group:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    gi_priority_phase=1u;
    gi_priority_gather_body(group,groups,lane);
}
@compute @workgroup_size(64) fn gi_priority_gather_background(@builtin(workgroup_id) group:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    gi_priority_phase=2u;
    gi_priority_gather_body(group,groups,lane);
}
