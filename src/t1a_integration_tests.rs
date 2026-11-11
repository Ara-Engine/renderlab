//! One offscreen integration lease, one renderer and one quarry upload.
use super::*;
use std::collections::BTreeMap;

/// The frozen Interactive acceptance's 36 artifacts, with all seeds on one scene.
fn interactive_baseline(
    r: &mut Renderer,
    asset: &mut prepared::Prepared,
    color: &wgpu::Texture,
    cameras: &cameras::Cameras,
    out: &std::path::Path,
    hashes: &mut Hashes,
) {
    let module = seeded_module(r);
    let soft = r.shadows.settings.clone();
    let mut hard = soft.clone();
    hard.directional_disk_angle = 0.;
    for (index, pose) in [0, 1, 0].into_iter().enumerate() {
        let camera = cameras.frames[pose].camera().unwrap();
        let name = |kind: &str| out.join(format!("pose-{index}-{kind}"));
        r.set_soft_shadows(crate::SoftShadows::Exact);
        traced_frame(r, color, &camera, asset, true);
        hashes.write(name("exact.rgba16f"), settled(r)).unwrap();
        let [world, normal, _] = receivers(r);
        hashes.write(name("world.f32"), world).unwrap();
        hashes.write(name("normal.f32"), normal).unwrap();
        for seed in 0..4 {
            let mut pipeline = r.primary.as_ref().unwrap().interactive_pipeline(
                &r.device,
                &r.ledger,
                &module,
                &[("FRESH_SEED", f64::from(seed)), ("FRESH_DIRECTIONS", 8.0)],
            );
            r.set_soft_shadows(crate::SoftShadows::Interactive);
            std::mem::swap(
                &mut r.pipeline.primary.as_mut().unwrap().interactive,
                &mut pipeline,
            );
            traced_frame(r, color, &camera, asset, true);
            hashes
                .write(
                    name(&format!("seed-{seed}-interactive.rgba16f")),
                    settled(r),
                )
                .unwrap();
            hashes
                .write(
                    name(&format!("seed-{seed}-interactive.u32")),
                    &interactive_outputs(r)[1],
                )
                .unwrap();
            std::mem::swap(
                &mut r.pipeline.primary.as_mut().unwrap().interactive,
                &mut pipeline,
            );
        }
        r.set_shadows(&hard).unwrap();
        traced_frame(r, color, &camera, asset, true);
        hashes.write(name("hard.rgba16f"), settled(r)).unwrap();
        r.set_shadows(&soft).unwrap();
    }
}

/// Exercise each public program through cold starts, camera changes and full histories.
fn program_resets(
    r: &mut Renderer,
    asset: &mut prepared::Prepared,
    color: &wgpu::Texture,
    cameras: &[Camera],
) -> serde_json::Value {
    let mut rows = vec![];
    for (name, source) in [
        ("standard", STANDARD),
        ("toon", include_str!("../shaders/programs/toon.wgsl")),
    ] {
        r.test_reload_standard(source).unwrap();
        r.set_soft_shadows(crate::SoftShadows::Interactive);
        let expected: Vec<_> = cameras
            .iter()
            .map(|camera| {
                traced_frame(r, color, camera, asset, true);
                let hdr = digest(&settled(r));
                let geometry = interactive_outputs(r).map(|b| digest(&b));
                (hdr, geometry)
            })
            .collect();
        reset_film(r);
        for pose in [0, 1, 2, 0] {
            // Each camera change must reset by itself, with no explicit history reset.
            traced_frame(r, color, &cameras[pose], asset, true);
            assert!(r.primary.as_ref().unwrap().film.as_ref().unwrap().reset);
            assert_eq!(
                digest(&settled(r)),
                expected[pose].0,
                "{name} camera reset HDR"
            );
            assert_eq!(
                interactive_outputs(r).map(|b| digest(&b)),
                expected[pose].1,
                "{name} camera reset trace"
            );
            traced_frame(r, color, &cameras[pose], asset, true);
            assert!(!r.primary.as_ref().unwrap().film.as_ref().unwrap().reset);
        }
        let mut expected_run = None;
        for _ in 0..8 {
            reset_film(r);
            let mut run = vec![];
            for n in 1..=128 {
                traced_frame(r, color, &cameras[0], asset, true);
                if [1, 2, 4, 8, 128].contains(&n) {
                    run.push(digest(&settled(r)));
                }
            }
            run.push(digest(&history(r)));
            if let Some(expected) = &expected_run {
                assert_eq!(&run, expected, "{name} repeat incl. history");
            } else {
                expected_run = Some(run);
            }
        }
        // Reload rejection retains a live temporal pipeline and history; accepted edits reset.
        let revision = r.revision;
        assert!(r.test_reload_standard("invalid WGSL").is_err());
        assert_eq!(r.revision, revision);
        traced_frame(r, color, &cameras[0], asset, true);
        assert!(!r.primary.as_ref().unwrap().film.as_ref().unwrap().reset);
        r.test_reload_standard(source).unwrap();
        traced_frame(r, color, &cameras[0], asset, true);
        assert!(r.primary.as_ref().unwrap().film.as_ref().unwrap().reset);
        assert_eq!(digest(&settled(r)), expected[0].0);
        // A prelude rejection keeps the pipelines; a valid reload is atomic and resets.
        assert!(r.reload("invalid WGSL").is_err());
        r.reload(SOURCE).unwrap();
        traced_frame(r, color, &cameras[0], asset, true);
        assert!(r.primary.as_ref().unwrap().film.as_ref().unwrap().reset);
        assert_eq!(digest(&settled(r)), expected[0].0);
        rows.push(serde_json::json!({"program":name,"camera_resets":4,"repeats":8,"digests":expected_run,"reload":true}));
    }
    r.test_reload_standard(STANDARD).unwrap();
    serde_json::json!(rows)
}

