// ---- renderlab engine inputs: occluded environment specular (M7-ES1) ----
// Composed after gi_api.wgsl into modules whose program calls one of these inputs
// (src/program.rs), so other programs compile exactly as before. docs/m7-es1.md.
//
// The selected RLE2 map's lobe table (src/environment_lobes.rs owns its layout): row 0
// holds lobe k's source-space axis and sharpness (texel k), amplitude and solid angle
// (texel 4 + k) and the lobe count (texel 8); row 1 + i holds knot i's share of the
// prefiltered field, lobe k in channel k, over angle bins uniform in sqrt(angle / pi).
@group(3) @binding(10) var environment_lobe_table:texture_2d<f32>;
const ES_BINS:f32=128.0;

// The sky visibility a receiver's specular sees: the world radiance cache's occluded
// irradiance over the open environment's, per channel, about the receiver's vertex normal
// on its viewed side. A concentrated sky makes it the visibility of its dominant lobe; an
// even one, cosine-weighted sky visibility. 1 where the cache has no value or is off.
// The generated entry computes it once per receiver, before `shade`, through
// rl_indirect_prepare (src/program.rs): every input below shares that one cache lookup.
var<private> es_visibility:vec3<f32>;
fn es_prepare(input:SurfaceInput) {
    es_visibility=vec3<f32>(1.0);
    if gi_word(GI_H_ENABLED)==0u { return; }
    let n=unit_normal(gi_bin_source(gi_viewed_normal(input.receiver.world,input.receiver.geometric),input.normal));
    let open=gi_open_irradiance(n);
    let ratio=clamp(indirect_irradiance(input,n)/max(open,vec3<f32>(1e-30)),vec3<f32>(0.0),vec3<f32>(1.0));
    es_visibility=select(vec3<f32>(1.0),ratio,open>vec3<f32>(1e-20));
}

// Occluded incident radiance about `direction`, averaged over the N=V GGX lobe of
// perceptual `roughness`: the prepared field's prefiltered radiance (or the constant
// environment) times this receiver's sky visibility. Without the world radiance cache it
// is exactly the open value. With a map bound it is in environment_reflection's units
// (before intensity and exposure: combine with environment_product); otherwise in
// environment_constant's (combine with exposed_product). No per-pixel ray.
fn indirect_radiance(input:SurfaceInput, direction:vec3<f32>, roughness:f32) -> vec3<f32> {
    var open=lighting.environment;
    if lighting.map.w!=0.0 { open=environment_reflection(direction,roughness); }
    if gi_word(GI_H_ENABLED)==0u { return open; }
    return open*es_visibility;
}

// A dominant source of the environment (a sun and its aureole) that the engine separates
// from the prefiltered field, for programs to evaluate against their own BRDF.
struct EnvironmentLobe {
    // Unit world direction of the lobe's axis.
    direction:vec3<f32>,
    // Its radiance falls as exp(sharpness*(cos-1)) away from the axis.
    sharpness:f32,
    // Its radiance integrated over the sphere (the illuminance it delivers at normal
    // incidence), occluded at this receiver, before intensity and exposure: combine with
    // environment_product.
    illuminance:vec3<f32>,
};
// The lobes the engine separates: zero without a world radiance cache, which alone can
// occlude them; they then stay inside indirect_radiance's prefiltered value.
fn environment_lobe_count() -> u32 {
    if gi_word(GI_H_ENABLED)==0u || lighting.map.w>=0.0 { return 0u; }
    return u32(textureLoad(environment_lobe_table,vec2<i32>(8,0),0).x);
}
// The inverse of environment_direction: source space to world.
fn es_world_direction(source:vec3<f32>) -> vec3<f32> {
    return vec3<f32>(lighting.map.x*source.x+lighting.map.y*source.z,source.y,lighting.map.x*source.z-lighting.map.y*source.x);
}
fn environment_lobe(input:SurfaceInput, i:u32) -> EnvironmentLobe {
    let axis=textureLoad(environment_lobe_table,vec2<i32>(i32(i),0),0);
    let power=textureLoad(environment_lobe_table,vec2<i32>(4+i32(i),0),0);
    return EnvironmentLobe(es_world_direction(axis.xyz),axis.w,power.rgb*power.w*es_visibility);
}
// Lobe i's part of indirect_radiance(input, direction, roughness), in its units: a program
// that evaluates the lobe itself subtracts this from that value. Knots bracket and blend
// in r^4 as environment_radiance's do, so the two stay consistent.
fn environment_lobe_reflection(input:SurfaceInput, i:u32, direction:vec3<f32>, roughness:f32) -> vec3<f32> {
    let axis=textureLoad(environment_lobe_table,vec2<i32>(i32(i),0),0);
    let power=textureLoad(environment_lobe_table,vec2<i32>(4+i32(i),0),0);
    let c=clamp(dot(unit_normal(environment_direction(direction)),axis.xyz),-1.0,1.0);
    let u=sqrt(acos(c)/PI)*(ES_BINS-1.0);
    let j=min(i32(floor(u)),i32(ES_BINS)-2);
    let f=clamp(u-f32(j),0.0,1.0);
    let r=clamp(roughness,0.0,1.0);
    var lo=64;
    var hi=64;
    var t=0.0;
    if r<1.0 {
        lo=min(i32(floor(64.0*sqrt(r))),63);
        hi=lo+1;
        let a=f32(lo)/64.0;
        let b=f32(lo+1)/64.0;
        let a2=a*a; let b2=b*b;
        let a4=a2*a2; let b4=b2*b2;
        let a8=a4*a4; let b8=b4*b4;
        let r2=r*r;
        t=clamp((r2*r2-a8)/(b8-a8),0.0,1.0);
    }
    let p0=mix(textureLoad(environment_lobe_table,vec2<i32>(j,1+lo),0)[i],
        textureLoad(environment_lobe_table,vec2<i32>(j+1,1+lo),0)[i],f);
    let p1=mix(textureLoad(environment_lobe_table,vec2<i32>(j,1+hi),0)[i],
        textureLoad(environment_lobe_table,vec2<i32>(j+1,1+hi),0)[i],f);
    return power.rgb*mix(p0,p1,t)*es_visibility;
}
// ---- end of the occluded environment specular inputs ----
