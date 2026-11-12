// ---- PANE_COMMON ----
// Per-pixel pane shading on the ara-gpu port (transparency_glass.rs `pane_*`), appended to
// the glass module `transparency_gpu::glass_source` composes. One pane at a time, far to
// near, as the glass raster draws them: a four-sample raster of the pane's triangle ids
// (each sample the last triangle in draw order that covers it, as on the glass raster's
// four-sample target), then one dispatch shades each distinct triangle under a pixel's
// samples once at the pixel centre and averages the samples against the colour behind the
// pane, as that target's resolve does. Cost follows covered pixels, not triangles.
// ---- end PANE_COMMON ----

// ---- PANE_RASTER ----
// The triangle under each sample, plus one, from the glass entry's vertex stage (`vs`); the
// glass entry's two discards apply as it takes them, per fragment at the pixel centre.
// With depth testing off the last triangle in draw order wins each sample.
@fragment fn tp_pane_id_fs(input:RasterInput, @builtin(front_facing) front:bool, @builtin(primitive_index) primitive:u32) -> @location(0) u32 {
    let source_front=front==(input.winding>0.0);
    if material.double_sided==0u && !source_front { discard; }
    let pixel=vec2<i32>(input.position.xy);
    let depth=dot(input.world-tp.eye.xyz,tp.forward.xyz);
    if depth>textureLoad(tp_depth,pixel,0).x { discard; }
    return primitive+1u;
}
// ---- end PANE_RASTER ----

