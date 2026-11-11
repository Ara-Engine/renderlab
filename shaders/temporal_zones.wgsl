// ---- TR1 restart zones (P-RELIT, docs/p-relit.md) ----
// Where this frame's resolve restarts static receivers' histories, decided from engine data
// and the receiver's world position alone, so the sun-shadow passes before shading can ask
// the same question of the same record. ZONES is the frame record's Tr1Zones member; each
// module that includes this text names its own (temporal_reconstruction.rs `zones_source`).

// Whether the moving caster's swept bounds can change point x's light this frame (SK1). With
// a shadowed sun (shadow_hi.w 1): x lies inside the bounds, or some ray toward the sun within
// its disk meets them. Conservative: a disk ray of length t stays within t*tan(half-angle) of
// the central ray, and no point of the bounds is farther than `reach`, so the central ray meets
// the bounds grown by reach*tan(half-angle) whenever a disk ray meets the bounds.
fn tr1_caster_shadow(x:vec3<f32>)->bool {
    if ZONES.shadow_lo.w==0.0 {return false;}
    let lo=ZONES.shadow_lo.xyz;let hi=ZONES.shadow_hi.xyz;
    if ZONES.shadow_hi.w==0.0 {return all(x>=lo) && all(x<=hi);}
    let reach=length(max(abs(x-lo),abs(x-hi)));
    let grow=vec3<f32>(reach*ZONES.shadow_light.w);
    let rcp=vec3<f32>(1.0)/ZONES.shadow_light.xyz;
    let a=(lo-grow-x)*rcp;let b=(hi+grow-x)*rcp;
    let t0=min(a,b);let t1=max(a,b);
    return min(min(t1.x,t1.y),t1.z)>=max(max(max(t0.x,t0.y),t0.z),0.0);
}

// Whether point x lies in the restart sphere of a local light whose intensity or colour
// changed this frame: where the light's swing since the histories began could leave more
// stale light than Reconstruction::light_budget, so the pixel follows the light exactly
// instead. The spheres come from the lights' positions, ranges and candela history and the
// exposure (relit_zones): no pixel colour or lighting difference.
fn tr1_relit(x:vec3<f32>)->bool {
    for(var k=0u;k<ZONES.counts.x;k+=1u) {
        let sphere=ZONES.relit[k];
        let d=x-sphere.xyz;
        if dot(d,d)<=sphere.w {return true;}
    }
    return false;
}

// Sun-shadow directions a receiver at x evaluates this frame: the frame's `rays`, or the
// restart count (counts.y; 0 when it is no larger) where the resolve will restart a static
// receiver there, so a pixel that starts again from one frame gets the rays a frame that
// restarts every pixel would give it and every other pixel keeps the frame's count. A moving
// receiver in a caster's swept shadow gets the restart count too; its history continues.
fn tr1_zone_rays(x:vec3<f32>,rays:u32)->u32 {
    let restart=ZONES.counts.y;
    if restart<=rays {return rays;}
    return select(rays,restart,tr1_caster_shadow(x) || tr1_relit(x));
}
