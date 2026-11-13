// Ordered dispatches reserve 80% for pixels; the background can use unused visible tickets.
fn gi_priority_visible_limit(cap:u32) -> u32 { return cap-cap/5u; }
fn gi_priority_ticket(limit:u32) -> bool {
    var used=atomicLoad(&gi_work[47u]);
    loop {
        if used>=limit { return false; }
        let result=atomicCompareExchangeWeak(&gi_work[47u],used,used+1u);
        if result.exchanged { return true; }
        used=result.old_value;
    }
}
fn gi_priority_claim_admit(band:u32) -> bool {
    let cap=gi_word(33u);
    if cap==0u { return true; }
    return gi_priority_ticket(select(cap,gi_priority_visible_limit(cap),band>=GI_BAND_PIXEL));
}
var<private> gi_priority_phase:u32;
fn gi_priority_class(slot:u32) -> u32 {
    if gi_table[gi_cold(slot)+GI_C_SEEN]!=gi_word(GI_H_FRAME) { return 2u; }
    // A gathered zero has the nonzero GI_BOUNCE_NONE sentinel; zero means no value yet.
    return select(1u,0u,gi_table[gi_hot(slot)+GI_HOT_BOUNCE]==0u);
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
