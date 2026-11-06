// Scalar words keep the per-pixel capacity below a 128 MiB binding at 1080p.
struct PrimaryRepairItem {pixel:u32,placement:u32,primitive:u32,u:u32,v:u32,px:u32,py:u32,pz:u32,fallback:u32};
struct PrimaryRepairQueue {x:u32,y:u32,z:u32,count:atomic<u32>,items:array<PrimaryRepairItem>};
struct PrimaryRepairInput {x:u32,y:u32,z:u32,count:u32,items:array<PrimaryRepairItem>};
struct PrimaryRepairArgs {x:atomic<u32>,y:atomic<u32>,z:u32,pad:u32};
struct PrimaryRetryQueue {x:atomic<u32>,y:atomic<u32>,z:u32,count:atomic<u32>,items:array<u32>};
struct PrimaryRetryInput {x:u32,y:u32,z:u32,count:u32,items:array<u32>};