#[test]
#[ignore = "owned GPU lease; 4 GiB preflight/2 GiB abort; one uploaded quarry"]
fn gpu_t1a_integration_batch() {
    let m = manifest();
    let _validation = validation::begin(true);
    let root = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&root).unwrap();
    let mut r = renderer(true);
    let (mut asset, cameras) = load_quarry(&m, &mut r);
    let color = target(&r, 1920, 1080);
    let mut primary = Hashes::load(m["primary_hashes"].as_str().unwrap());
    primary_baseline(&mut r, &mut asset, &color, &cameras, &root, &mut primary);
    primary.finish(&root.join("primary-hashes.json"), 285);
    eprintln!("T1a primary 285/285 identical");
    let mut interactive = Hashes::load(m["interactive_hashes"].as_str().unwrap());
    interactive_baseline(
        &mut r,
        &mut asset,
        &color,
        &cameras,
        &root,
        &mut interactive,
    );
    interactive.finish(&root.join("interactive-hashes.json"), 36);
    eprintln!("T1a Interactive 36/36 identical");
    let resets = program_resets(&mut r, &mut asset, &color, &t1_cameras(&m));
    fs::write(
        root.join("program-resets.json"),
        serde_json::to_vec_pretty(&resets).unwrap(),
    )
    .unwrap();
    temporal_acceptance(&mut r, &mut asset, &color, &m);
    r.set_soft_shadows(crate::SoftShadows::Exact);
    r.test_drain("T1a integration complete");
    eprintln!("T1a integration complete: one renderer, asset loaded once");
}

/// Compare SHA-256 identities and retain only a small manifest, not bulk baseline dumps.
struct Hashes {
    expected: BTreeMap<String, String>,
    actual: BTreeMap<String, String>,
}
impl Hashes {
    fn load(path: &str) -> Self {
        Self {
            expected: serde_json::from_slice(&fs::read(path).unwrap()).unwrap(),
            actual: BTreeMap::new(),
        }
    }
    fn write(
        &mut self,
        path: impl AsRef<std::path::Path>,
        bytes: impl AsRef<[u8]>,
    ) -> Result<(), String> {
        let name = path
            .as_ref()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let hash = digest(bytes.as_ref());
        self.actual.insert(name.clone(), hash.clone());
        if self.expected.get(&name) != Some(&hash) {
            return Err(format!("baseline mismatch: {name}"));
        }
        Ok(())
    }
    fn finish(self, path: &std::path::Path, count: usize) {
        assert_eq!(self.actual.len(), count);
        assert_eq!(self.actual, self.expected);
        fs::write(path, serde_json::to_vec_pretty(&self.actual).unwrap()).unwrap();
    }
}

