// SUNRT tile vote, appended to the reconstruction's Interactive trace modules by
// temporal_reconstruction.rs `trace_source_with` (TR1_TILE_PROBES is prepended there).
// Each 8x4 trace tile (one warp on NVIDIA) first traces a few directions of its receivers'
// budget: TR1_TILE_PROBES, or tr1_tile_hint_probes() when every receiver's history says the
// tile is uniformly lit or uniformly shadowed (sunrt-2; the held trace module's hint). When
// every probe of every receiver in the tile agrees (all clear, or all blocked) the tile keeps
// them (0 or 256). Any disagreement, anywhere in the tile, resumes every receiver after its
// probes, so penumbra tiles trace main's full budget in main's direction order and their
// estimate is main's exactly.
// tr1_tile_state: 0 normal, 1 pause at the probes, 2 paused (resume), 3 tile unanimous.
var<private> tr1_tile_state:u32=0u;
var<private> tr1_tile_count:u32=0u;
var<private> tr1_tile_pause:u32=TR1_TILE_PROBES;
var<workgroup> tr1_tile_votes:atomic<u32>;
var<workgroup> tr1_tile_hints:atomic<u32>;
// Called once per invocation in uniform control flow, before the entry's estimate. Hints:
// 1 no usable history, 2 history lit, 4 history shadowed. Votes: 1 every probe blocked,
// 2 every probe clear, 4 anything else. Receivers that skip (outside every cascade) or miss
// neither hint nor vote.
fn tr1_tile_vote(hit:bool,receiver:Receiver,slot:u32,pixel:vec2<u32>,width:u32) {
    if hit {atomicOr(&tr1_tile_hints,max(tr1_tile_hint(receiver,slot),1u));}
    workgroupBarrier();
    let hints=atomicLoad(&tr1_tile_hints);
    tr1_tile_pause=select(TR1_TILE_PROBES,tr1_tile_hint_probes(),hints==2u || hints==4u);
    var vote=0u;
    if hit {
        tr1_tile_state=1u;
        let v=fresh_estimate(receiver,pixel,width,0u);
        if tr1_tile_state==2u {
            tr1_tile_count=v;
            vote=select(select(4u,2u,v==tr1_tile_pause),1u,v==0u);
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
    tr1_tile_vote(trace.record.x!=0u,receiver,max(trace.record.x,1u)-1u,pixel,width);
}
// tile hint begin
// Without a sun-shadow history (the port, legacy modules): no hint, every tile probes TR1_TILE_PROBES.
fn tr1_tile_hint(receiver:Receiver,slot:u32)->u32 {return 0u;}
fn tr1_tile_hint_probes()->u32 {return TR1_TILE_PROBES;}
// tile hint end
