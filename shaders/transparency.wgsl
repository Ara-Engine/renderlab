// TP1 transparent pass (docs/m7-tp1.md, src/transparency_gpu.rs). Sections, each composed
// into its own module:
//   TP_FRAME      the frame uniform and the sun-visibility shadow map, in every module;
//   TP_DEPTH      opaque view depth from traced/visibility-buffer hit records or raster depth;
//   TP_SHADOW     the transparent receivers' sun shadow map (depth only, cached);
//   TP_GLASS      the transparent-material fragment entry, appended to the material's own
//                 composed raster program;
//   TP_PARTICLES  GPU particle simulation, sort and billboard drawing.
// Opaque depth is view-space depth along the camera's forward axis in metres, 1e30 for misses.

// ---- TP_FRAME ----
struct TpFrame {
    view_projection:mat4x4<f32>,
    // Inverse of the raster frame's projection, for raster depth.
    inverse:mat4x4<f32>,
    // World to the sun shadow map's clip space: xy in [-1, 1], z in [0, 1] from the sun.
    shadow:mat4x4<f32>,
    // xyz eye; w exposure.
    eye:vec4<f32>,
    forward:vec4<f32>,
    right:vec4<f32>,
    up:vec4<f32>,
    // width, height, 1/width, 1/height.
    size:vec4<f32>,
    // xyz toward the sun; w 1 when the shadow map holds this frame's casters.
    sun:vec4<f32>,
    // rgb sun illuminance (lux, unexposed); w: the map's texel size in metres.
    sun_lux:vec4<f32>,
    // The primary scene rectangle [x0, y0, x1, y1).
    scene:vec4<u32>,
    // x: particle step (s), y: shadow depth bias, z: shadow map size.
    misc:vec4<f32>,
};
@group(0) @binding(30) var<uniform> tp:TpFrame;
const TP_MISS:f32=1e30;
// ---- end TP_FRAME ----

// ---- TP_SUN ----
@group(0) @binding(34) var tp_shadow_map:texture_depth_2d;
@group(0) @binding(35) var tp_shadow_sampler:sampler_comparison;
// Sun visibility at a world point from the cached map: 3x3 PCF, 1 outside its footprint.
fn tp_sun_visibility(world:vec3<f32>) -> f32 {
    if tp.sun.w==0.0 { return 1.0; }
    let p=tp.shadow*vec4<f32>(world,1.0);
    let uv=p.xy*vec2<f32>(0.5,-0.5)+vec2<f32>(0.5);
    if any(uv<vec2<f32>(0.0)) || any(uv>vec2<f32>(1.0)) || p.z>1.0 { return 1.0; }
    let texel=1.0/tp.misc.z;
    var sum=0.0;
    for(var y=-1;y<=1;y+=1) {
        for(var x=-1;x<=1;x+=1) {
            sum+=textureSampleCompareLevel(tp_shadow_map,tp_shadow_sampler,uv+vec2<f32>(f32(x),f32(y))*texel,p.z-tp.misc.y);
        }
    }
    return sum/9.0;
}
// ---- end TP_SUN ----

