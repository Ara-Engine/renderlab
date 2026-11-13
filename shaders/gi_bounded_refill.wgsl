// A ticket per candidate, not per ray. Counter 47 is reset with the other frame counters.
// The existing slot rotation selects candidates; this ceiling also bounds skewed buckets
// and the visible/background budget's extra quarter. Rejected cells are requeued unchanged.
fn gi_bounded_gather_admit() -> bool {
    return atomicAdd(&gi_work[47u],1u)<gi_word(32u);
}
