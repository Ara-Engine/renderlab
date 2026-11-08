//! AT2's existing quarry loader runs one still/moving-sun batch per device.
use super::*;
use serde_json::json;

#[allow(clippy::too_many_arguments)]
pub(super) fn quarry(
    r: &mut Renderer,
    asset: &mut quarry_prepared::Prepared,
    camera: &Camera,
    size: [u32; 2],
    out: &Path,
    path: &str,
    azimuth: Vec3,
    sun: impl Fn(&mut Renderer, Vec3, [f32; 3]),
    render: impl Fn(&mut Renderer, &mut quarry_prepared::Prepared, &Camera),
) {
    r.set_atmosphere(Atmosphere::default()).unwrap();
    r.set_environment(None, Default::default()).unwrap();
    r.set_indirect(IndirectSettings::default()).unwrap();
    // New RT tables are already zero-initialized; explicitly start the diagnostic age.
    r.reset_indirect();
    r.set_reconstruction(Reconstruction::default()).unwrap();
    let base = sun_travel(azimuth, 25f32.to_radians());
    sun(r, base, [120_000.0; 3]);
    for _ in 0..96 {
        render(r, asset, camera);
    }
    r.test_drain("AT2 warm");
    png(&out.join(format!("{path}-still.png")), size, &capture(r));
    let identity = r.test_atmosphere_environment();
    let initial_age = r.test_indirect_age();
    assert!(identity.2 != 0);
    if path != "raster" {
        assert!(initial_age >= 96);
        assert!(initial_age < u32::MAX && r.indirect_stats().frames >= 96);
    }
    let mut images = Vec::new();
    for frame in 0..32 {
        // 0.05 degrees/frame is an accelerated but continuous day cycle.
        let travel = sun_travel(azimuth, (25.0 + (frame + 1) as f32 * 0.05).to_radians());
        sun(r, travel, [120_000.0; 3]);
        // Reproduce Engine::set_sun's automatic-sky publication.
        r.set_environment(None, Default::default()).unwrap();
        render(r, asset, camera);
        let state = r.test_atmosphere_environment();
        assert_eq!(
            (state.0, state.2),
            (identity.0, identity.2),
            "sun replaced storage"
        );
        assert_eq!(state.1, identity.1 + u64::from(frame + 1));
        if path != "raster" {
            assert_eq!(
                r.test_indirect_age(),
                initial_age + frame + 1,
                "sun cleared GI"
            );
        }
        if [0, 7, 15, 31].contains(&frame) {
            let image = capture(r);
            assert!(image.iter().flatten().all(|v| v.is_finite()));
            let mean = image
                .iter()
                .flat_map(|p| p.iter())
                .map(|v| f64::from(*v))
                .sum::<f64>()
                / (image.len() * 3) as f64;
            assert!(mean > 1e-5, "sun motion blacked out");
            png(
                &out.join(format!("{path}-sun-{frame:02}.png")),
                size,
                &image,
            );
            images.push(
                json!({"frame":frame,"mean_exposed_rgb":mean,"gi_age":r.test_indirect_age()}),
            );
        }
    }
    // A continuous angle after the moving sequence, and then its stationary counterpart.
    // Keep GI active in both cases: the same 96-frame settle precedes each timed segment.
    let mut timings = Vec::new();
    for moving in [false, true] {
        for _ in 0..96 {
            render(r, asset, camera);
        }
        r.test_drain("AT2 timing warm");
        let _ = r.poll().unwrap();
        let mut samples = Vec::new();
        let begin = Instant::now();
        let frames = 32;
        let mut cpu = Vec::new();
        for frame in 0..frames {
            let begin = Instant::now();
            let angle = 26.6
                + if moving {
                    (frame + 1) as f32 * 0.05
                } else {
                    0.0
                };
            sun(r, sun_travel(azimuth, angle.to_radians()), [120_000.0; 3]);
            r.set_environment(None, Default::default()).unwrap();
            render(r, asset, camera);
            cpu.push(begin.elapsed().as_secs_f64() * 1e3);
            samples.extend(r.poll().unwrap().gpu);
        }
        r.test_drain("AT2 timing");
        samples.extend(r.poll().unwrap().gpu);
        timings.push(json!({"moving":moving,"frames":frames,"wall_ms_per_frame":begin.elapsed().as_secs_f64()*1e3/f64::from(frames),
            "cpu_submit_ms":cpu,"gpu_span_ms":samples.iter().map(|s|s.render_span.as_secs_f64()*1e3).collect::<Vec<_>>() }));
    }
    // A high sky view isolates the real-radius disk on both miss implementations.
    // Disable temporal reconstruction so the analytic edge itself is inspected.
    r.set_indirect(IndirectSettings::off()).unwrap();
    r.set_reconstruction(Reconstruction::off()).unwrap();
    let sky = Atmosphere {
        aerial_perspective: false,
        height_fog: None,
        volumetric_fog: None,
        ..Default::default()
    };
    r.set_atmosphere(sky).unwrap();
    let eye = camera.view().inverse().w_axis.truncate();
    let mut disk = Vec::new();
    for frame in 0..8 {
        let direction = -sun_travel(azimuth, (45.0 + frame as f32 * 0.01).to_radians());
        r.set_lighting(&Lighting {
            lights: vec![Light::Directional {
                travel: -direction,
                lux: [120_000.0; 3],
            }],
            environment: [0.0; 3],
            exposure_stops: -28.0,
        })
        .unwrap();
        // Keep the camera fixed across sun subpixel shifts.
        let fixed = -sun_travel(azimuth, 45f32.to_radians());
        let view = Camera::look_at(
            eye,
            eye + fixed * 100.0,
            Vec3::Y,
            camera.fov_y(),
            0.05,
            500.0,
        )
        .unwrap();
        render(r, asset, &view);
        let image = capture(r);
        let mut crop = Vec::new();
        for y in size[1] / 2 - 24..size[1] / 2 + 24 {
            for x in size[0] / 2 - 24..size[0] / 2 + 24 {
                crop.push(image[(y * size[0] + x) as usize]);
            }
        }
        let energy = crop
            .iter()
            .map(|p| f64::from(p[0] + p[1] + p[2]))
            .sum::<f64>();
        disk.push(json!({"frame":frame,"energy_exposed_rgb":energy,"crop_size":[48,48],"crop_hdr_rgb":crop}));
        png(
            &out.join(format!("{path}-disk-{frame:02}.png")),
            size,
            &image,
        );
    }
    let log = json!({"path":path,"size":size,"sun_degrees_per_frame":0.05,
        "initial_map_generation":identity.0,"initial_content_revision":identity.1,
        "final_map_generation":r.test_atmosphere_environment().0,"gi_still_age":initial_age,
        "moving_images":images,"timings":timings,"disk":disk,"debug_build":cfg!(debug_assertions)});
    std::fs::write(
        out.join(format!("{path}-at2.json")),
        serde_json::to_vec_pretty(&log).unwrap(),
    )
    .unwrap();
}