// ---- TP_DEPTH ----
// Traced and visibility-buffer frames: hit records as primary_rays.wgsl describes them.
struct TpVec4s { data:array<vec4<f32>> };
struct TpU32s { data:array<u32> };
@group(1) @binding(0) var<storage,read> tp_models:array<mat4x4<f32>>;
@group(1) @binding(2) var<storage,read> tp_vertices:binding_array<TpVec4s,PRIMARY_BATCHES>;
@group(1) @binding(3) var<storage,read> tp_indices:binding_array<TpU32s,PRIMARY_BATCHES>;
@group(1) @binding(5) var<storage,read> tp_placements:array<u32>;
@group(0) @binding(50) var tp_hit:texture_2d<u32>;
@group(0) @binding(51) var tp_tiles:texture_2d<u32>;
@group(0) @binding(52) var tp_depth_out:texture_storage_2d<r32float,write>;
// primary_rays.wgsl PRIMARY_TILE.
const TP_TRACE_TILE=vec2<u32>(8u,4u);
@compute @workgroup_size(8,8) fn tp_depth_hits(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(tp_depth_out);
    if any(id.xy>=size) { return; }
    var depth=TP_MISS;
    let inside=all(id.xy>=tp.scene.xy) && all(id.xy<tp.scene.zw);
    if inside && textureLoad(tp_tiles,id.xy/TP_TRACE_TILE,0).x!=0u {
        let record=textureLoad(tp_hit,id.xy,0);
        if record.x!=0u {
            let placement=record.x-1u;
            let batch=tp_placements[placement]&0x1fffffffu;
            let i=record.y*3u;
            let a=tp_vertices[batch].data[tp_indices[batch].data[i]*3u].xyz;
            let b=tp_vertices[batch].data[tp_indices[batch].data[i+1u]*3u].xyz;
            let c=tp_vertices[batch].data[tp_indices[batch].data[i+2u]*3u].xyz;
            let w=bitcast<vec2<f32>>(record.zw);
            let local=a*(1.0-w.x-w.y)+b*w.x+c*w.y;
            let world=(tp_models[placement]*vec4<f32>(local,1.0)).xyz;
            depth=dot(world-tp.eye.xyz,tp.forward.xyz);
        }
    }
    textureStore(tp_depth_out,id.xy,vec4<f32>(depth,0.0,0.0,0.0));
}
// ---- end TP_DEPTH ----

// ---- TP_DEPTH_RASTER ----
@group(0) @binding(52) var tp_depth_out:texture_storage_2d<r32float,write>;
@group(0) @binding(53) var tp_raster_depth:texture_depth_2d;
@compute @workgroup_size(8,8) fn tp_depth_raster(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(tp_depth_out);
    if any(id.xy>=size) { return; }
    let z=textureLoad(tp_raster_depth,id.xy,0);
    var depth=TP_MISS;
    // Reversed Z: the cleared far plane is 0.
    if z>0.0 {
        let ndc=vec2<f32>((f32(id.x)+0.5)*tp.size.z*2.0-1.0,1.0-(f32(id.y)+0.5)*tp.size.w*2.0);
        let h=tp.inverse*vec4<f32>(ndc,z,1.0);
        depth=dot(h.xyz/h.w-tp.eye.xyz,tp.forward.xyz);
    }
    textureStore(tp_depth_out,id.xy,vec4<f32>(depth,0.0,0.0,0.0));
}
// ---- end TP_DEPTH_RASTER ----

// ---- TP_SHADOW ----
@group(1) @binding(0) var<storage,read> tp_models:array<mat4x4<f32>>;
@vertex fn tp_shadow_vs(@location(0) position:vec3<f32>, @builtin(instance_index) slot:u32) -> @builtin(position) vec4<f32> {
    return tp.shadow*(tp_models[slot]*vec4<f32>(position,1.0));
}
// ---- end TP_SHADOW ----

