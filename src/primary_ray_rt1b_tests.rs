//! RT1b's experimental footprint estimators. Production/reference modules stay
//! untouched until a candidate passes the complete static and motion gate.
use super::*;
use sha2::{Digest, Sha256};

fn sources(mode: u32, pattern: u32, rays: u32) -> [String; 2] {
    assert!([8, 256].contains(&rays));
    let mut sources = primary_ray_gpu::shader_sources(SOURCE).unwrap();
    let original = sources[1].clone();
    let soft = &mut sources[1];
    *soft = edit(
        soft,
        (
            "        var ray_origin=origin;var ray_projected=projected;",
            "        var ray_origin=origin;var ray_projected=projected;var ray_epsilon=receiver.epsilon;",
        ),
    );
    *soft = edit(
        soft,
        (
            "j<FRESH_DIRECTIONS;j+=select(FRESH_LANES,1u,FRESH_DIRECTIONS==8u)",
            "j<RT1B_COUNT;j+=1u",
        ),
    );
    *soft = edit(
        soft,
        (
            "let direction=fresh_direction(j,shift,l,u,v,view.texel.z);",
            "let direction=rt1b_direction(j,shift,l,u,v,view.texel.z);",
        ),
    );
    *soft = edit(
        soft,
        (
            "        if receiver.thin {\n            ray_origin=shadow_origin(receiver,direction);",
            "        if RT1B_MODE!=0u {\n            let sample_receiver=rt1b_sample(receiver,j,pixel,width);\n            if !rt1b_sample_valid {continue;}\n            accepted+=1u;\n            if RT1B_MODE==4u {ray_epsilon=sample_receiver.epsilon;}\n            ray_origin=shadow_origin(sample_receiver,select(l,direction,sample_receiver.thin));\n            ray_projected=(view.world_to_clip*vec4<f32>(ray_origin,1.0)).xyz;\n        } else if receiver.thin {\n            ray_origin=shadow_origin(receiver,direction);",
        ),
    );
    *soft = edit(
        soft,
        ("let tmin=receiver.epsilon;", "let tmin=ray_epsilon;"),
    );
    assert_eq!(soft.matches("visible=fresh_estimate(receiver,").count(), 3);
    *soft = soft.replace(
        "visible=fresh_estimate(receiver,",
        "visible=rt1b_estimate(trace,receiver,",
    );
    *soft = soft.replace(
        "(256u/FRESH_DIRECTIONS)*visible",
        "select((256u/RT1B_COUNT)*visible,visible,RT1B_MODE==4u)",
    );
    *soft = edit(
        soft,
        ("    var visible=0u;", "    var visible=0u;var accepted=0u;"),
    );
    *soft = edit(
        soft,
        (
            "    return visible;",
            "    if RT1B_MODE==4u {\n        if accepted==0u {return (256u/FRESH_DIRECTIONS)*fresh_estimate(receiver,pixel,width,lane);}\n        return (256u*visible+accepted/2u)/accepted;\n    }\n    return visible;",
        ),
    );
    let helper = include_str!("../support/rt1b_footprint.wgsl")
        .replace(
            "override RT1B_MODE:u32=0u;",
            &format!("const RT1B_MODE:u32={mode}u;"),
        )
        .replace(
            "override RT1B_STRATIFIED:bool=false;",
            &format!("const RT1B_STRATIFIED:bool={};", pattern == 1),
        )
        .replace(
            "override RT1B_SUNFLOWER:bool=false;",
            &format!("const RT1B_SUNFLOWER:bool={};", pattern == 2),
        )
        .replace("FRESH_DIRECTIONS", "RT1B_COUNT");
    // Keep the ordinary fresh_compute entry on its original ABI. Only the fused
    // experimental entries may read geometry while constructing footprints.
    let range = |source: &str| {
        let start = source.find("fn fresh_estimate(").unwrap();
        let end = start
            + source[start..]
                .find("// Receiver pixels per workgroup")
                .unwrap();
        start..end
    };
    let modified = range(soft);
    let estimator =
        soft[modified.clone()].replacen("fn fresh_estimate(", "fn rt1b_fresh_estimate(", 1);
    soft.replace_range(modified, &original[range(&original)]);
    let helper = helper.replace(
        "return fresh_estimate(receiver,pixel,width,lane);",
        "return rt1b_fresh_estimate(receiver,pixel,width,lane);",
    );
    soft.push_str(&format!(
        "\nconst RT1B_COUNT:u32={rays}u;\n{estimator}\n{helper}"
    ));
    sources
}

