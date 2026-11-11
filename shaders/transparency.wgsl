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
    misc:vec4<f32>,          // x soft distance, y sorted size (power of two), z textured,
                             // w span this step's spawns spread over (s; 0: the step)
    shape:vec4<f32>,         // x 0 sphere, 1 box (yzw half extents), 2 cone (yzw axis)
    velocity_box:vec4<f32>,  // xyz per-axis velocity half range
    curl:vec4<f32>,          // x frequency, y strength, z this step's burst births,
                             // w phase per second of birth time (0: per spawn ordinal)
    vortex_center:vec4<f32>,
    vortex_axis:vec4<f32>,   // xyz axis scaled by strength
    attractor:vec4<f32>,     // xyz position, w strength
    plane:vec4<f32>,         // xyz unit normal (zero: no collision), w offset
    curves:vec4<f32>,        // x restitution, y attractor softening, z colour keys, w size keys
    color_t:vec4<f32>,
    color_keys:array<vec4<f32>,4>,
    emission_keys:array<vec4<f32>,4>,
    size_t:vec4<f32>,
    size_keys:vec4<f32>,
    align:vec4<f32>,         // x 0 camera, 1 velocity (y stretch), 2 plane (yzw unit normal)
    flipbook:vec4<f32>,      // x columns, y rows, z frames per second, w 1 at that rate,
                             // 2 once over each life (0: no flipbook)
    sprite:vec4<f32>,        // x height over width, y 1 to turn each sprite randomly,
                             // z the emitter's clock at the end of this step (s), w a
                             // stepped prewarm's substep (s; 0: the frame's step)
};
// A curve's interpolation weights at life fraction t: the key below (index) and the share
// of the next. Keys hold their end values outside their range.
fn tp_curve(times:vec4<f32>, count:u32, t:f32) -> vec2<f32> {
    if t<=times[0] { return vec2<f32>(0.0,0.0); }
    for(var i=1u;i<count;i+=1u) {
        if t<times[i] { return vec2<f32>(f32(i-1u),(t-times[i-1u])/max(times[i]-times[i-1u],1e-6)); }
    }
    return vec2<f32>(f32(count-1u),0.0);
}
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
// Offset of a new particle from the emitter position, by spawn shape.
fn tp_spawn_offset(seed:ptr<function,u32>) -> vec3<f32> {
    let shape=tp_emitter.shape;
    if shape.x==1.0 {
        return (vec3<f32>(tp_unit(seed),tp_unit(seed),tp_unit(seed))*2.0-vec3<f32>(1.0))*shape.yzw;
    }
    if shape.x==2.0 {
        let h=pow(tp_unit(seed),1.0/3.0);
        let r=sqrt(tp_unit(seed))*h*tp_emitter.position.w;
        let a=tp_unit(seed)*6.2831853;
        // An orthonormal frame about the axis (Duff et al.'s branchless basis).
        let length=max(length(shape.yzw),1e-12);
        let n=shape.yzw/length;
        let sign=select(-1.0,1.0,n.z>=0.0);
        let b=-1.0/(sign+n.z);
        let c=n.x*n.y*b;
        let u=vec3<f32>(1.0+sign*n.x*n.x*b,sign*c,-sign*n.x);
        let v=vec3<f32>(c,sign+n.y*n.y*b,-n.y);
        return (u*cos(a)+v*sin(a))*r+shape.yzw*h;
    }
    return tp_ball(seed)*tp_emitter.position.w;
}
// The curl field's origin (Forces::curl_origin), in the w lanes of velocity_box,
// vortex_center and vortex_axis.
fn tp_curl_origin() -> vec3<f32> {
    return vec3<f32>(tp_emitter.velocity_box.w,tp_emitter.vortex_center.w,tp_emitter.vortex_axis.w);
}
// Accelerations beyond gravity: curl noise, vortex and attractor (Ara's formulas).
fn tp_forces(x:vec3<f32>, phase:f32) -> vec3<f32> {
    var a=vec3<f32>(0.0);
    if tp_emitter.curl.y!=0.0 {
        let q=(x-tp_curl_origin())*tp_emitter.curl.x+vec3<f32>(phase);
        a+=tp_emitter.curl.y*vec3<f32>(cos(q.y)-cos(q.z),cos(q.z)-cos(q.x),cos(q.x)-cos(q.y));
    }
    a+=cross(tp_emitter.vortex_axis.xyz,x-tp_emitter.vortex_center.xyz);
    if tp_emitter.attractor.w!=0.0 {
        let d=tp_emitter.attractor.xyz-x;
        a+=d*(tp_emitter.attractor.w/pow(dot(d,d)+tp_emitter.curves.y,1.5));
    }
    return a;
}
// Plane collision: push out of the plane and reflect inward velocity by the restitution.
fn tp_collide(p:ptr<function,TpParticle>) {
    let n=tp_emitter.plane.xyz;
    if dot(n,n)==0.0 || (*p).position.w>=(*p).velocity.w { return; }
    let distance=dot((*p).position.xyz,n)-tp_emitter.plane.w;
    if distance>=0.0 { return; }
    (*p).position=vec4<f32>((*p).position.xyz-distance*n,(*p).position.w);
    let vn=dot((*p).velocity.xyz,n);
    if vn<0.0 {
        (*p).velocity=vec4<f32>((*p).velocity.xyz-(1.0+tp_emitter.curves.x)*vn*n,(*p).velocity.w);
    }
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
        // TP2: particles receive the same coloured sun transmission as surfaces.
        let transmission=select(vec3<f32>(1.0),transparent_sun_transmission(world,l),light.kind.z!=0u);
        sum+=light.rgb_inner.xyz*visible*transmission;
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
    let dt=select(tp.misc.x,tp_emitter.sprite.w,tp_emitter.sprite.w>0.0);
    var p=tp_particles[slot];
    let first=tp_emitter.counts.y;
    let spawned=tp_emitter.counts.z;
    // Spawn index k of this step lands in slot (first + k) % capacity.
    let k=(slot+capacity-first%capacity)%capacity;
    if k<spawned {
        var seed=tp_hash((first+k)^tp_hash(tp_emitter.counts.w));
        let life=mix(tp_emitter.life.x,tp_emitter.life.y,tp_unit(&seed));
        // Births spread evenly over the step, so a steady rate shows no banding.
        // The last `burst` spawns are this step's burst, born within the step; the others
        // spread over the prewarm window (or the step), oldest first.
        let burst=u32(tp_emitter.curl.z);
        let rate=spawned-min(burst,spawned);
        var age=dt*tp_unit(&seed);
        if k<rate {
            let window=select(dt,tp_emitter.misc.w,tp_emitter.misc.w>0.0);
            age=window*(f32(rate-1u-k)+tp_unit(&seed))/f32(rate);
        }
        let box=vec3<f32>(tp_unit(&seed),tp_unit(&seed),tp_unit(&seed))*2.0-vec3<f32>(1.0);
        let v0=tp_emitter.velocity.xyz+tp_ball(&seed)*tp_emitter.velocity.w+box*tp_emitter.velocity_box.xyz;
        let start=tp_emitter.position.xyz+tp_spawn_offset(&seed);
        // Closed-form motion under drag d and acceleration a over the age (exact for any
        // age, so a prewarmed population stands where a running one would).
        let d=tp_emitter.acceleration.w;
        let a=tp_emitter.acceleration.xyz;
        let e=exp(-d*age);
        let f=select((1.0-e)/max(d,1e-6),age,d<1e-4);
        let g=select((age-f)/max(d,1e-6),0.5*age*age,d<1e-4);
        let velocity=v0*e+a*f;
        let position=start+v0*f+a*g;
        // light.w: the particle's curl-noise phase: its birth time (the emitter's clock less
        // its age) times the phase rate, or by default its spawn ordinal.
        var phase=f32((first+k)%100000u)*0.013;
        if tp_emitter.curl.w!=0.0 { phase=(tp_emitter.sprite.z-age)*tp_emitter.curl.w; }
        p=TpParticle(vec4<f32>(position,age),vec4<f32>(velocity,life),vec4<f32>(0.0,0.0,0.0,phase));
    } else if p.position.w<p.velocity.w {
        let drag=exp(-tp_emitter.acceleration.w*dt);
        var velocity=p.velocity.xyz*drag+tp_emitter.acceleration.xyz*dt;
        velocity+=tp_forces(p.position.xyz,p.light.w)*dt;
        p.position=vec4<f32>(p.position.xyz+velocity*dt,p.position.w+dt);
        p.velocity=vec4<f32>(velocity,p.velocity.w);
    }
    tp_collide(&p);
    if p.position.w<p.velocity.w { p.light=vec4<f32>(tp_incident(p.position.xyz),p.light.w); }
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
    // The flipbook's two neighbouring frames (uv) and the second's weight.
    @location(6) uv_next:vec2<f32>,
    @location(7) @interpolate(flat) blend:f32,
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
    var size=mix(tp_emitter.emission0.w,tp_emitter.emission1.w,t);
    let size_keys=u32(tp_emitter.curves.w);
    if size_keys>0u {
        let w=tp_curve(tp_emitter.size_t,size_keys,t);
        let i=u32(w.x);
        size=mix(tp_emitter.size_keys[i],tp_emitter.size_keys[min(i+1u,size_keys-1u)],w.y);
    }
    // The sprite's horizontal and vertical axes, each scaled to half its extent.
    var across=tp.right.xyz*(0.5*size);
    var along=tp.up.xyz*(0.5*size);
    if tp_emitter.align.x==1.0 {
        let v=p.velocity.xyz-dot(p.velocity.xyz,tp.forward.xyz)*tp.forward.xyz;
        let length2=dot(v,v);
        if length2>1e-12 {
            let dir=v*inverseSqrt(length2);
            along=dir*(0.5*size*tp_emitter.align.y);
            across=normalize(cross(tp.forward.xyz,dir))*(0.5*size);
        }
    } else if tp_emitter.align.x==2.0 {
        let n=tp_emitter.align.yzw;
        let sign=select(-1.0,1.0,n.z>=0.0);
        let b=-1.0/(sign+n.z);
        let c=n.x*n.y*b;
        across=vec3<f32>(1.0+sign*n.x*n.x*b,sign*c,-sign*n.x)*(0.5*size);
        along=vec3<f32>(c,sign+n.y*n.y*b,-n.y)*(0.5*size);
    }
    along*=tp_emitter.sprite.x;
    if tp_emitter.sprite.y!=0.0 {
        let angle=f32(tp_hash(slot*2654435761u+7u)>>8)/16777216.0*6.2831853;
        let a=across;
        across=a*cos(angle)+along*sin(angle);
        along=along*cos(angle)-a*sin(angle);
    }
    let world=p.position.xyz+across*corner.x+along*corner.y;
    out.position=tp.view_projection*vec4<f32>(world,1.0);
    let local=vec2<f32>(corner.x*0.5+0.5,0.5-corner.y*0.5);
    out.uv=local;
    out.uv_next=local;
    out.blend=0.0;
    if tp_emitter.flipbook.w!=0.0 {
        let grid=tp_emitter.flipbook.xy;
        let frames=grid.x*grid.y;
        var at=p.position.w*tp_emitter.flipbook.z;
        var first=floor(at)%frames;
        var next=(first+1.0)%frames;
        if tp_emitter.flipbook.w==2.0 {
            at=t*(frames-1.0);
            first=min(floor(at),frames-1.0);
            next=min(first+1.0,frames-1.0);
        }
        let cell=vec2<f32>(1.0)/grid;
        out.uv=(vec2<f32>(first%grid.x,floor(first/grid.x))+local)*cell;
        out.uv_next=(vec2<f32>(next%grid.x,floor(next/grid.x))+local)*cell;
        out.blend=fract(at);
    }
    var color=mix(tp_emitter.color0,tp_emitter.color1,t);
    var emitted=mix(tp_emitter.emission0.rgb,tp_emitter.emission1.rgb,t);
    let color_keys=u32(tp_emitter.curves.z);
    if color_keys>0u {
        let w=tp_curve(tp_emitter.color_t,color_keys,t);
        let i=u32(w.x);
        let j=min(i+1u,color_keys-1u);
        color=mix(tp_emitter.color_keys[i],tp_emitter.color_keys[j],w.y);
        emitted=mix(tp_emitter.emission_keys[i].rgb,tp_emitter.emission_keys[j].rgb,w.y);
    }
    let fade_in=select(1.0,clamp(t/tp_emitter.life.z,0.0,1.0),tp_emitter.life.z>0.0);
    let fade_out=select(1.0,clamp((1.0-t)/tp_emitter.life.w,0.0,1.0),tp_emitter.life.w>0.0);
    color.a*=fade_in*fade_out;
    out.color=color;
    let emission=emitted*fade_in*fade_out;
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
        let sprite=mix(textureSampleLevel(tp_sprite,tp_sampler,input.uv,0.0),
            textureSampleLevel(tp_sprite,tp_sampler,input.uv_next,0.0),input.blend);
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
