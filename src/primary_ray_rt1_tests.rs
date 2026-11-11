//! RT1's current-source, single-upload film-defect diagnostic. This is not a
//! replacement for the static/motion acceptance gates or a default promotion.
use super::*;
use sha2::{Digest, Sha256};

#[test]
#[ignore = "owned GPU lease, no game, RENDERLAB_PRIMARY_MANIFEST; offscreen"]
fn gpu_rt1_film_defects() {
    let m = manifest();
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    let (mut asset, cameras) = load_quarry(&m, &mut r);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let color = target(&r, 1920, 1080);
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let source = primary_ray_gpu::shader_sources(SOURCE).unwrap()[1].clone();
    let module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("RT1 separate eight-ray control"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let separate = r.view.primary.as_ref().unwrap().fresh_pipeline(
        &r.device,
        &r.ledger,
        &module,
        "fresh_compute",
        &[("FRESH_DIRECTIONS", 8.0)],
    );
    let mut rows = vec![];
    for pose in 0..2 {
        let camera = cameras.frames[pose].camera().unwrap();
        // This gate exports the 256-ray reference explicitly, independent of any
        // future device-default decision.
        r.set_soft_shadows(crate::SoftShadows::Exact);
        traced_frame(&mut r, &color, &camera, &mut asset, true);
        let [exact_hits, exact_counts] = interactive_outputs(&r);
        fs::write(out.join(format!("pose-{pose}-exact.u32")), &exact_counts).unwrap();
        let mut encoder = r.device.create_command_encoder(&Default::default());
        {
            let primary = r.view.primary.as_ref().unwrap();
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&separate);
            pass.set_bind_group(0, primary.test_fresh_bindings().1, &[]);
            pass.set_bind_group(3, r.lighting.bind(), &[]);
            pass.dispatch_workgroups(1920 / 8, 1080 / 4, 1);
        }
        r.queue.submit([encoder.finish()]);
        let [_, separate_counts] = interactive_outputs(&r);
        r.set_soft_shadows(crate::SoftShadows::Interactive);
        let mut hashes = vec![];
        let mut counts = vec![];
        let mut hits = vec![];
        for repeat in 0..8 {
            traced_frame(&mut r, &color, &camera, &mut asset, true);
            let [new_hits, new_counts] = interactive_outputs(&r);
            let hdr = capture(&mut r);
            let hash =
                [&new_hits, &new_counts, &hdr].map(|data| format!("{:x}", Sha256::digest(data)));
            if repeat == 0 {
                counts = new_counts;
                hits = new_hits;
                fs::write(out.join(format!("pose-{pose}-interactive.u32")), &counts).unwrap();
                fs::write(out.join(format!("pose-{pose}-interactive.hit.u32")), &hits).unwrap();
                fs::write(out.join(format!("pose-{pose}-interactive.rgba16f")), &hdr).unwrap();
            } else {
                assert_eq!(hashes[0], hash, "static RT1 pose {pose}, repeat {repeat}");
            }
            hashes.push(hash);
        }
        let differing = |a: &[u8], b: &[u8], stride| {
            a.chunks_exact(stride)
                .zip(b.chunks_exact(stride))
                .filter(|(a, b)| a != b)
                .count()
        };
        let row = serde_json::json!({
            "pose": pose,
            "hashes_hits_counts_hdr": hashes,
            "fused_separate_count_differences": differing(&counts, &separate_counts, 4),
            "interactive_exact_hit_differences": differing(&hits, &exact_hits, 16),
        });
        rows.push(row);
        // Preserve both sides if a handoff difference is found; it is evidence,
        // not grounds for discarding those receivers from the film score.
        fs::write(
            out.join(format!("pose-{pose}-separate.u32")),
            &separate_counts,
        )
        .unwrap();
        fs::write(out.join(format!("pose-{pose}-exact.hit.u32")), &exact_hits).unwrap();
        fs::write(
            out.join("current-source.json"),
            serde_json::to_vec_pretty(&rows).unwrap(),
        )
        .unwrap();
    }
    r.test_drain("RT1 film defect diagnostic");
}
