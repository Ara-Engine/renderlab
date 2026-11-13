//! Lean shading on wgpu (lean_gpu.rs): frames shade full until the background lean compile
//! lands, and the lean pipeline shades the same frame byte for byte. Nested under
//! primary_ray_gpu_tests for its fixture helpers.
use super::*;

/// Re-shades the last frame's shading pass with `pipeline` and reads the HDR back.
fn reshade(
    r: &mut Renderer,
    pipeline: &wgpu::ComputePipeline,
    shading: primary_ray_gpu::Shading,
) -> Vec<u8> {
    let mut encoder = r.device.create_command_encoder(&Default::default());
    r.encode_primary_shade(
        &mut encoder,
        pipeline,
        primary_ray_gpu::SHADE_TILE,
        shading,
        None,
    );
    r.queue.submit([encoder.finish()]);
    capture(r)
}

#[test]
#[ignore = "shared GPU slot, experimental acknowledgment; small fixture, no window"]
fn gpu_lean_shading_lands_in_the_background_and_matches_full() {
    if !crate::program::lean_on() {
        return;
    }
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let (w, h) = (192, 128);
    let color = target(&r, w, h);
    let mesh = r
        .try_upload_mesh(&MeshData {
            vertices: &[[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]].map(
                |position| Vertex {
                    position,
                    normal: [0., 0., 1.],
                    uv: [0.; 2],
                    color: [0.8, 0.5, 0.3, 1.],
                },
            ),
            indices: &[0, 1, 2, 0, 2, 3],
        })
        .unwrap()
        .unwrap();
    let mut instances = r
        .try_upload_instances(&[Mat4::from_scale(Vec3::new(3., 2., 1.))])
        .unwrap()
        .unwrap();
    r.test_drain("lean fixture");
    r.set_lighting(&Lighting {
        lights: vec![Light::Directional {
            travel: Vec3::new(0.3, -0.4, -1.),
            lux: [8.; 3],
        }],
        environment: [0.02, 0.03, 0.05],
        ..Default::default()
    })
    .unwrap();
    let camera = Camera::look_at(
        Vec3::new(0., 0., 4.),
        Vec3::ZERO,
        Vec3::Y,
        60f32.to_radians(),
        0.1,
        100.,
    )
    .unwrap();
    // Frames keep shading full while the one lean compile runs; it lands within the budget.
    let started = std::time::Instant::now();
    let shading = loop {
        let report = render(
            &mut r,
            &color,
            &camera,
            &mut [Draw::new(&mesh, &mut instances)],
        );
        assert_eq!(report.primary, PrimaryVisibility::Traced);
        let lean = &r.test_standard().lean[0];
        if let Some(i) = lean.iter().position(Option::is_some) {
            assert!(
                lean.iter().filter(|p| p.is_some()).count() == 1,
                "one shading, one compile"
            );
            break i;
        }
        assert!(
            started.elapsed().as_secs() < 120,
            "the lean pipeline never landed"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let shading_kind = [
        primary_ray_gpu::Shading::Hard,
        primary_ray_gpu::Shading::Soft,
    ][shading];
    // The same frame shaded by the full and the lean pipeline: identical bytes.
    let p = r.test_standard();
    let [full, lean]: [wgpu::ComputePipeline; 2] =
        [&p.traced[shading], &p.lean[0][shading]].map(|p| (**p.as_ref().unwrap()).clone());
    let full_image = reshade(&mut r, &full, shading_kind);
    let lean_image = reshade(&mut r, &lean, shading_kind);
    assert!(full_image == lean_image, "lean shading changed the picture");
}
