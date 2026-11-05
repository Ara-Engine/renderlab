//! R1 test-only receiver policies. No production shader, ABI or default changes.
use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) enum Policy {
    Minimal,
    Magnitude,
    Viewed,
}

pub(super) fn sources(policy: Policy) -> FilmSources {
    let mut sources = film_sources(true);
    if matches!(policy, Policy::Minimal) {
        return sources;
    }
    // Receiver reconstruction belongs to tracing; the program consumes the
    // unchanged Receiver interface and the visibility texture for every policy.
    sources
        .kernels
        .push_str(include_str!("../support/receiver_policy.wgsl"));
    sources.kernels = edit(
        &sources.kernels,
        (
            "ray.near+direction*hit.t,getCommittedHitVertexPositions(&query)",
            "world_position(getCommittedHitVertexPositions(&query)[0]*(1.0-hit.barycentrics.x-hit.barycentrics.y)+getCommittedHitVertexPositions(&query)[1]*hit.barycentrics.x+getCommittedHitVertexPositions(&query)[2]*hit.barycentrics.y,hit.instance_custom_data).xyz,getCommittedHitVertexPositions(&query)",
        ),
    );
    sources.kernels = edit(
        &sources.kernels,
        (
            "textureStore(primary_world_output,p,vec4<f32>(trace.point,clip.z/clip.w));",
            "let a=world_position(trace.positions[0],trace.record.x-1u).xyz;\n    let b=world_position(trace.positions[1],trace.record.x-1u).xyz;\n    let c=world_position(trace.positions[2],trace.record.x-1u).xyz;\n    textureStore(primary_world_output,p,vec4<f32>(trace.point,receiver_epsilon(a,b,c)));",
        ),
    );
    sources.kernels = edit(
        &sources.kernels,
        (
            "let origin=world+normal*1e-5;",
            "let epsilon=textureLoad(fresh_world,vec2<i32>(pixel),0).w;\n    let origin=world+normal*epsilon;",
        ),
    );
    sources.kernels = edit(&sources.kernels, ("let tmin=1e-5;", "let tmin=epsilon;"));
    if matches!(policy, Policy::Viewed) {
        sources.kernels = edit(
            &sources.kernels,
            (
                "let normal=select(-geometric,geometric,dot(geometric,l)>=0.0);",
                "let normal=select(-geometric,geometric,dot(geometric,lighting.eye-world)>=0.0);",
            ),
        );
    }
    sources
}

pub(super) fn direct_source() -> String {
    // Preserve F0/R1's environment reset in the standard program itself. A separate
    // sun-only shade body changes rounding on the quarry, even with the same BRDF.
    let program = edit(
        STANDARD,
        (
            "    for(var i=0u;i<light_count();i+=1u) {",
            "    exposed=vec3<f32>(0.0);\n    for(var i=0u;i<light_count();i+=1u) {",
        ),
    );
    let source = film_shading(&program);
    assert_eq!(source.matches("primary_frame.clear").count(), 3);
    source.replace("primary_frame.clear", "vec4<f32>(0.0,0.0,0.0,1.0)")
}

pub(super) fn accumulator_source(r1: bool) -> String {
    let source = include_str!("../support/film_accumulate.wgsl");
    if !r1 {
        // Uncovered trace tiles leave stale hit records. F0 predates that engine
        // contract: reject those samples as R1 already does, preserving its estimand.
        let source = edit(
            source,
            (
                "    if h!=0u && h==bitcast<u32>(c.w) && abs(ds-dc)<=0.02*dc {",
                "    if textureLoad(tiles,id.xy/vec2<u32>(8u,4u),0).x!=0u && h!=0u && h==bitcast<u32>(c.w) && abs(ds-dc)<=0.02*dc {",
            ),
        );
        return format!("{source}\n@group(0) @binding(8) var tiles:texture_2d<u32>;\n");
    }
    let source = edit(
        source,
        (
            "    if h!=0u && h==bitcast<u32>(c.w) && abs(ds-dc)<=0.02*dc {",
            "    saved[i]=0xffffu;\n    if textureLoad(tiles,id.xy/vec2<u32>(8u,4u),0).x!=0u && h!=0u && h==bitcast<u32>(c.w) && abs(ds-dc)<=0.02*dc {",
        ),
    );
    let source = edit(
        &source,
        (
            "        let v=textureLoad(visibility,p,0).x;",
            "        let v=textureLoad(visibility,p,0).x;\n        saved[i]=v;",
        ),
    );
    format!(
        "{source}\n@group(0) @binding(7) var<storage,read_write> saved:array<u32>;\n@group(0) @binding(8) var tiles:texture_2d<u32>;\n"
    )
}