// ---- PANE_COMPUTE ----
// rect: the pane's pixels [x0, y0, x1, y1). info.x: the instance slot of its placement.
struct PanePixels { rect:vec4<u32>, info:vec4<u32> };
@group(0) @binding(77) var<storage,read> pane:PanePixels;
@group(0) @binding(70) var<storage,read> pane_vertices:array<vec4<f32>>;
@group(0) @binding(71) var<storage,read> pane_indices:array<u32>;
// The pane's four-sample id raster.
@group(0) @binding(72) var pane_ids:texture_multisampled_2d<u32>;
@group(0) @binding(76) var pane_out:texture_storage_2d<rgba16float,write>;
// Which sign of a triangle's area in pixels the pipeline's front faces have.
const PANE_FRONT_NEGATIVE:bool=true;
// The raster's dpdx and dpdy of the interpolants the glass entry differentiates.
struct PaneDerivatives { world_x:vec3<f32>, world_y:vec3<f32>, normal_x:vec3<f32>, normal_y:vec3<f32>, uv_x:vec2<f32>, uv_y:vec2<f32> };
// One vertex as the raster's vertex stage outputs it: 48-byte records (position, normal,
// uv, colour) and the pane's placement.
fn pane_vertex(index:u32)->VertexOutput {
    let a=pane_vertices[index*3u];let b=pane_vertices[index*3u+1u];let c=pane_vertices[index*3u+2u];
    return vertex_output(VertexInput(a.xyz,vec3<f32>(a.w,b.xy),b.zw,c,pane.info.x));
}
// A clip position in pixels (y down), with its clip w.
fn pane_screen(clip:vec4<f32>)->vec3<f32> {
    let ndc=clip.xy/clip.w;
    return vec3<f32>((ndc.x*0.5+0.5)*tp.size.x,(0.5-ndc.y*0.5)*tp.size.y,clip.w);
}
// Perspective-correct barycentrics of pixel point p in the projected triangle s.
fn pane_bary(s0:vec3<f32>,s1:vec3<f32>,s2:vec3<f32>,p:vec2<f32>)->vec3<f32> {
    let area=(s1.x-s0.x)*(s2.y-s0.y)-(s2.x-s0.x)*(s1.y-s0.y);
    let l1=((p.x-s0.x)*(s2.y-s0.y)-(s2.x-s0.x)*(p.y-s0.y))/area;
    let l2=((s1.x-s0.x)*(p.y-s0.y)-(p.x-s0.x)*(s1.y-s0.y))/area;
    let l=vec3<f32>(1.0-l1-l2,l1,l2)/vec3<f32>(s0.z,s1.z,s2.z);
    return l/(l.x+l.y+l.z);
}
// The interpolants at barycentrics w, as the rasterizer interpolates them; flat ones from
// the first vertex.
fn pane_interpolate(v0:VertexOutput,v1:VertexOutput,v2:VertexOutput,w:vec3<f32>,p:vec2<f32>)->RasterInput {
    var input:RasterInput;
    input.position=vec4<f32>(p,0.0,1.0);
    input.color=v0.color*w.x+v1.color*w.y+v2.color*w.z;
    input.uv=v0.uv*w.x+v1.uv*w.y+v2.uv*w.z;
    input.normal=v0.normal*w.x+v1.normal*w.y+v2.normal*w.z;
    input.winding=v0.winding;
    input.world=v0.world*w.x+v1.world*w.y+v2.world*w.z;
    input.tangent=v0.tangent*w.x+v1.tangent*w.y+v2.tangent*w.z;
    input.handedness=v0.handedness;
    return input;
}
// Triangle t shaded at pixel p's centre by the glass entry's own body (`tp_pane_fragment`,
// derived from `fs_transparent`). Derivatives are the fine ones of p's 2x2 quad: the
// difference to the horizontal and vertical neighbour on the same triangle's plane.
// Alpha below zero: the glass entry discarded the fragment.
fn pane_shade_triangle(t:u32,p:vec2<u32>)->vec4<f32> {
    let v0=pane_vertex(pane_indices[t*3u]);
    let v1=pane_vertex(pane_indices[t*3u+1u]);
    let v2=pane_vertex(pane_indices[t*3u+2u]);
    let s0=pane_screen(v0.position);let s1=pane_screen(v1.position);let s2=pane_screen(v2.position);
    let c=vec2<f32>(p)+vec2<f32>(0.5);
    let sx=select(1.0,-1.0,(p.x&1u)==1u);
    let sy=select(1.0,-1.0,(p.y&1u)==1u);
    let here=pane_interpolate(v0,v1,v2,pane_bary(s0,s1,s2,c),c);
    let across=pane_interpolate(v0,v1,v2,pane_bary(s0,s1,s2,c+vec2<f32>(sx,0.0)),c);
    let down=pane_interpolate(v0,v1,v2,pane_bary(s0,s1,s2,c+vec2<f32>(0.0,sy)),c);
    let d=PaneDerivatives(sx*(across.world-here.world),sy*(down.world-here.world),
        sx*(across.normal-here.normal),sy*(down.normal-here.normal),
        sx*(across.uv-here.uv),sy*(down.uv-here.uv));
    // The raster's front_facing, from the triangle's winding in pixels (y down).
    let area=(s1.x-s0.x)*(s2.y-s0.y)-(s2.x-s0.x)*(s1.y-s0.y);
    let triangle=RasterTriangle(array<vec3<f32>,3>(v0.receiver_vertex,v1.receiver_vertex,v2.receiver_vertex));
    return tp_pane_fragment(here,triangle,(area<0.0)==PANE_FRONT_NEGATIVE,d);
}
// One pixel of the pane's rectangle: the mean of its four samples, each the triangle
// under it shaded or the colour behind the pane. A pixel no sample covers is left as is
// (the target already holds the colour behind).
@compute @workgroup_size(8,8) fn tp_pane_shade(@builtin(global_invocation_id) id:vec3<u32>) {
    let p=pane.rect.xy+id.xy;
    if any(p>=pane.rect.zw) { return; }
    let pixel=vec2<i32>(p);
    let ids=array<u32,4>(textureLoad(pane_ids,pixel,0).x,textureLoad(pane_ids,pixel,1).x,
        textureLoad(pane_ids,pixel,2).x,textureLoad(pane_ids,pixel,3).x);
    if (ids[0]|ids[1]|ids[2]|ids[3])==0u { return; }
    let behind=textureLoad(tp_color,pixel,0);
    var seen=array<u32,4>();
    var shaded=array<vec4<f32>,4>();
    var count=0u;
    var sum=vec4<f32>(0.0);
    for(var k=0u;k<4u;k+=1u) {
        let id=ids[k];
        if id==0u { sum+=behind; continue; }
        var colour=vec4<f32>(0.0);
        var found=false;
        for(var j=0u;j<count;j+=1u) {
            if seen[j]==id { colour=shaded[j]; found=true; }
        }
        if !found {
            colour=pane_shade_triangle(id-1u,p);
            seen[count]=id;shaded[count]=colour;count+=1u;
        }
        sum+=select(colour,behind,colour.w<0.0);
    }
    textureStore(pane_out,pixel,sum*0.25);
}
// ---- end PANE_COMPUTE ----
