// ---- renderlab world radiance cache: rays aimed around local lights (U-SPECK) ----
// Composed after gi_cache.wgsl (src/gi_cache.rs kernel_source), whose gathers call it; it reads
// the lights, the header, a ray source's gi_closest and the kernels' gi_hit_radiance and
// gi_gather_sh. A file of its own so that a test can run it against a stub scene
// (src/gi_cache_tests.rs gpu_near_light_rays_keep_the_gathered_light).
// A surface a local light reaches from a distance r reflects a radiance that grows as 1 / r^2,
// while what it sends a cell falls with its solid angle. A fixed direction stands for
// 2 pi / K sr whatever it hits, so the few cells whose direction met a small surface close to
// a lamp (its bulb, a fitting) took that radiance at many times its true solid angle, and
// every other cell took none of it: bright blobs a cell wide on every surface in sight of the
// lamp, at rest and in motion.
// The surfaces near a light are therefore gathered by rays aimed at them. For a cell at
// distance d from a local light in its upper hemisphere, the light's near region is the ball
// of radius GI_NEAR_REACH * d about it (the nearer a lit surface is to the light than the cell
// is, the less a fixed direction resolves it):
// - the fixed directions add nothing for hits inside the ball (gi_near_owner);
// - GI_H_NEAR_RAYS rays leave the cell around the light's direction, their angle psi from it
//   log-uniform from the light's source to the ball's edge, a density of 1 / psi^2 per solid
//   angle, which cancels the lit surfaces' 1 / r^2; their azimuths are a golden-angle sequence
//   turned by the cell's own angle, and one more ray goes at the light itself, for the cap
//   inside the smallest angle. Each adds its hit's radiance over (rays * density), only for
//   hits inside the ball.
// The two sets of rays split the hemisphere's hits by where the hit lies, so their sum
// estimates the same integral: no light is added or dropped, and the lamp's surroundings are
// sampled where they are bright. A hit inside two lights' balls belongs to the lower-indexed
// light. Beyond a light's range its near surfaces send a cell next to nothing: there the
// fixed directions only leave out a ball of at most a sixth of the range, and no ray is aimed.
// The rays are fixed per cell, as the directions are: no sample counts, no ranking (W1, W2).
const GI_NEAR_REACH:f32=0.3333333;
// asin(GI_NEAR_REACH): the ball's angular radius from the cell.
const GI_NEAR_ANGLE:f32=0.3398369;
// Local light l as a cell at `origin` with normal n sees it: the unit direction to it and
// its distance, or w 0 when the light has no near region for the cell (a directional light, a
// dark one, one at or below the cell's horizon).
fn gi_near_light(l:u32, origin:vec3<f32>, n:vec3<f32>) -> vec4<f32> {
    let light=lights[l];
    if light.kind.x==0u || all(light.rgb_inner.xyz==vec3<f32>(0.0)) { return vec4<f32>(0.0); }
    let delta=light.position_range.xyz-origin;
    let distance=length(delta);
    if distance<=1e-6 || dot(delta,n)<=0.0 { return vec4<f32>(0.0); }
    return vec4<f32>(delta/distance,distance);
}
fn gi_near_in_range(l:u32, distance:f32) -> bool {
    let range=lights[l].position_range.w;
    return range<=0.0 || distance<range;
}
// The light whose near region holds the point x for that cell (the lowest index), or
// 0xffffffff.
fn gi_near_owner(origin:vec3<f32>, n:vec3<f32>, x:vec3<f32>) -> u32 {
    if gi_word(GI_H_NEAR_RAYS)==0u { return 0xffffffffu; }
    for(var l=0u;l<lighting.count;l+=1u) {
        let near=gi_near_light(l,origin,n);
        if near.w<=0.0 { continue; }
        var radius=GI_NEAR_REACH*near.w;
        if !gi_near_in_range(l,near.w) { radius=min(radius,lights[l].position_range.w/6.0); }
        let v=x-lights[l].position_range.xyz;
        if dot(v,v)<radius*radius { return l; }
    }
    return 0xffffffffu;
}
// One lane's share of a cell's aimed rays (ray i of the cell runs on lane i % GI_GATHER_LANES),
// added to gi_gather_sh. Returns the rays traced, those whose hit cell had no value yet and
// those whose hit value still changes, as the fixed directions count them.
fn gi_near_gather(lane:u32, origin:vec3<f32>, n:vec3<f32>, key:vec2<u32>, band:u32, tertiary:bool) -> vec3<u32> {
    let aimed=gi_word(GI_H_NEAR_RAYS);
    var rays=0u;
    var missing=0u;
    var changing=0u;
    if aimed==0u { return vec3<u32>(0u); }
    let per=aimed+1u;
    let turn=select(0.0,f32(key.y>>8u)*(6.283185307179586/16777216.0),(gi_word(GI_H_FLAGS)&GI_F_ROTATE)!=0u);
    // The index of the light's first ray among the cell's aimed rays.
    var first=0u;
    for(var l=0u;l<lighting.count;l+=1u) {
        let near=gi_near_light(l,origin,n);
        if near.w<=0.0 || !gi_near_in_range(l,near.w) { continue; }
        let angle_min=min(max(0.25*lights[l].source.x,0.01)/near.w,0.5*GI_NEAR_ANGLE);
        let span=log(GI_NEAR_ANGLE/angle_min);
        let frame=gi_basis(near.xyz);
        let reach=GI_NEAR_REACH*near.w;
        for(var i=first+(lane+GI_GATHER_LANES-first%GI_GATHER_LANES)%GI_GATHER_LANES;i<first+per;i+=GI_GATHER_LANES) {
            let s=i-first;
            // The last ray is the cap's, at the light itself.
            var psi=0.0;
            var weight=6.283185307179586*(1.0-cos(angle_min));
            if s<aimed {
                psi=angle_min*exp(span*(f32(s)+0.5)/f32(aimed));
                weight=6.283185307179586*sin(psi)*psi*span/f32(aimed);
            }
            // Each light's sequence starts at its own azimuth.
            let phi=f32(s)*2.399963229728653+turn+f32(l)*0.754877666;
            let d=frame*vec3<f32>(sin(psi)*cos(phi),sin(psi)*sin(phi),cos(psi));
            if dot(d,n)<=0.0 { continue; }
            // The ball ends at the light's distance plus its radius.
            let h=gi_closest(origin,d,0.0,near.w+reach);
            rays+=1u;
            if !h.hit || gi_near_owner(origin,n,origin+d*h.t)!=l { continue; }
            let o=gi_hit_radiance(origin,d,h,band,tertiary);
            if o.w==GI_HIT_MISSING || o.w==0.0 { missing+=1u; }
            if o.w==GI_HIT_CHANGING { changing+=1u; }
            if o.w<=GI_HIT_NONE { continue; }
            gi_sh_basis(d);
            let radiance=o.rgb*weight;
            for(var j=0u;j<4u;j+=1u) { gi_gather_sh[j]+=radiance*gi_y[j]; }
        }
        first+=per;
    }
    return vec3<u32>(rays,missing,changing);
}