// ---- TP_GLASS ----
// Appended to a transparent material's composed raster program. Group 0 is this pass's
// own: the prelude camera (binding 0), the TLAS on RT renderers (binding 2), and below.
@group(0) @binding(31) var tp_color:texture_2d<f32>;
@group(0) @binding(32) var tp_depth:texture_2d<f32>;
@group(0) @binding(33) var tp_sampler:sampler;
@group(0) @binding(38) var tp_reflected:texture_2d<f32>;
struct TpDraw { thickness:f32, pad0:f32, pad1:f32, pad2:f32 };
@group(0) @binding(37) var<uniform> tp_draw:TpDraw;
// The opaque light behind a thin slab at `world`: entry refraction through the shading
// normal, exit through the flat back face, continued to the opaque depth behind. The
// shifted point is sampled from the colour behind this draw; samples of something nearer
// than the slab fall back to the unshifted pixel.
fn tp_behind(world:vec3<f32>, shading:vec3<f32>, geometric:vec3<f32>, pixel:vec2<i32>, depth:f32, ior:f32) -> vec3<f32> {
    let here=(vec2<f32>(pixel)+vec2<f32>(0.5))*tp.size.zw;
    let opaque=textureLoad(tp_depth,pixel,0).x;
    let d=normalize(world-tp.eye.xyz);
    var ng=geometric; if dot(ng,d)>0.0 { ng=-ng; }
    var ns=shading; if dot(ns,ng)<0.0 { ns=-ns; }
    var uv=here;
    if tp_draw.thickness>0.0 && ior>1.0 && opaque<TP_MISS {
        let t1=refract(d,ns,1.0/ior);
        let inside=dot(t1,-ng);
        if dot(t1,t1)>0.0 && inside>0.05 {
            let exit=world+t1*(tp_draw.thickness/inside);
            // Leaving through the back face: its normal faces the ray, as ng does.
            var t2=refract(t1,ng,ior);
            if dot(t2,t2)==0.0 { t2=d; }
            // Continue to the opaque surface's depth behind this pixel.
            let along=max(opaque-dot(exit-tp.eye.xyz,tp.forward.xyz),0.0)/max(dot(t2,tp.forward.xyz),0.05);
            let seen=exit+t2*along;
            let clip=tp.view_projection*vec4<f32>(seen,1.0);
            if clip.w>0.0 {
                let shifted=vec2<f32>(clip.x/clip.w*0.5+0.5,0.5-clip.y/clip.w*0.5);
                let p=vec2<i32>(clamp(shifted*tp.size.xy,vec2<f32>(0.0),tp.size.xy-vec2<f32>(1.0)));
                if all(shifted>=vec2<f32>(0.0)) && all(shifted<=vec2<f32>(1.0)) && textureLoad(tp_depth,p,0).x>depth {
                    uv=shifted;
                }
            }
        }
    }
    return textureSampleLevel(tp_color,tp_sampler,uv,0.0).rgb;
}
@fragment fn fs_transparent(input:RasterInput, triangle:RasterTriangle, @builtin(front_facing) front:bool) -> @location(0) vec4<f32> {
    let source_front=front==(input.winding>0.0);
    let geometric=unit_normal(cross(unit_normal(dpdx(input.world)),unit_normal(dpdy(input.world))));
    let face=-geometric;
    let source_geometric=select(-face,face,source_front);
    let pixel=vec2<i32>(input.position.xy);
    let surface_input=SurfaceInput(input.world,dpdx(input.world),dpdy(input.world),source_geometric,input.normal,dpdx(input.normal),dpdy(input.normal),input.tangent,input.handedness,
        input.uv,dpdx(input.uv),dpdy(input.uv),input.color,source_front,lighting.eye,vec2<u32>(pixel),material,
        raster_receiver(input.world,triangle,pixel,material.thin!=0u),decal_none());
    // DC1 decals project onto opaque receivers; transparent surfaces take none.
    let surface_output=surface(surface_input);
    // Engine inputs in uniform control flow: coverage alpha and the refraction normal.
    let sampled_alpha=sample_base_color(surface_input).a;
    let refraction_normal=material_normal(surface_input);
    if material.double_sided==0u && !source_front { discard; }
    let depth=dot(input.world-tp.eye.xyz,tp.forward.xyz);
    if depth>textureLoad(tp_depth,pixel,0).x { discard; }
    rl_indirect_prepare(surface_input);
    let lit=rl_finite(shade(surface_output,surface_input));
    let behind=tp_behind(input.world,refraction_normal,geometric,pixel,depth,transparent_ior(material));
    // AT1 fog and aerial perspective over the surface's own light, and the share of the
    // path's in-scattering that the light behind (already fogged up to the eye) carries:
    // atmosphere_surface(c, p) = c t + s, so fully clear glass leaves `behind` unchanged.
    let coverage=transparent_coverage(material,material.base_color.a*sampled_alpha*input.color.a);
    let fog=atmosphere_surface(vec3<f32>(0.0),input.world);
    let passed=TP_TRANSMIT;
    return vec4<f32>(rl_finite(atmosphere_surface(rl_finite(TP_SURFACE),input.world)+passed*max(behind-fog,vec3<f32>(0.0))),1.0);
}
// ---- end TP_GLASS ----

