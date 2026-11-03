use super::*;
use crate::asset::virtual_mesh::{PAGE_BYTES, PageId, decode_page};
use crate::virtual_geometry::{
    tests::{directory, fixture, retire_for_inflight_test, write_pages},
    *,
};
use sha2::{Digest, Sha256};
#[test]
#[ignore = "Vulkan GPU; exact admission accounting and conservative completion against scan policy"]
fn gpu_virtual_admission_accounting() {
    let _validation = validation::begin(true);
    let (device, queue) = device(true);
    let mut r = Renderer::new(device, queue, FORMAT, Limits::default(), SOURCE).unwrap();
    crate::virtual_geometry::tests::admission_accounting(&mut r);
}

fn virtual_frame(
    r: &mut Renderer,
    color: &wgpu::Texture,
    camera: &Camera,
    a: &mut VirtualAsset,
    s: &mut VirtualScene,
) -> FrameReport {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(report) = r
            .try_render_virtual(color, camera, a, s, ViewMode::BaseColor)
            .unwrap()
        {
            return report;
        }
        assert!(Instant::now() < deadline, "virtual startup/render deadline");
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
#[ignore = "Vulkan GPU; actual page-local ranges, materials, transforms, depth, 1x/4x and 300 settled frames"]
fn gpu_virtual_ranges_reference_and_settled() {
    let _validation = validation::begin(true);
    for samples in [SampleCount::One, SampleCount::Four] {
        let (device, queue) = device(true);
        let mut r = Renderer::new_with_options(
            device,
            queue,
            FORMAT,
            Limits::default(),
            RendererOptions { samples },
            SOURCE,
        )
        .unwrap();
        let color = target(r.device(), 513, 321);
        let cam = camera(0.);
        let (package, blobs) = fixture(3);
        let dir = directory("gpu-ranges");
        write_pages(&package, &blobs, &dir);
        let models = [
            Mat4::from_translation(Vec3::new(-1.2, 0., 0.)) * Mat4::from_scale(Vec3::splat(0.7)),
            Mat4::from_translation(Vec3::new(1.2, 0., -0.4))
                * Mat4::from_scale(Vec3::new(-0.7, 0.5, 1.2)),
        ];
        let mut a = VirtualAsset::new(
            package.clone(),
            dir,
            StreamOptions {
                geometry_bytes: 2 * PAGE_BYTES,
                max_candidates: 0,
                max_clusters: 6,
                ..Default::default()
            },
        )
        .unwrap();
        let mut scene = VirtualScene::new(
            &a,
            &models.map(|transform| VirtualPlacement { mesh: 0, transform }),
        )
        .unwrap();
        virtual_frame(&mut r, &color, &cam, &mut a, &mut scene);
        let (actual, depth) = if samples == SampleCount::One {
            let (c, d) = r.test_capture_full(&color);
            (c, Some(d))
        } else {
            (r.test_capture_color(&color), None)
        };
        // Independent raw-M3 oracle: decode actual disk bytes and upload each selected span.
        // Production holds one vertex/index allocation for BOTH cluster ranges of this page.
        let root = 2;
        let geom = decode_page(
            &package.pages[root],
            &blobs[root],
            Sha256::digest(&blobs[root]).into(),
        )
        .unwrap();
        let meshes: Vec<_> = (0..2)
            .map(|i| upload(&mut r, &geom.vertices, &geom.indices[i * 3..i * 3 + 3]))
            .collect();
        let materials: Vec<_> = package
            .resources
            .materials
            .iter()
            .map(|m| {
                let deadline = Instant::now() + Duration::from_secs(20);
                loop {
                    if let Some(m) = r
                        .try_create_material(&MaterialDescriptor {
                            base_color: m.base_color,
                            double_sided: m.double_sided,
                            ..Default::default()
                        })
                        .unwrap()
                    {
                        break m;
                    }
                    assert!(Instant::now() < deadline);
                    std::thread::yield_now();
                }
            })
            .collect();
        let mut refs: Vec<_> = (0..4)
            .map(|i| instances(&mut r, &[models[i / 2]]))
            .collect();
        let mut draws: Vec<_> = refs
            .iter_mut()
            .enumerate()
            .map(|(i, inst)| {
                Draw::new(&meshes[i % 2], inst)
                    .with_material(&materials[i % 2])
                    .with_visibility(Visibility::Unculled)
            })
            .collect();
        render(&mut r, &color, &cam, &mut draws);
        if let Some(depth) = depth {
            let (c, d) = r.test_capture_full(&color);
            assert_eq!(actual, c);
            assert_eq!(depth, d);
        } else {
            assert_eq!(actual, r.test_capture_color(&color));
        }
        save(
            &format!("m4-ranges-{}x", samples as u32),
            513,
            321,
            &actual
                .chunks_exact(4)
                .flat_map(|p| p[..3].iter().copied())
                .collect::<Vec<_>>(),
        );
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if r.try_render_virtual(&color, &cam, &mut a, &mut scene, ViewMode::Normal)
                .unwrap()
                .is_some()
            {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        let normal = r.test_capture_color(&color);
        loop {
            if r.try_render_view(&color, &cam, &mut draws, ViewMode::Normal)
                .unwrap()
                .is_some()
            {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(normal, r.test_capture_color(&color));
        virtual_frame(&mut r, &color, &cam, &mut a, &mut scene);
        let before = a.stats().clone();
        let r_before = r.stats().uploads;
        for _ in 0..300 {
            virtual_frame(&mut r, &color, &cam, &mut a, &mut scene);
        }
        assert_eq!(a.stats().reads, before.reads);
        assert_eq!(a.stats().uploads, before.uploads);
        assert_eq!(a.stats().selection_runs, before.selection_runs);
        assert_eq!(r.stats().uploads.geometry, r_before.geometry);
        assert_eq!(a.stats().states[PageStatus::Resident as usize], 1);
        assert!(a.stats().geometry_peak <= a.options().geometry_bytes);
        let mut other = VirtualScene::new(
            &a,
            &[VirtualPlacement {
                mesh: 0,
                transform: Mat4::from_translation(Vec3::new(1000., 0., 0.)),
            }],
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !other.ready(&a) {
            a.advance(&mut r, &mut other, &cam, 321).unwrap();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(a.stats().reads, before.reads);
        assert_eq!(a.stats().uploads, before.uploads);
        assert_eq!(a.stats().states[PageStatus::Resident as usize], 1);
        assert_eq!(a.stats().cut_clusters, 6);
        drop(other);
        // Invalid syntax and binding change retain image equality, then valid TINT correction recovers.
        assert!(r.reload("invalid WGSL").is_err());
        virtual_frame(&mut r, &color, &cam, &mut a, &mut scene);
        assert_eq!(actual, r.test_capture_color(&color));
        assert!(
            r.reload(&SOURCE.replace("@group(0) @binding(0)", "@group(0) @binding(7)"))
                .is_err()
        );
        virtual_frame(&mut r, &color, &cam, &mut a, &mut scene);
        assert_eq!(actual, r.test_capture_color(&color));
        r.reload(&SOURCE.replace("vec3<f32>(1.0, 1.0, 1.0)", "vec3<f32>(0.6, 0.8, 1.0)"))
            .unwrap();
        virtual_frame(&mut r, &color, &cam, &mut a, &mut scene);
        assert_ne!(actual, r.test_capture_color(&color));
        r.reload(SOURCE).unwrap();
        virtual_frame(&mut r, &color, &cam, &mut a, &mut scene);
        assert_eq!(actual, r.test_capture_color(&color));
        eprintln!(
            "m4_settled samples={samples:?} frames=300 stats={:?}",
            a.stats()
        );
    }
}
#[test]
#[ignore = "Vulkan GPU; real async IO, missing/corrupt repair, staged admission, in-flight retiring replacement waves"]
fn gpu_virtual_streaming_failure_repair_and_retirement_waves() {
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    let color = target(r.device(), 1920, 1080);
    let (package, blobs) = fixture(4);
    let dir = directory("gpu-stream");
    write_pages(&package, &blobs, &dir);
    let missing = dir.join(
        package.pages[0]
            .sha256
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect::<String>(),
    );
    fs::remove_file(&missing).unwrap();
    let corrupt = dir.join(
        package.pages[1]
            .sha256
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect::<String>(),
    );
    fs::write(&corrupt, [0u8; 64]).unwrap();
    let mut a = VirtualAsset::new(
        package.clone(),
        dir.clone(),
        StreamOptions {
            geometry_bytes: 4 * PAGE_BYTES,
            io_delay: Duration::from_millis(100),
            ..Default::default()
        },
    )
    .unwrap();
    let mut scene = VirtualScene::from_package(&a).unwrap();
    let close = camera(0.);
    let start = Instant::now();
    virtual_frame(&mut r, &color, &close, &mut a, &mut scene);
    assert!(start.elapsed() >= Duration::from_millis(100));
    let deadline = Instant::now() + Duration::from_secs(10);
    while a.failure(PageId(0)).is_none() {
        virtual_frame(&mut r, &color, &close, &mut a, &mut scene);
        assert!(Instant::now() < deadline);
    }
    let failures = a.stats().failures;
    let reads = a.stats().reads;
    for _ in 0..10 {
        virtual_frame(&mut r, &color, &close, &mut a, &mut scene);
    }
    assert_eq!(failures, a.stats().failures);
    assert_eq!(reads, a.stats().reads);
    assert!(!scene.selections()[0].cut.clusters.is_empty());
    fs::write(&missing, &blobs[0]).unwrap();
    a.retry(PageId(0)).unwrap();
    while a.status(PageId(0)) != Some(PageStatus::Resident) {
        virtual_frame(&mut r, &color, &close, &mut a, &mut scene);
        assert!(Instant::now() < deadline);
    }
    // Moving farther selects the next level: actual corrupt read must preserve a legal fallback.
    drop(scene);
    scene = VirtualScene::from_package(&a).unwrap();
    let far = Camera::look_at(
        Vec3::new(0., 0., 100.),
        Vec3::ZERO,
        Vec3::Y,
        1.,
        0.05,
        10000.,
    )
    .unwrap();
    while a.failure(PageId(1)).is_none() {
        virtual_frame(&mut r, &color, &far, &mut a, &mut scene);
        assert!(Instant::now() < deadline);
    }
    fs::write(&corrupt, &blobs[1]).unwrap();
    a.retry(PageId(1)).unwrap();
    while a.status(PageId(1)) != Some(PageStatus::Resident) {
        virtual_frame(&mut r, &color, &far, &mut a, &mut scene);
        assert!(Instant::now() < deadline);
    }
    r.test_drain("failure/recovery finished");
    drop(scene);
    drop(a);
    let (device, queue) = device(true);
    let mut r = Renderer::new_with_options(
        device,
        queue,
        FORMAT,
        Limits::default(),
        RendererOptions {
            samples: SampleCount::Four,
        },
        SOURCE,
    )
    .unwrap();
    // Large 4x target keeps real pass work incomplete during the CPU-side replacement.
    let color = target(r.device(), 4096, 4096);
    let mut a = VirtualAsset::new(
        package,
        dir,
        StreamOptions {
            geometry_bytes: 4 * PAGE_BYTES,
            ..Default::default()
        },
    )
    .unwrap();
    let mut scene = VirtualScene::from_package(&a).unwrap();
    // Force bounded eviction across more representations than the pool can retain with staging.
    let mut observed_incomplete = 0;
    let mut observed_retiring = 0;
    let mut incomplete_evictions = 0;
    let mut peaks = vec![];
    for wave in 0..12 {
        if wave % 3 == 2 {
            virtual_frame(&mut r, &color, &close, &mut a, &mut scene);
            let (incomplete, _) = r.test_overlap();
            assert!(
                incomplete > 0,
                "focused last-use draw completed before retirement observation"
            );
            // Replacing the scene releases CPU protection, not the page's GPU last-use charge.
            drop(scene);
            scene = VirtualScene::from_package(&a).unwrap();
            retire_for_inflight_test(&mut a, r.completed_submission());
        } else {
            drop(scene);
            scene = VirtualScene::from_package(&a).unwrap();
        }
        let distance = [4., 100., 1000.][wave % 3];
        let cam = Camera::look_at(
            Vec3::new(0., 0., distance),
            Vec3::ZERO,
            Vec3::Y,
            1.,
            0.05,
            10000.,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut frames = 0;
        loop {
            frames += 1;
            let evictions = a.stats().evictions;
            virtual_frame(&mut r, &color, &cam, &mut a, &mut scene);
            let (incomplete, _) = r.test_overlap();
            observed_incomplete += usize::from(incomplete > 0);
            observed_retiring = observed_retiring.max(a.stats().retiring_bytes);
            if a.stats().evictions > evictions && incomplete > 0 {
                incomplete_evictions += 1;
            }
            assert!(a.stats().geometry_peak <= a.options().geometry_bytes);
            let target = [0, 1, 2][wave % 3];
            if frames >= 16 && scene.selections()[0].cut.pages == [PageId(target)] {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "wave {wave} did not settle: {:?}",
                a.stats()
            );
        }
        peaks.push(a.stats().geometry_peak);
    }
    assert!(
        observed_incomplete > 0,
        "no actual unfinished draws observed"
    );
    assert!(a.stats().evictions > 0, "waves never exercised replacement");
    assert!(
        a.stats().inflight_retirements > 0,
        "evicted page last use was already complete"
    );
    assert!(a.stats().retiring_peak > 0);
    r.test_drain("virtual waves final");
    a.advance(&mut r, &mut scene, &close, 1080).unwrap();
    eprintln!(
        "m4_waves incomplete={observed_incomplete} retiring_peak_observed={observed_retiring} incomplete_evictions={incomplete_evictions} peaks={peaks:?} stats={:?}",
        a.stats()
    );
}

#[allow(dead_code)]
#[path = "../examples/m4/cameras.rs"]
mod cameras;
use super::materials::prepared;

fn assert_registered_image(actual: &[u8], reference: &[u8], width: usize, height: usize) {
    let mask = |pixels: &[u8]| -> Vec<bool> {
        pixels
            .chunks_exact(4)
            .map(|p| p[..3] != pixels[..3])
            .collect()
    };
    let actual_mask = mask(actual);
    let reference_mask = mask(reference);
    let boundary = |mask: &[bool]| -> Vec<bool> {
        (0..width * height)
            .map(|i| {
                mask[i]
                    && (i % width == 0
                        || i % width == width - 1
                        || i < width
                        || i >= width * (height - 1)
                        || !mask[i - 1]
                        || !mask[i + 1]
                        || !mask[i - width]
                        || !mask[i + width])
            })
            .collect()
    };
    let actual_edge = boundary(&actual_mask);
    let reference_edge = boundary(&reference_mask);
    let near = |i: usize, edge: &[bool], radius_squared: isize| {
        (-2_isize..=2).any(|dy| {
            (-2_isize..=2).any(|dx| {
                let x = (i % width) as isize + dx;
                let y = (i / width) as isize + dy;
                dx * dx + dy * dy <= radius_squared
                    && x >= 0
                    && y >= 0
                    && x < width as isize
                    && y < height as isize
                    && edge[y as usize * width + x as usize]
            })
        })
    };
    let mut distances_squared = Vec::new();
    for (source, target) in [
        (&actual_edge, &reference_edge),
        (&reference_edge, &actual_edge),
    ] {
        for (i, &edge) in source.iter().enumerate().filter(|(_, edge)| **edge) {
            assert!(
                edge && near(i, target, 4),
                "registered silhouette exceeds 2px at ({},{})",
                i % width,
                i / width
            );
            distances_squared.push(if target[i] {
                0_u8
            } else if near(i, target, 1) {
                1
            } else {
                4
            });
        }
    }
    assert!(!distances_squared.is_empty());
    distances_squared.sort_unstable();
    let p99 = distances_squared[((distances_squared.len() - 1) as f64 * 0.99).ceil() as usize];
    assert!(p99 <= 1, "registered silhouette p99 exceeds 1px");
    let mut interior = 0;
    let mut disagreements = 0;
    for i in 0..width * height {
        if actual_mask[i]
            && reference_mask[i]
            && !near(i, &actual_edge, 4)
            && !near(i, &reference_edge, 4)
        {
            interior += 1;
            // The registered unlit sheet's blue/orange factors have opposite red/blue ordering.
            disagreements += usize::from(
                (actual[i * 4] > actual[i * 4 + 2]) != (reference[i * 4] > reference[i * 4 + 2]),
            );
        }
    }
    assert!(interior > 0);
    assert!(
        disagreements * 1000 <= interior,
        "registered material IDs disagree in >0.1% of interior"
    );
    eprintln!(
        "m4_image boundary_pixels={} p99_squared={p99} max_distance<=2 material_disagreements={disagreements} interior={interior}",
        distances_squared.len()
    );
}

#[test]
#[ignore = "Vulkan GPU; ordinary paced registered motion must publish the resident fin independently of missing pages"]
fn gpu_virtual_registered_moving_fin_publication() {
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    let color = target(r.device(), 1920, 1080);
    let cameras = cameras::Cameras::load(std::path::Path::new(
        "measurements/m4-cook/registration-initial/cameras.json",
    ))
    .unwrap();
    let mut a = VirtualAsset::open(
        std::path::Path::new("measurements/m4-cook/root-unit-a/cache"),
        std::path::Path::new("measurements/m4-cook/root-unit-a/material-thin.manifest"),
        StreamOptions::default(),
    )
    .unwrap();
    let mut scene = VirtualScene::from_package(&a).unwrap();
    for pose in &cameras.frames[..=210] {
        let start = Instant::now();
        virtual_frame(&mut r, &color, &pose.camera().unwrap(), &mut a, &mut scene);
        assert!(a.stats().geometry_peak <= 8 << 20);
        assert!(a.stats().metadata_bytes + a.stats().scratch_bound_bytes <= 8 << 20);
        std::thread::sleep(Duration::from_micros(16_670).saturating_sub(start.elapsed()));
    }
    assert_eq!(a.status(PageId(1)), Some(PageStatus::Resident));
    assert!(
        scene.selections()[0]
            .cut
            .active_groups
            .contains(&asset::virtual_mesh::GroupId(1004)),
        "resident orange fin blocked behind an unrelated plan page"
    );
    let pixels = r.test_capture_color(&color);
    save(
        "m4-moving-fin-210",
        1920,
        1080,
        &pixels
            .chunks_exact(4)
            .flat_map(|p| p[..3].iter().copied())
            .collect::<Vec<_>>(),
    );
    let cam = cameras.frames[210].camera().unwrap();
    let mut full = prepared::Prepared::load(std::path::Path::new(
        "measurements/m4-cook/registration-initial/material-thin.rla",
    ))
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        full.advance(&mut r).unwrap();
        if full.ready()
            && r.try_render_view(
                &color,
                &cam,
                &mut full.draws(Visibility::Unculled),
                ViewMode::BaseColor,
            )
            .unwrap()
            .is_some()
        {
            break;
        }
        r.poll().unwrap();
        assert!(Instant::now() < deadline, "original RLA reference deadline");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_registered_image(&pixels, &r.test_capture_color(&color), 1920, 1080);
}

#[test]
#[ignore = "Vulkan GPU; registered 8MiB near-view convergence and 300 quiet frames after actual page pressure"]
fn gpu_virtual_registered_pressure_quiescence() {
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    let color = target(r.device(), 1920, 1080);
    let cameras = cameras::Cameras::load(std::path::Path::new(
        "measurements/m4-cook/registration-initial/cameras.json",
    ))
    .unwrap();
    let cam = cameras.frames[270].camera().unwrap();
    let mut a = VirtualAsset::open(
        std::path::Path::new("measurements/m4-cook/root-unit-a/cache"),
        std::path::Path::new("measurements/m4-cook/root-unit-a/material-thin.manifest"),
        StreamOptions::default(),
    )
    .unwrap();
    let mut scene = VirtualScene::from_package(&a).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut frames = 0;
    loop {
        virtual_frame(&mut r, &color, &cam, &mut a, &mut scene);
        frames += 1;
        let states = a.stats().states;
        if states[PageStatus::Queued as usize]
            + states[PageStatus::Ready as usize]
            + states[PageStatus::Uploading as usize]
            == 0
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "registered pressure failed to converge: {:?}",
            a.stats()
        );
    }
    assert!(
        a.stats().residency_debt,
        "registered fixture did not reach pool capacity"
    );
    let before = a.stats().clone();
    for _ in 0..300 {
        virtual_frame(&mut r, &color, &cam, &mut a, &mut scene);
        assert_eq!(a.stats().reads, before.reads);
        assert_eq!(a.stats().uploads, before.uploads);
        assert_eq!(a.stats().selection_runs, before.selection_runs);
        assert!(a.stats().geometry_peak <= 8 << 20);
    }
    let pixels = r.test_capture_color(&color);
    save(
        "m4-registered-pressure",
        1920,
        1080,
        &pixels
            .chunks_exact(4)
            .flat_map(|p| p[..3].iter().copied())
            .collect::<Vec<_>>(),
    );
    eprintln!(
        "m4_registered_pressure convergence_frames={frames} settled_frames=300 stats={:?}",
        a.stats()
    );
    let mut full = prepared::Prepared::load(std::path::Path::new(
        "measurements/m4-cook/registration-initial/material-thin.rla",
    ))
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        full.advance(&mut r).unwrap();
        if full.ready()
            && r.try_render_view(
                &color,
                &cam,
                &mut full.draws(Visibility::Unculled),
                ViewMode::BaseColor,
            )
            .unwrap()
            .is_some()
        {
            break;
        }
        r.poll().unwrap();
        assert!(Instant::now() < deadline, "original RLA reference deadline");
        std::thread::sleep(Duration::from_millis(1));
    }
    let reference = r.test_capture_color(&color);
    assert_registered_image(&pixels, &reference, 1920, 1080);
}

#[test]
#[ignore = "Vulkan GPU; one-candidate moving progress with actual delayed reads, failure and repair"]
fn gpu_virtual_one_candidate_motion_progress_and_failed_read() {
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    let color = target(r.device(), 1920, 1080);
    let camera_at = |distance| {
        Camera::look_at(
            Vec3::new(0., 0., distance),
            Vec3::ZERO,
            Vec3::Y,
            1.,
            0.05,
            10000.,
        )
        .unwrap()
    };
    for fail in [false, true] {
        let (package, blobs) = fixture(4);
        let dir = directory("gpu-moving-progress");
        for (i, (page, blob)) in package.pages.iter().zip(&blobs).enumerate() {
            if !fail || i != 0 {
                fs::write(
                    dir.join(
                        page.sha256
                            .iter()
                            .map(|v| format!("{v:02x}"))
                            .collect::<String>(),
                    ),
                    blob,
                )
                .unwrap();
            }
        }
        let detail_path = dir.join(
            package.pages[0]
                .sha256
                .iter()
                .map(|v| format!("{v:02x}"))
                .collect::<String>(),
        );
        let mut a = VirtualAsset::new(
            package,
            dir,
            StreamOptions {
                max_candidates: 1,
                io_delay: Duration::from_millis(3),
                ..Default::default()
            },
        )
        .unwrap();
        let mut scene = VirtualScene::from_package(&a).unwrap();
        for _ in 0..120 {
            virtual_frame(&mut r, &color, &camera_at(22.), &mut a, &mut scene);
            if scene.selections()[0].cut.pages == [PageId(1)] {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(scene.selections()[0].cut.pages, [PageId(1)]);
        assert!(scene.selections()[0].demand.requested_pages.is_empty());
        let mut ticks = 0;
        for tick in 0..120 {
            virtual_frame(
                &mut r,
                &color,
                &camera_at(21. - tick as f32 * 0.001),
                &mut a,
                &mut scene,
            );
            ticks = tick + 1;
            assert!(a.stats().candidates <= 1);
            assert!(a.stats().geometry_peak <= a.options().geometry_bytes);
            assert!(
                scene.selections()[0]
                    .cut
                    .pages
                    .iter()
                    .all(|p| a.status(*p) == Some(PageStatus::Resident))
            );
            if (fail && a.failure(PageId(0)).is_some())
                || scene.selections()[0].cut.pages == [PageId(0)]
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        if fail {
            assert!(
                a.failure(PageId(0)).is_some(),
                "moving query never read missing detail"
            );
            assert_eq!(scene.selections()[0].cut.pages, [PageId(1)]);
            let reads = a.stats().reads;
            for tick in 0..30 {
                virtual_frame(
                    &mut r,
                    &color,
                    &camera_at(20. - tick as f32 * 0.001),
                    &mut a,
                    &mut scene,
                );
            }
            assert_eq!(
                a.stats().reads,
                reads,
                "failed page retried without explicit repair"
            );
            fs::write(detail_path, &blobs[0]).unwrap();
            a.retry(PageId(0)).unwrap();
            for tick in 0..120 {
                virtual_frame(
                    &mut r,
                    &color,
                    &camera_at(20. - tick as f32 * 0.001),
                    &mut a,
                    &mut scene,
                );
                if scene.selections()[0].cut.pages == [PageId(0)] {
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        assert_eq!(scene.selections()[0].cut.pages, [PageId(0)]);
        for _ in 0..4 {
            virtual_frame(&mut r, &color, &camera_at(22.), &mut a, &mut scene);
        }
        assert_eq!(
            scene.selections()[0].cut.pages,
            [PageId(1)],
            "capped root rebuild pinned unnecessary fine detail"
        );
        for _ in 0..8 {
            virtual_frame(&mut r, &color, &camera_at(21.), &mut a, &mut scene);
        }
        assert_eq!(scene.selections()[0].cut.pages, [PageId(0)]);
        for _ in 0..4 {
            virtual_frame(&mut r, &color, &camera_at(4000.), &mut a, &mut scene);
        }
        assert_eq!(scene.selections()[0].cut.pages, [PageId(3)]);
        let before = a.stats().clone();
        for _ in 0..300 {
            virtual_frame(&mut r, &color, &camera_at(4000.), &mut a, &mut scene);
        }
        assert_eq!(a.stats().reads, before.reads);
        assert_eq!(a.stats().uploads, before.uploads);
        assert_eq!(a.stats().selection_runs, before.selection_runs);
        eprintln!(
            "m4_one_candidate_progress fail={fail} moving_ticks={ticks} reads={} uploads={} quiet=300",
            a.stats().reads,
            a.stats().uploads
        );
    }
}
