// Test-only local footprint verification (src/film_accumulation_tests.rs), appended to the
// temporal kernel module: the film visibility of one pixel's centre surface over a G x G
// stratified grid of jittered positions covering the whole footprint, corners included, each
// with 32 independently salted production shadow rays (FRESH_DIRECTIONS = 32, eight lanes).
// Acceptance is the reference's: the centre's placement and depth along the camera forward
// within 2%. Opposite samples (any ray disagreeing with the expected solid state) are counted
// by where they lie: within 0.05 px of a corner, of an edge, or inside.
// spacing > 0 selects a micro grid of that spacing (pixels) centred on `centre` instead of the
// whole footprint.
struct FootprintTarget { pixel:vec2<u32>, size:vec2<u32>, placement:u32, depth:f32, expected:u32, pad:u32, eye:vec4<f32>, forward:vec4<f32>, centre:vec2<f32>, spacing:f32, pad2:u32 };
struct FootprintSums { accepted:atomic<u32>, visible:atomic<u32>, opposite:atomic<u32>, corner:atomic<u32>, edge:atomic<u32>, interior:atomic<u32>, nearest:atomic<u32>, rejected:atomic<u32> };
@group(2) @binding(0) var<storage,read> footprint_targets:array<FootprintTarget>;
@group(2) @binding(1) var<storage,read_write> footprint_sums:array<FootprintSums>;
// x: grid size G, y: the target this dispatch verifies.
@group(2) @binding(2) var<uniform> footprint_params:vec4<u32>;
var<private> film_footprint_seed:u32;
@compute @workgroup_size(8,4) fn film_footprint(@builtin(global_invocation_id) id:vec3<u32>) {
    let grid=footprint_params.x;
    if any(id.xy>=vec2<u32>(grid)) {return;}
    let index=footprint_params.y;
    let t=footprint_targets[index];
    let cell=id.x+id.y*grid;
    let h=fresh_hash(cell^0x51ed270bu);
    let q=vec2<f32>(f32(h>>8u),f32(fresh_hash(h)>>8u))*(1.0/16777216.0);
    film_sample=(vec2<f32>(id.xy)+q)/f32(grid);
    if t.spacing>0.0 {
        film_sample=t.centre+(vec2<f32>(id.xy)+q-vec2<f32>(f32(grid)*0.5))*t.spacing;
        if any(film_sample<vec2<f32>(0.0)) || any(film_sample>=vec2<f32>(1.0)) {return;}
    }
    film_rotation=0u;
    film_footprint_seed=1u+cell;
    let trace=primary_closest(t.pixel,t.size);
    let depth=dot(trace.point-t.eye.xyz,t.forward.xyz);
    if trace.record.x!=t.placement || abs(depth-t.depth)>0.02*t.depth {
        atomicAdd(&footprint_sums[index].rejected,1u);
        return;
    }
    let receiver=primary_receiver(trace.point,trace.positions,t.placement-1u,vec2<i32>(t.pixel));
    var visible=0u;
    for(var lane=0u;lane<8u;lane+=1u) {
        let v=fresh_estimate(receiver,t.pixel,t.size.x,lane);
        visible+=select(v,4u,v==FRESH_SKIP);
    }
    atomicAdd(&footprint_sums[index].accepted,1u);
    atomicAdd(&footprint_sums[index].visible,visible);
    let opposite=select(visible>0u,visible<32u,t.expected!=0u);
    if opposite {
        atomicAdd(&footprint_sums[index].opposite,1u);
        let d=min(film_sample,vec2<f32>(1.0)-film_sample);
        if max(d.x,d.y)<0.05 {atomicAdd(&footprint_sums[index].corner,1u);}
        else if min(d.x,d.y)<0.05 {atomicAdd(&footprint_sums[index].edge,1u);}
        else {atomicAdd(&footprint_sums[index].interior,1u);}
        // Nearest opposite sample to a pixel corner, in micro-pixels.
        atomicMin(&footprint_sums[index].nearest,u32(length(d)*1e6));
    }
}
