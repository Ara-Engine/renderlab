// ---- T1 temporal trace kernel additions: tile flags, cold samples and the reset pass ----
// Appended to the temporal kernel module only (film_accumulation.rs kernel_source).

// This frame's tile flags: nonzero while the tile keeps history, i.e. a camera ray in it has
// hit since the reset and it meets the scene rectangle. Outside the rectangle no ray can hit
// within 2 px, so the film there is the background and keeps none.
@group(0) @binding(30) var film_tiles_out:texture_storage_2d<r32uint,write>;
// Moving frames only, covered tiles: the trace's current sample per pixel (row-major), the
// sequence index it used in bits 0..7 and its view distance as f32 bits with the low 8
// mantissa bits cleared (relative precision 3e-5), which the shading's reverse test reads.
// A buffer, since the trace already binds the four storage textures traced devices allow.
@group(0) @binding(31) var<storage,read_write> film_sample_out:array<u32>;
// The trace's first invocation per tile records whether the tile keeps history from now on.
fn film_keep_tile(tile:vec2<u32>,covered:u32) {
    let scene=primary_frame.scene;
    let lo=tile*PRIMARY_TILE;
    let inside=all(lo<scene.zw) && all(lo+PRIMARY_TILE>scene.xy);
    let hit=textureLoad(film_tiles_in,tile,0).x!=0u || covered!=0u;
    let keep=film_frame.materialize!=0u || (inside && hit);
    textureStore(film_tiles_out,tile,vec4<u32>(u32(keep)));
}

// The surface a trace shows (film_hit_surface on its returned triangle, or the background).
fn film_trace_surface(trace:PrimaryTrace,id:vec2<u32>,size:vec2<u32>)->FilmSurface {
    if trace.record.x==0u {return film_miss_surface(id,size);}
    let b=bitcast<vec2<f32>>(trace.record.zw);
    return film_hit_surface(trace.positions[0],trace.positions[1],trace.positions[2],
        vec3<f32>(1.0-b.x-b.y,b),trace.record.x-1u);
}

// The camera ray of the pixel's sample. Under motion a sample whose fetch finds no inner tap
// on its surface by the forward test is cold: it is traced again at the pixel centre with
// index 0, so a revealed pixel's first sample is Interactive's.
fn film_closest(id:vec2<u32>,size:vec2<u32>)->PrimaryTrace {
    var trace=primary_closest(id,size);
    if film_frame.moving!=0u && film_frame.reset==0u && film_k!=0u
        && !film_forward_inner(id,film_trace_surface(trace,id,size),size) {
        film_begin(0u,id,size.x);
        trace=primary_closest(id,size);
    }
    return trace;
}
// Records the pixel's current sample for the shading (moving frames).
fn film_record_sample(p:vec2<i32>,trace:PrimaryTrace,size:vec2<u32>) {
    if film_frame.moving==0u {return;}
    var d=0.0;
    if trace.record.x!=0u {d=film_distance(film_trace_surface(trace,vec2<u32>(p),size));}
    film_sample_out[u32(p.x)+u32(p.y)*size.x]=(bitcast<u32>(d)&0xffffff00u)|film_k;
}

// A reset frame reads no history, so it is traced and shaded by the production Interactive
// kernel and Soft shading pipelines: its image is the Interactive image by construction
// (T-0), whatever code the driver generates for the temporal modules. This pass then starts
// the history of every tile with a hit from that image: mean = the stored HDR, weight 1,
// sequence index 0, and the anchor from the hit record. Other tiles keep none.
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
    let tile=id.xy/PRIMARY_TILE;
    // Uncovered trace tiles keep stale hit records; every pixel there is background.
    let covered=textureLoad(primary_tiles,tile,0).x!=0u;
    let keep=covered || film_frame.materialize!=0u;
    // A tile's first pixel is always inside the image.
    if all(id.xy==tile*PRIMARY_TILE) {textureStore(film_tiles_out,tile,vec4<u32>(u32(keep)));}
    if !keep {return;}
    let p=vec2<i32>(id.xy);
    var anchor=vec2<u32>(0u);
    if covered {
        let record=textureLoad(primary_hit,p,0);
        if record.x!=0u {
            let slot=record.x-1u;
            let batch=primary_placements[slot]&0x1fffffffu;
            let i=record.y*3u;
            let b=bitcast<vec2<f32>>(record.zw);
            let s=film_hit_surface(film_position(batch,i),film_position(batch,i+1u),
                film_position(batch,i+2u),vec3<f32>(1.0-b.x-b.y,b),slot);
            anchor=vec2<u32>((s.placement&0x7fffffu)|(s.side<<23u),bitcast<u32>(film_distance(s)));
        }
    }
    textureStore(film_init_colour,p,vec4<f32>(textureLoad(film_hdr,p,0).rgb,1.0));
    textureStore(film_init_anchor,p,vec4<u32>(anchor,0u,0u));
}
// ---- end of the T1 kernel additions ----