/// The frozen primary acceptance's 285 artifacts, using the batch's uploaded scene.
fn primary_baseline(
    r: &mut Renderer,
    asset: &mut prepared::Prepared,
    color: &wgpu::Texture,
    cameras: &cameras::Cameras,
    out: &std::path::Path,
    hashes: &mut Hashes,
) {
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let module = seeded_module(r);
    let primary = r.primary.as_ref().unwrap();
    let mut seeded: Vec<_> = (0..=80u32)
        .map(|s| {
            primary.fresh_pipeline(
                &r.device,
                &r.ledger,
                &module,
                "fresh_compute",
                &[("FRESH_SEED", f64::from(s))],
            )
        })
        .collect();
    let mut noop = primary.fresh_pipeline(&r.device, &r.ledger, &module, "fresh_noop", &[]);
    let mut sources = primary_ray_gpu::shading_sources(SOURCE, STANDARD).unwrap();
    sources[1] = edit(
        &sources[1],
        (COUNT_DECODE, &COUNT_DECODE.replace("/256.0", "/16777216.0")),
    );
    let mut fixed = shading_pipelines(r, &sources);
    let soft = r.shadows.settings.clone();
    let mut hard = soft.clone();
    hard.directional_disk_angle = 0.0;
    for (index, pose) in [0, 1, 0].into_iter().enumerate() {
        let camera = cameras.frames[pose].camera().unwrap();
        let name = |kind: &str| out.join(format!("pose-{index}-{kind}"));
        traced_frame(r, color, &camera, asset, true);
        let production = capture(r);
        let [world, normal, counts] = receivers(r);
        let hit = hit_record(r);
        traced_frame(r, color, &camera, asset, true);
        assert_eq!(capture(r), production, "repeat traced HDR differs");
        assert_eq!(receivers(r)[2], counts, "repeat traced visibility differs");
        let mut reference = vec![0u32; counts.len() / 4];
        for (seed, pipeline) in seeded.iter().enumerate() {
            let mut encoder = r.device.create_command_encoder(&Default::default());
            r.encode_primary_fresh(&mut encoder, pipeline, None);
            r.queue.submit([encoder.finish()]);
            let seed_counts = receivers(r)[2].clone();
            if seed == 0 {
                assert_eq!(seed_counts, counts, "seed 0 differs from production");
            }
            if (1..=64).contains(&seed) {
                for (sum, c) in reference.iter_mut().zip(seed_counts.chunks_exact(4)) {
                    *sum += u32::from_le_bytes(c.try_into().unwrap());
                }
            }
            hashes
                .write(name(&format!("seed-{seed}-production.u32")), &seed_counts)
                .unwrap();
        }
        for (seed, pipeline) in seeded.iter_mut().enumerate().take(4).skip(1) {
            swap_compute(r, pipeline);
            traced_frame(r, color, &camera, asset, true);
            swap_compute(r, pipeline);
            hashes
                .write(name(&format!("seed-{seed}-production.rgba16f")), capture(r))
                .unwrap();
        }
        // Mean of 64 seeds at 2^-24 fixed point: sum * 2^24 / 16384, exact in u32.
        let fixed_bytes: Vec<u8> = reference
            .iter()
            .flat_map(|s| (s * 1024).to_le_bytes())
            .collect();
        swap_compute(r, &mut noop);
        r.test_standard().swap_traced(&mut fixed);
        let t = &r
            .fresh
            .as_ref()
            .unwrap()
            .targets
            .as_ref()
            .unwrap()
            .visibility
            .texture;
        r.queue.write_texture(
            t.as_image_copy(),
            &fixed_bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(t.width() * 4),
                rows_per_image: Some(t.height()),
            },
            t.size(),
        );
        traced_frame(r, color, &camera, asset, true);
        swap_compute(r, &mut noop);
        r.test_standard().swap_traced(&mut fixed);
        hashes.write(name("reference.rgba16f"), capture(r)).unwrap();
        traced_frame(r, color, &camera, asset, false);
        let raster = capture(r);
        let [raster_world, raster_normal, raster_counts] = receivers(r);
        r.set_shadows(&hard).unwrap();
        traced_frame(r, color, &camera, asset, true);
        let traced_hard = capture(r);
        traced_frame(r, color, &camera, asset, false);
        let raster_hard = capture(r);
        r.set_shadows(&soft).unwrap();
        for (kind, bytes) in [
            ("world.f32", &world),
            ("normal.f32", &normal),
            ("hit.u32", &hit),
            ("production.rgba16f", &production),
            ("raster.rgba16f", &raster),
            ("raster-world.f32", &raster_world),
            ("raster-normal.f32", &raster_normal),
            ("raster-counts.u32", &raster_counts),
            ("traced-hard.rgba16f", &traced_hard),
            ("raster-hard.rgba16f", &raster_hard),
        ] {
            hashes.write(name(kind), bytes).unwrap();
        }
    }
}