#[test]
fn receiver_policy_sources_compile() {
    for policy in [Policy::Minimal, Policy::Magnitude, Policy::Viewed] {
        let sources = sources(policy);
        for source in [sources.kernels, sources.shading] {
            validate_wgsl(&source);
        }
    }
    validate_wgsl(&direct_source());
    validate_wgsl(&accumulator_source(true));
}

#[test]
fn receiver_precision_scale() {
    let epsilon = |m: f32| 1e-5f32.max(16.0 * (f32::from_bits(m.to_bits() + 1) - m));
    assert_eq!(epsilon(4.0), 1e-5);
    assert_eq!(epsilon(1000.0), 0.0009765625);
    assert_eq!(epsilon(10000.0), 0.015625);
}

pub(super) fn analytic_fixture(r: &mut Renderer, m: &serde_json::Value, policy: Policy) {
    let workload = m["workload"].as_str().unwrap();
    let distance = m["distance"].as_f64().unwrap_or(0.0) as f32;
    let translation = Vec3::splat(distance);
    let mut profile = vec![];
    if workload == "sawtooth" {
        let mut x = -0.1008f32;
        profile.push([x, 0.]);
        for dx in [0.0018, 0.0036, 0.0072] {
            for _ in 0..8 {
                x += dx;
                profile.push([x, dx * 0.45]);
                x += dx;
                profile.push([x, 0.]);
            }
        }
    } else {
        profile = vec![
            [-2., if workload == "convex" { -0.004 } else { 0. }],
            [0., 0.],
            [2., if workload == "convex" { -0.004 } else { 0. }],
        ];
    }
    let mut vertices = vec![];
    let mut indices = vec![];
    for pair in profile.windows(2) {
        let [[x0, z0], [x1, z1]] = [pair[0], pair[1]];
        let normal = Vec3::new(z0 - z1, 0., x1 - x0).normalize().to_array();
        let base = vertices.len() as u32;
        for position in [[x0, -8., z0], [x1, -8., z1], [x1, 8., z1], [x0, 8., z0]] {
            vertices.push(Vertex {
                position,
                normal,
                uv: [0.; 2],
                color: [1.; 4],
            });
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let mesh = r
        .try_upload_mesh(&MeshData {
            vertices: &vertices,
            indices: &indices,
        })
        .unwrap()
        .unwrap();
    r.test_drain("R1 fixture mesh");
    let mut instances = r
        .try_upload_instances(&[Mat4::from_translation(translation)])
        .unwrap()
        .unwrap();
    r.test_drain("R1 fixture instances");
    let incidence = m["incidence"].as_f64().unwrap_or(85.) as f32;
    let theta = incidence.to_radians();
    r.set_lighting_and_shadows(
        &Lighting {
            lights: vec![Light::Directional {
                travel: Vec3::new(-theta.sin(), 0., -theta.cos()),
                lux: [1000.; 3],
            }],
            exposure_stops: -8.,
            ..Default::default()
        },
        &Shadows {
            directional_disk_angle: if workload == "sawtooth" { 0.04 } else { 0.001 },
            requests: vec![ShadowRequest::Directional {
                split_ends: vec![10., 40.],
            }],
            ..Default::default()
        },
    )
    .unwrap();
    let eye = if workload == "sawtooth" { 0.15 } else { 4. };
    let camera = Camera::look_at(
        translation + Vec3::Z * eye,
        translation,
        Vec3::Y,
        55f32.to_radians(),
        0.01,
        100.,
    )
    .unwrap();
    let color = target(r, 256, 192);
    render(r, &color, &camera, &mut [Draw::new(&mesh, &mut instances)]);
    let out = PathBuf::from(m["output"].as_str().unwrap());
    integrate_with(r, &camera, &out, 0, true, Some(policy));
    fs::write(out.join("geometry.json"),serde_json::to_vec_pretty(&serde_json::json!({"profile":profile,"translation":translation.to_array(),"incidence":incidence,"disk_angle":if workload=="sawtooth" {0.04} else {0.001},"workload":workload})).unwrap()).unwrap();
}