#[test]
fn rt1b_footprint_sources_validate() {
    for (mode, pattern, rays) in [
        (1, 0, 8),
        (2, 0, 8),
        (3, 0, 8),
        (3, 1, 8),
        (3, 2, 8),
        (3, 0, 256),
        (4, 0, 8),
        (4, 1, 8),
        (4, 2, 8),
        (4, 0, 256),
    ] {
        let (module, info) = validate_wgsl(&sources(mode, pattern, rays)[1]);
        for entry in &module.entry_points {
            let constants = [("FRESH_DIRECTIONS".to_owned(), 8.0)].into_iter().collect();
            naga::back::pipeline_constants::process_overrides(
                &module,
                &info,
                Some((entry.stage, &entry.name)),
                &constants,
            )
            .unwrap_or_else(|error| {
                panic!(
                    "mode={mode}, pattern={pattern}, rays={rays}, entry={}: {error:?}",
                    entry.name
                )
            });
        }
    }
}

fn swap(r: &mut Renderer, candidate: &mut Option<primary_ray_gpu::PrimaryPipelines>) {
    if let Some(candidate) = candidate {
        std::mem::swap(r.pipeline.primary.as_mut().unwrap(), candidate);
    }
}

/// Current GPU receiver attachments at the three old-origin witnesses. Normal.w
/// carries the explicit material thin sign; world.w carries the actual epsilon.
fn receiver_policy(r: &Renderer, camera: &Camera) -> serde_json::Value {
    let t = r.view.fresh.as_ref().unwrap().targets.as_ref().unwrap();
    let world = read_texture(r, &t.world.texture);
    let normal = read_texture(r, &t.normal.texture);
    let eye = camera.view().inverse().w_axis.truncate();
    let rows: Vec<_> = [(1002usize,491usize),(910,635),(1212,653)]
        .map(|(x,y)| {
            let decode = |data: &[u8]| -> [f32;4] {std::array::from_fn(|c| {
                let at=(x+y*1920)*16+c*4;
                f32::from_le_bytes(data[at..at+4].try_into().unwrap())
            })};
            let w=decode(&world);let n=decode(&normal);
            assert_eq!(w[3],1e-5);assert!(n[3]>0.0);
            let mut geometric=Vec3::from_array(n[..3].try_into().unwrap());
            let point=Vec3::from_array(w[..3].try_into().unwrap());
            if geometric.dot(eye-point)<0.0 {geometric = -geometric;}
            let origin=std::array::from_fn::<_,3,_>(|i| geometric[i].mul_add(w[3],point[i]));
            serde_json::json!({"pixel":[x,y],"world":w,"geometric":n,"eye":eye.to_array(),
                "viewed_normal":geometric.to_array(),"origin_replayed_fma":origin,"thin":n[3]<0.0,"t_min":w[3]})
        }).into_iter().collect();
    serde_json::json!(rows)
}

