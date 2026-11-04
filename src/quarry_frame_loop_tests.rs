//! End-to-end offscreen benchmark of the public renderer, without shader substitutions.
//! The Python runner owns preflight/lease enforcement and immutable input identities.
use super::*;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Serialize)]
struct Row {
    index: usize,
    frame: u64,
    pose: usize,
    sun: usize,
    /// Start of this iteration through submission: includes update and admission backpressure.
    cpu_frame_ms: f64,
    /// Timed update/draw preparation/renderer calls, excluding yields between busy attempts.
    cpu_calls_ms: f64,
    update_ms: f64,
    draw_preparation_ms: f64,
    submit_ms: f64,
    render_poll_ms: f64,
    preparation_ms: f64,
    shadow_preparation_ms: f64,
    upload_ms: f64,
    encode_ms: f64,
    busy_attempts: usize,
    frame_to_frame_ms: f64,
    collection_ms: f64,
    gpu_ms: Option<f64>,
    receiver_ms: Option<f64>,
    shading_ms: Option<f64>,
    tone_ms: Option<f64>,
    completion_latency_ms: Option<f64>,
    camera_bytes: u64,
    light_bytes: u64,
    geometry_bytes: u64,
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

/// No per-frame device wait: the library's bounded submission slots provide backpressure.
/// Completion timestamps are host observations, so throughput uses the entire drained block.
#[test]
#[ignore = "GPU lease, 8 GiB free RAM and RENDERLAB_PRIMARY_MANIFEST; no windows"]
#[allow(clippy::assertions_on_constants)] // Keep debug test builds legal, but reject running this benchmark in one.
fn gpu_quarry_frame_loop() {
    assert!(!cfg!(debug_assertions), "use an optimized --release binary");
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(std::env::var("RENDERLAB_PRIMARY_MANIFEST").unwrap()).unwrap(),
    )
    .unwrap();
    let validate = manifest["validate"].as_bool().unwrap_or(false);
    let _validation = validation::begin(validate);
    let startup = Instant::now();
    let mut r = renderer(validate);
    let device_setup_ms = ms(startup.elapsed());
    let load = Instant::now();
    let (mut asset, cameras) = load_quarry(&manifest, &mut r);
    let load_upload_ms = ms(load.elapsed());
    let color = target(&r, 1920, 1080);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    r.set_soft_shadows(crate::SoftShadows::Interactive);
    let poses = [0, 1].map(|i| cameras.frames[i].camera().unwrap());
    let lighting = lighting_settings::parse(
        &fs::read_to_string(manifest["lighting"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    let mut alternate_sun = lighting.clone();
    let Light::Directional { travel, .. } = &mut alternate_sun.lights[0] else {
        panic!("expected the registered directional sun");
    };
    // UE yaw +1 degree maps to source-basis rotation about Y by -1 degree.
    *travel = Mat4::from_rotation_y(-1f32.to_radians()).transform_vector3(*travel);
    let sequence = manifest["sequence"].as_str().unwrap();
    assert!(["static", "camera", "sun"].contains(&sequence));
    let static_pose = manifest["pose"].as_u64().unwrap_or(0) as usize;
    assert!(static_pose < 2);
    let warmup = manifest["warmup"].as_u64().unwrap_or(512) as usize;
    let count = manifest["frames"].as_u64().unwrap_or(2048) as usize;
    assert!(warmup > 0 && count > 1);
    let mut rows: Vec<Row> = Vec::with_capacity(count);
    let mut indices = BTreeMap::new();
    // The shared startup loader collects upload completions without delivering
    // them. Establish a fresh observation boundary before the timed frame loop.
    let startup_dropped = r.poll().unwrap().dropped;
    let collect = |r: &mut Renderer, rows: &mut [Row], indices: &BTreeMap<u64, usize>| {
        let events = r.poll().unwrap();
        assert_eq!(events.dropped, startup_dropped);
        for g in events.gpu {
            if let Some(&i) = indices.get(&g.frame.0) {
                rows[i].gpu_ms = Some(ms(g.render_span));
                rows[i].receiver_ms = g.receiver_pass.map(ms);
                rows[i].shading_ms = Some(ms(g.mesh_pass));
                rows[i].tone_ms = g.present_pass.map(ms);
            }
        }
        for c in events.completed {
            if let Some(&i) = c.frame.as_ref().and_then(|f| indices.get(&f.0)) {
                rows[i].completion_latency_ms = Some(ms(c.observed_at - c.submitted_at));
            }
        }
    };
    let mut block_start = Instant::now();
    let mut previous = block_start;
    let mut first_frame_ms = None;
    let mut last_scene = None;
    for i in 0..warmup + count {
        if i == warmup {
            r.test_drain("frame-loop warmup boundary only");
            collect(&mut r, &mut rows, &indices);
            block_start = Instant::now();
            previous = block_start;
        }
        let begin = Instant::now();
        let interval = ms(begin - previous);
        previous = begin;
        let pose = if sequence == "camera" {
            i % 2
        } else {
            static_pose
        };
        let sun = usize::from(sequence == "sun") * (i % 2);
        let update = Instant::now();
        if sequence == "sun" {
            r.set_lighting(if sun == 0 { &lighting } else { &alternate_sun })
                .unwrap();
        }
        let update_ms = ms(update.elapsed());
        let prep = Instant::now();
        let mut draws = asset.draws(Visibility::Frustum);
        let draw_preparation_ms = ms(prep.elapsed());
        let mut calls_ms = update_ms + draw_preparation_ms;
        let mut busy_attempts = 0;
        let report = loop {
            let call = Instant::now();
            let report = r
                .try_render_view(&color, &poses[pose], &mut draws, ViewMode::Lit)
                .unwrap();
            calls_ms += ms(call.elapsed());
            if let Some(report) = report {
                break report;
            }
            busy_attempts += 1;
            assert!(begin.elapsed() < Duration::from_secs(10));
            std::thread::yield_now();
        };
        let cpu_frame_ms = ms(begin.elapsed());
        assert_eq!(report.primary, PrimaryVisibility::Traced);
        assert_eq!(report.soft_shadows, crate::SoftShadows::Interactive);
        assert!(report.timed);
        assert_eq!(report.scene.draws, 12);
        assert_eq!(report.scene.triangles, 13_034_903);
        last_scene = Some(report.scene);
        if i == 0 {
            r.test_drain("first completed image outside measured block");
            first_frame_ms = Some(ms(startup.elapsed()));
        }
        if i >= warmup {
            let cpu = report.cpu;
            indices.insert(report.frame.0, rows.len());
            rows.push(Row {
                index: i - warmup,
                frame: report.frame.0,
                pose,
                sun,
                cpu_frame_ms,
                cpu_calls_ms: calls_ms,
                update_ms,
                draw_preparation_ms,
                submit_ms: ms(cpu.submit),
                render_poll_ms: ms(cpu.poll),
                preparation_ms: ms(cpu.preparation + cpu.visibility + cpu.packing),
                shadow_preparation_ms: ms(cpu.shadow_preparation),
                upload_ms: ms(cpu.upload + cpu.shadow_upload),
                encode_ms: ms(cpu.encode + cpu.shadow_encode),
                busy_attempts,
                frame_to_frame_ms: interval,
                collection_ms: 0.,
                gpu_ms: None,
                receiver_ms: None,
                shading_ms: None,
                tone_ms: None,
                completion_latency_ms: None,
                camera_bytes: report.uploads.camera,
                light_bytes: report.uploads.lights,
                geometry_bytes: report.uploads.geometry,
            });
        }
        let collection = Instant::now();
        collect(&mut r, &mut rows, &indices);
        if i >= warmup {
            rows.last_mut().unwrap().collection_ms = ms(collection.elapsed());
        }
    }
    let submission_block_ms = ms(block_start.elapsed());
    r.test_drain("frame-loop final boundary only");
    let completed_block_ms = ms(block_start.elapsed());
    collect(&mut r, &mut rows, &indices);
    assert!(
        rows.iter()
            .all(|row| row.gpu_ms.is_some() && row.completion_latency_ms.is_some())
    );
    assert!(rows.iter().all(|row| row.geometry_bytes == 0));
    // Readback is after the measured completion boundary. Verify the complete HDR
    // image against the accepted Interactive checkpoint, not a region average.
    let hdr = capture(&mut r);
    let frozen = manifest["accepted_interactive"].as_str().map(|directory| {
        PathBuf::from(directory).join(format!(
            "pose-{}-seed-0-interactive.rgba16f",
            rows.last().unwrap().pose
        ))
    });
    let hdr_matches_accepted = if sequence != "sun" {
        frozen.as_ref().map(|path| {
            assert_eq!(hdr, fs::read(path).unwrap(), "complete HDR changed");
            true
        })
    } else {
        None
    };
    use sha2::{Digest, Sha256};
    let hdr_sha256 = format!("{:x}", Sha256::digest(&hdr));
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    fs::write(
        out.join("frames.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "sequence":sequence,"pose":static_pose,"warmup":warmup,"frames":count,
            "validation":validate,"debug_build":cfg!(debug_assertions),
            "startup_dropped_observations":startup_dropped,"measured_dropped_observations":0,
            "width":1920,"height":1080,"samples":1,"anisotropy":16,
            "device_setup_ms":device_setup_ms,"load_upload_ms":load_upload_ms,
            "startup_to_first_completed_image_ms":first_frame_ms,
            "submission_block_ms":submission_block_ms,"completed_block_ms":completed_block_ms,
            "completed_ms_per_frame":completed_block_ms/count as f64,
            "completed_fps":count as f64*1000./completed_block_ms,
            "hdr_matches_accepted":hdr_matches_accepted,"hdr_sha256":hdr_sha256,
            "scene":format!("{:?}",last_scene.unwrap()),"stats":format!("{:?}",r.stats()),
            "scope":"public Renderer::try_render_view, actual Interactive traced shading/tone, updates, bounded in-flight admission and timestamp collection; offscreen; no acquire/present/scanout; first-image and resident preparation outside steady-state",
            "rows":rows
        }))
        .unwrap(),
    )
    .unwrap();
}
