// Debug-only aggregate counters; no per-pixel image or history modification.
@group(0) @binding(80) var<storage,read_write> sunprobe_counts:array<atomic<u32>,289>;
var<private> sunprobe_receiver:u32;
var<private> sunprobe_rays:u32;
var<private> sunprobe_budget:u32;
var<private> sunprobe_prefix_agrees:u32;
fn sunprobe_bucket(n:u32)->u32 {
    if n==8u {return 0u;} if n==16u {return 1u;}
    if n==32u {return 2u;} if n==256u {return 3u;}
    return 4u;
}
fn sunprobe_write(visible:u32) {
    atomicAdd(&sunprobe_counts[0],1u);
    if sunprobe_receiver==0u {return;}
    atomicAdd(&sunprobe_counts[1],1u);
    atomicAdd(&sunprobe_counts[2],sunprobe_rays);
    atomicAdd(&sunprobe_counts[3u+sunprobe_bucket(sunprobe_budget)],1u);
    atomicAdd(&sunprobe_counts[8],u32(tr1_held_probes!=0xffffffffu));
    atomicAdd(&sunprobe_counts[9],u32(visible==FRESH_SKIP));
    atomicAdd(&sunprobe_counts[10u+sunprobe_bucket(tr1_rays)],1u);
    atomicAdd(&sunprobe_counts[15],u32(visible!=FRESH_SKIP && visible==tr1_rays));
    atomicAdd(&sunprobe_counts[16],u32(visible==0u));
    atomicAdd(&sunprobe_counts[17],u32(visible!=FRESH_SKIP && visible!=0u && visible!=tr1_rays));
    atomicAdd(&sunprobe_counts[18],sunprobe_budget);
    let eligible=tr1_held_probes!=0xffffffffu;
    atomicAdd(&sunprobe_counts[19],u32(eligible && sunprobe_budget==32u));
    atomicAdd(&sunprobe_counts[20],u32(eligible && sunprobe_budget==32u && sunprobe_prefix_agrees!=0u));
    atomicAdd(&sunprobe_counts[32u+min(sunprobe_rays,256u)],1u);
}