#[test]
#[ignore = "RT1b GPU lease, no game, 4/2 GiB guard; one quarry upload"]
fn gpu_rt1b_footprint_batch() {
    let m = manifest();
    let _validation = validation::begin(true);
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let mut r = renderer(true);
    let (mut asset, cameras) = load_quarry(&m, &mut r);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let color = target(&r, 1920, 1080);
    let mut variants = vec![("baseline", None)];
    let camera_control = m["camera_control"].as_bool().unwrap_or(false);
    let variants_to_compile = if camera_control {
        vec![
            ("camera", 4, 0, 8),
            ("camera-stratified", 4, 1, 8),
            ("camera-sunflower", 4, 2, 8),
            ("camera-exact256", 4, 0, 256),
        ]
    } else {
        vec![
            ("clipped", 1, 0, 8),
            ("plane", 2, 0, 8),
            ("surface", 3, 0, 8),
            ("surface-stratified", 3, 1, 8),
            ("surface-sunflower", 3, 2, 8),
            ("surface-exact256", 3, 0, 256),
        ]
    };
    for (name, mode, pattern, rays) in variants_to_compile {
        let p = r.view.primary.as_ref().unwrap().compile(
            &r.device,
            &r.ledger,
            &sources(mode, pattern, rays),
        );
        variants.push((name, Some(p)));
    }
    let mut rows = vec![];
    let ms = |d: Duration| d.as_secs_f64() * 1000.;
    for pose in 0..2 {
        let camera = cameras.frames[pose].camera().unwrap();
        // The real reference and policy probe are pinned to the unchanged Exact.
        r.set_soft_shadows(crate::SoftShadows::Exact);
        traced_frame(&mut r, &color, &camera, &mut asset, true);
        let [reference_hits, _] = interactive_outputs(&r);
        if pose == 0 {
            fs::write(
                out.join("receiver-policy.json"),
                serde_json::to_vec_pretty(&receiver_policy(&r, &camera)).unwrap(),
            )
            .unwrap();
        }
        fs::write(
            out.join(format!("pose-{pose}-baseline.hit.u32")),
            &reference_hits,
        )
        .unwrap();
        r.set_soft_shadows(crate::SoftShadows::Interactive);
        for (name, candidate) in &mut variants {
            swap(&mut r, candidate);
            let mut hashes = vec![];
            for repeat in 0..8 {
                traced_frame(&mut r, &color, &camera, &mut asset, true);
                let [hits, counts] = interactive_outputs(&r);
                let hdr = capture(&mut r);
                assert_eq!(hits, reference_hits, "primary identity changed: {name}");
                let hash = [&hits, &counts, &hdr].map(|b| format!("{:x}", Sha256::digest(b)));
                if repeat == 0 {
                    fs::write(out.join(format!("pose-{pose}-{name}.u32")), &counts).unwrap();
                    fs::write(out.join(format!("pose-{pose}-{name}.rgba16f")), &hdr).unwrap();
                } else {
                    assert_eq!(hashes[0], hash, "repeat {repeat}, {name}");
                }
                hashes.push(hash);
            }
            rows.push(serde_json::json!({"pose":pose,"name":name,"hashes_hits_counts_hdr":hashes}));
            swap(&mut r, candidate);
        }
        // A short paired screen, no readbacks inside the timing block. This is
        // development-profile diagnostic timing, not a release cost certificate.
        r.test_drain("RT1b before paired screen");
        r.poll().unwrap();
        let mut pending = std::collections::HashMap::new();
        let mut timings = vec![];
        for pair in 0..10 {
            let n = variants.len();
            for k in 0..n {
                let i = if pair % 2 == 0 { k } else { n - 1 - k };
                let (name, candidate) = &mut variants[i];
                swap(&mut r, candidate);
                let report = traced_frame(&mut r, &color, &camera, &mut asset, true);
                if pair >= 2 {
                    pending.insert(report.frame.0, (pair - 2, *name));
                }
                swap(&mut r, candidate);
                for g in r.poll().unwrap().gpu {
                    if let Some((pair, name)) = pending.remove(&g.frame.0) {
                        timings.push(serde_json::json!({"pair":pair,"name":name,"receiver_ms":g.receiver_pass.map(ms),"span_ms":ms(g.render_span)}));
                    }
                }
            }
        }
        r.test_drain("RT1b paired screen");
        for g in r.poll().unwrap().gpu {
            if let Some((pair, name)) = pending.remove(&g.frame.0) {
                timings.push(serde_json::json!({"pair":pair,"name":name,"receiver_ms":g.receiver_pass.map(ms),"span_ms":ms(g.render_span)}));
            }
        }
        assert!(pending.is_empty());
        fs::write(
            out.join(format!("pose-{pose}-timing.json")),
            serde_json::to_vec_pretty(&timings).unwrap(),
        )
        .unwrap();
        fs::write(
            out.join("batch.json"),
            serde_json::to_vec_pretty(&rows).unwrap(),
        )
        .unwrap();
    }
    r.test_drain("RT1b complete");
}