// ---- TP_AA_SEED ----
// TP2: four geometric coverage samples smooth glass edges without temporal trails.
// Shade once per pixel; untouched samples retain the exact colour behind the pane.
@group(0) @binding(31) var tp_seed_color:texture_2d<f32>;
@vertex fn tp_seed_vs(@builtin(vertex_index) id:u32)->@builtin(position) vec4<f32> {
    let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    return vec4<f32>(p[id],0.0,1.0);
}
@fragment fn tp_seed_fs(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32> {
    return textureLoad(tp_seed_color,vec2<i32>(p.xy),0);
}
// ---- end TP_AA_SEED ----

// ---- TP_PARTICLES ----
struct TpParticle { position:vec4<f32>, velocity:vec4<f32>, light:vec4<f32> };
// position.w age (s), velocity.w lifetime (s; 0 never alive), light.rgb incident irradiance.
struct TpEmitter {
    position:vec4<f32>,      // xyz, w spawn radius
    velocity:vec4<f32>,      // xyz, w spread
    acceleration:vec4<f32>,  // xyz, w drag
    color0:vec4<f32>,
    color1:vec4<f32>,
    emission0:vec4<f32>,     // rgb, w size at birth
    emission1:vec4<f32>,     // rgb, w size at death
    life:vec4<f32>,          // min, max lifetime; fade in, fade out
    counts:vec4<u32>,        // capacity, first spawn index (low 32 bits), spawn count, seed
    misc:vec4<f32>,          // x soft distance, y sorted size (power of two), z textured
};
@group(0) @binding(42) var<uniform> tp_emitter:TpEmitter;
@group(0) @binding(36) var tp_sprite:texture_2d<f32>;
@group(0) @binding(33) var tp_sampler:sampler;
@group(0) @binding(32) var tp_depth:texture_2d<f32>;
fn tp_hash(x:u32) -> u32 {
    var h=x*747796405u+2891336453u;
    h=((h>>((h>>28u)+4u))^h)*277803737u;
    return (h>>22u)^h;
}
fn tp_unit(seed:ptr<function,u32>) -> f32 {
    *seed=tp_hash(*seed);
    return f32(*seed>>8u)/16777216.0;
}
fn tp_ball(seed:ptr<function,u32>) -> vec3<f32> {
    let z=tp_unit(seed)*2.0-1.0;
    let phi=tp_unit(seed)*6.2831853;
    let r=pow(tp_unit(seed),1.0/3.0);
    return vec3<f32>(sqrt(max(1.0-z*z,0.0))*vec2<f32>(cos(phi),sin(phi)),z)*r;
}
// ---- end TP_PARTICLES ----

// ---- TP_PARTICLES_SIM ----
@group(0) @binding(40) var<storage,read_write> tp_particles:array<TpParticle>;
@group(0) @binding(41) var<storage,read_write> tp_sorted:array<u32>;
// The program API's `ambient`: prepared diffuse irradiance, or pi times the constant sky.
fn tp_ambient(n:vec3<f32>) -> vec3<f32> {
    if lighting.map.w!=0.0 {
        return environment_bilinear(environment_diffuse,environment_direction(n),0,0)*lighting.map.z;
    }
    return PI*lighting.environment;
}
// Directional lights' irradiance at a point: each light with the map's visibility of the
// frame's sun, plus the environment's mean irradiance over six axes (open sky).
fn tp_incident(world:vec3<f32>) -> vec3<f32> {
    var sum=vec3<f32>(0.0);
    for(var i=0u;i<lighting.count;i+=1u) {
        let light=lights[i];
        if light.kind.x!=0u { continue; }
        let l=-light.travel_outer.xyz;
        let visible=select(1.0,tp_sun_visibility(world),dot(l,tp.sun.xyz)>0.9999);
        sum+=light.rgb_inner.xyz*visible;
    }
    var sky=vec3<f32>(0.0);
    for(var a=0u;a<3u;a+=1u) {
        var n=vec3<f32>(0.0); n[a]=1.0;
        sky+=tp_ambient(n)+tp_ambient(-n);
    }
    return sum*0.5+sky/6.0;
}
@compute @workgroup_size(64) fn tp_particles_update(@builtin(global_invocation_id) id:vec3<u32>) {
    let capacity=tp_emitter.counts.x;
    let slot=id.x;
    if slot>=capacity { return; }
    let dt=tp.misc.x;
    var p=tp_particles[slot];
    let first=tp_emitter.counts.y;
    let spawned=tp_emitter.counts.z;
    // Spawn index k of this step lands in slot (first + k) % capacity.
    let k=(slot+capacity-first%capacity)%capacity;
    if k<spawned {
        var seed=tp_hash((first+k)^tp_hash(tp_emitter.counts.w));
        let life=mix(tp_emitter.life.x,tp_emitter.life.y,tp_unit(&seed));
        // Births spread evenly over the step, so a steady rate shows no banding.
        let age=dt*(f32(spawned-1u-k)+tp_unit(&seed))/f32(spawned);
        let velocity=tp_emitter.velocity.xyz+tp_ball(&seed)*tp_emitter.velocity.w;
        let position=tp_emitter.position.xyz+tp_ball(&seed)*tp_emitter.position.w+velocity*age;
        p=TpParticle(vec4<f32>(position,age),vec4<f32>(velocity,life),vec4<f32>(0.0));
    } else if p.position.w<p.velocity.w {
        let drag=exp(-tp_emitter.acceleration.w*dt);
        let velocity=p.velocity.xyz*drag+tp_emitter.acceleration.xyz*dt;
        p.position=vec4<f32>(p.position.xyz+velocity*dt,p.position.w+dt);
        p.velocity=vec4<f32>(velocity,p.velocity.w);
    }
    if p.position.w<p.velocity.w { p.light=vec4<f32>(tp_incident(p.position.xyz),0.0); }
    tp_particles[slot]=p;
}
// Bitonic sort of (depth, slot) keys in one workgroup: far to near, dead particles last.
// A key is the depth's top 21 float bits (positive floats order as integers) over an
// 11-bit slot; slots beyond the capacity and dead particles sort as depth 0.
const TP_SORT_THREADS=256u;
var<workgroup> tp_keys:array<u32,2048>;
@compute @workgroup_size(256) fn tp_particles_sort(@builtin(local_invocation_index) lane:u32) {
    let n=u32(tp_emitter.misc.y);
    let capacity=tp_emitter.counts.x;
    for(var i=lane;i<n;i+=TP_SORT_THREADS) {
        var key=0u;
        if i<capacity {
            let p=tp_particles[i];
            let depth=dot(p.position.xyz-tp.eye.xyz,tp.forward.xyz);
            if p.position.w<p.velocity.w && depth>0.0 { key=max(bitcast<u32>(depth)>>11u,1u)<<11u; }
        }
        tp_keys[i]=key|i;
    }
    workgroupBarrier();
    for(var k=2u;k<=n;k*=2u) {
        for(var j=k/2u;j>0u;j/=2u) {
            for(var i=lane;i<n/2u;i+=TP_SORT_THREADS) {
                let l=2u*j*(i/j)+i%j;
                let r=l+j;
                let a=tp_keys[l];let b=tp_keys[r];
                // Descending within ascending k-blocks: the whole array ends descending.
                let descending=(l&k)==0u;
                if (a<b)==descending { tp_keys[l]=b; tp_keys[r]=a; }
            }
            workgroupBarrier();
        }
    }
    for(var i=lane;i<n;i+=TP_SORT_THREADS) { tp_sorted[i]=tp_keys[i]; }
}
// ---- end TP_PARTICLES_SIM ----

// ---- TP_PARTICLES_DRAW ----
@group(0) @binding(40) var<storage,read> tp_particles:array<TpParticle>;
@group(0) @binding(41) var<storage,read> tp_sorted:array<u32>;
struct TpBillboard {
    @builtin(position) position:vec4<f32>,
    @location(0) uv:vec2<f32>,
    @location(1) @interpolate(flat) color:vec4<f32>,
    @location(2) @interpolate(flat) radiance:vec3<f32>,
    @location(3) depth:f32,
    @location(4) @interpolate(flat) emission:vec3<f32>,
    @location(5) world:vec3<f32>,
};
@vertex fn tp_particle_vs(@builtin(vertex_index) vertex:u32, @builtin(instance_index) rank:u32) -> TpBillboard {
    var out:TpBillboard;
    let key=tp_sorted[rank];
    let slot=key&0x7ffu;
    let p=tp_particles[slot];
    let corner=array<vec2<f32>,6>(vec2<f32>(-1.0,-1.0),vec2<f32>(1.0,-1.0),vec2<f32>(1.0,1.0),
        vec2<f32>(-1.0,-1.0),vec2<f32>(1.0,1.0),vec2<f32>(-1.0,1.0))[vertex];
    if (key>>11u)==0u || slot>=tp_emitter.counts.x {
        out.position=vec4<f32>(2.0,2.0,2.0,1.0);
        return out;
    }
    let t=clamp(p.position.w/max(p.velocity.w,1e-6),0.0,1.0);
    let size=mix(tp_emitter.emission0.w,tp_emitter.emission1.w,t);
    let world=p.position.xyz+(tp.right.xyz*corner.x+tp.up.xyz*corner.y)*(0.5*size);
    out.position=tp.view_projection*vec4<f32>(world,1.0);
    out.uv=corner*0.5+vec2<f32>(0.5);
    var color=mix(tp_emitter.color0,tp_emitter.color1,t);
    let fade_in=select(1.0,clamp(t/tp_emitter.life.z,0.0,1.0),tp_emitter.life.z>0.0);
    let fade_out=select(1.0,clamp((1.0-t)/tp_emitter.life.w,0.0,1.0),tp_emitter.life.w>0.0);
    color.a*=fade_in*fade_out;
    out.color=color;
    let emission=mix(tp_emitter.emission0.rgb,tp_emitter.emission1.rgb,t)*fade_in*fade_out;
    // Scattered light (albedo/pi of the incident irradiance) and emission, unexposed: the
    // vertex stage cannot read the lighting group.
    out.radiance=p.light.rgb*(color.rgb/PI);
    out.emission=emission;
    out.depth=dot(world-tp.eye.xyz,tp.forward.xyz);
    out.world=world;
    return out;
}
@fragment fn tp_particle_fs(input:TpBillboard) -> @location(0) vec4<f32> {
    let pixel=vec2<i32>(input.position.xy);
    let opaque=textureLoad(tp_depth,pixel,0).x;
    if input.depth>opaque { discard; }
    var shape=1.0;
    var tint=vec3<f32>(1.0);
    if tp_emitter.misc.z!=0.0 {
        let sprite=textureSampleLevel(tp_sprite,tp_sampler,input.uv,0.0);
        shape=sprite.a; tint=sprite.rgb;
    } else {
        let r=length(input.uv*2.0-vec2<f32>(1.0));
        shape=clamp(1.0-r,0.0,1.0); shape*=shape;
    }
    // Soft particles: fade toward the opaque surface behind.
    let soft=select(1.0,clamp((opaque-input.depth)/tp_emitter.misc.x,0.0,1.0),tp_emitter.misc.x>0.0);
    let alpha=clamp(input.color.a*shape*soft,0.0,1.0);
    // Premultiplied: scattering covers `alpha`; emission adds over the particle's shape.
    let rgb=exposed_product(input.radiance,tint*alpha)+exposed_product(input.emission,tint*(shape*soft));
    // AT1 fog over premultiplied light: rgb t + alpha s, as atmosphere_surface(rgb) less the
    // in-scattering the uncovered share leaves to what is behind.
    let fog=atmosphere_surface(vec3<f32>(0.0),input.world);
    let fogged=atmosphere_surface(min(rgb,vec3<f32>(65504.0)),input.world)-(1.0-alpha)*fog;
    return vec4<f32>(clamp(fogged,vec3<f32>(0.0),vec3<f32>(65504.0)),alpha);
}
// ---- end TP_PARTICLES_DRAW ----
