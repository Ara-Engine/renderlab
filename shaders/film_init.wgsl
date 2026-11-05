// ---- T1 reset frames: the history starts from the Interactive image ----
// A reset frame reads no history, so it is traced and shaded by the production Interactive
// kernel and Soft shading pipelines: its image is the Interactive image by construction
// (T-0), whatever code the driver generates for the temporal modules. This pass, appended to
// the temporal kernel module only, then starts every pixel's history from that image:
// mean = the stored HDR, weight 1, sequence index 0, and the anchor from the hit record.
struct FilmVec4s { data:array<vec4<f32>> };
struct FilmU32s { data:array<u32> };
@group(1) @binding(2) var<storage,read> film_vertices:binding_array<FilmVec4s,PRIMARY_BATCHES>;
@group(1) @binding(3) var<storage,read> film_indices:binding_array<FilmU32s,PRIMARY_BATCHES>;
@group(0) @binding(18) var film_hdr:texture_2d<f32>;
@group(0) @binding(27) var film_init_colour:texture_storage_2d<rgba32float,write>;
@group(0) @binding(28) var film_init_anchor:texture_storage_2d<rg32uint,write>;
fn film_position(batch:u32,index:u32)->vec3<f32> {
    return film_vertices[batch].data[film_indices[batch].data[index]*3u].xyz;
}
@compute @workgroup_size(8,4) fn film_init(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(primary_hit);
    if any(id.xy>=size) {return;}
    let p=vec2<i32>(id.xy);
    var placement=0u;
    var distance=0.0;
    // Uncovered trace tiles keep stale hit records; every pixel there is background.
    if textureLoad(primary_tiles,id.xy/PRIMARY_TILE,0).x!=0u {
        let record=textureLoad(primary_hit,p,0);
        if record.x!=0u {
            let slot=record.x-1u;
            let batch=primary_placements[slot]&0x1fffffffu;
            let i=record.y*3u;
            let b=bitcast<vec2<f32>>(record.zw);
            placement=record.x;
            distance=film_view_distance(film_position(batch,i),film_position(batch,i+1u),
                film_position(batch,i+2u),vec3<f32>(1.0-b.x-b.y,b),slot);
        }
    }
    textureStore(film_init_colour,p,vec4<f32>(textureLoad(film_hdr,p,0).rgb,1.0));
    textureStore(film_init_anchor,p,vec4<u32>(placement&0x7fffffu,bitcast<u32>(distance),0u,0u));
}
// ---- end of the T1 reset pass ----
