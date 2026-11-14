// SUNRT tile vote, appended to the reconstruction's Interactive trace modules by
// temporal_reconstruction.rs `trace_source_with` (TR1_TILE_PROBES is prepended there).
// Each 8x4 trace tile (one warp on NVIDIA) first traces TR1_TILE_PROBES directions of its
// receivers' budget. When every probe of every receiver in the tile agrees (all clear, or
// all blocked) the tile is uniformly lit or shadowed: each receiver keeps its probe count
// over TR1_TILE_PROBES rays (0 or 256). Any disagreement, anywhere in the tile, resumes
// every receiver at direction TR1_TILE_PROBES, so penumbra tiles trace main's full budget in
// main's direction order and their estimate is main's exactly.
// tr1_tile_state: 0 normal, 1 pause at the probes, 2 paused (resume), 3 tile unanimous.
var<private> tr1_tile_state:u32=0u;
var<private> tr1_tile_count:u32=0u;
var<workgroup> tr1_tile_votes:atomic<u32>;
// Called once per invocation in uniform control flow, before the entry's estimate. Votes:
// 1 every probe blocked, 2 every probe clear, 4 anything else; receivers that skip (outside
// every cascade) or miss do not vote.
fn tr1_tile_vote(hit:bool,receiver:Receiver,pixel:vec2<u32>,width:u32) {
    var vote=0u;
    if hit {
        tr1_tile_state=1u;
        let v=fresh_estimate(receiver,pixel,width,0u);
        if tr1_tile_state==2u {
            tr1_tile_count=v;
            vote=select(select(4u,2u,v==TR1_TILE_PROBES),1u,v==0u);
        } else {tr1_tile_state=0u;}
    }
    if vote!=0u {atomicOr(&tr1_tile_votes,vote);}
    workgroupBarrier();
    let votes=atomicLoad(&tr1_tile_votes);
    if tr1_tile_state==2u && (votes==1u || votes==2u) {tr1_tile_state=3u;}
}
fn tr1_tile_vote_trace(trace:PrimaryTrace,pixel:vec2<u32>,width:u32) {
    var receiver:Receiver;
    if trace.record.x!=0u {receiver=primary_receiver(trace.point,trace.positions,trace.record.x-1u,vec2<i32>(pixel));}
    tr1_tile_vote(trace.record.x!=0u,receiver,pixel,width);
}
