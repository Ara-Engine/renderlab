//! Requested-limit pipeline coverage and CPU view-projection reuse regression.
use super::*;

#[test]
#[ignore = "owned GPU lease; creates all non-RT engine/Virtual pipeline variants"]
fn gpu_non_rt_requested_limit_pipelines() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::debugging(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    eprintln!(
        "FIX1 adapter={:?} max_storage_buffers_per_shader_stage={}",
        adapter.get_info(),
        adapter.limits().max_storage_buffers_per_shader_stage
    );
    let requested = crate::visibility_limits(wgpu::Limits::default());
    assert!(requested.check_limits(&adapter.limits()));
    let features = crate::RECEIVER_FEATURES
        | crate::VISIBILITY_FEATURES
        | crate::VISIBILITY_RASTER_FEATURES
        | crate::VIRTUAL_FEATURES;
    let camera = Camera::look_at(
        Vec3::new(0.2, 1.2, 2.5),
        Vec3::ZERO,
        Vec3::Y,
        45f32.to_radians(),
        0.01,
        100.,
    )
    .unwrap();
    let make = |limits, samples| {
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: features,
            required_limits: limits,
            ..Default::default()
        }))
        .unwrap();
        assert!(!device.features().intersects(
            wgpu::Features::EXPERIMENTAL_RAY_QUERY
                | wgpu::Features::EXPERIMENTAL_RAY_HIT_VERTEX_RETURN
        ));
        Renderer::new_with_options(
            device,
            queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            Limits::default(),
            RendererOptions {
                samples,
                ..Default::default()
            },
            SOURCE,
        )
        .unwrap()
    };
    // A device below the optional path's minimum still creates/renders raster pipelines,
    // even with all the visibility features. A real lower-limit adapter can use defaults.
    for (limits, samples) in [
        (wgpu::Limits::default(), SampleCount::One),
        (
            wgpu::Limits {
                max_storage_buffers_per_shader_stage: 10,
                ..requested.clone()
            },
            SampleCount::One,
        ),
        (requested.clone(), SampleCount::Four),
    ] {
        let mut r = make(limits, samples);
        assert_eq!(r.primary_visibility(), PrimaryVisibility::Raster);
        assert!(
            r.set_primary_visibility(PrimaryVisibility::VisibilityBuffer)
                .is_err()
        );
        r.set_lighting_and_shadows(
            &Lighting {
                lights: vec![Light::Directional {
                    travel: -Vec3::new(0.4, 0.8, 0.3).normalize(),
                    lux: [1000.; 3],
                }],
                environment: [0.03; 3],
                exposure_stops: -8.,
            },
            &Shadows {
                resolution: 64,
                requests: vec![ShadowRequest::Directional {
                    split_ends: vec![10., 40.],
                }],
                ..Default::default()
            },
        )
        .unwrap();
        let (mesh, mut instances) = geometry(
            &mut r,
            &[[-1., 0., -1.], [1., 0., -1.], [0., 0., 1.]],
            &[0, 2, 1],
        );
        let color = target(&r, 64, 64);
        render(
            &mut r,
            &color,
            &camera,
            &mut [Draw::new(&mesh, &mut instances)],
        );
        r.test_drain("FIX1 portable raster pipelines");
    }
    let mut r = make(requested.clone(), SampleCount::One);
    assert_eq!(r.primary_visibility(), PrimaryVisibility::VisibilityBuffer);
    assert_eq!(r.soft_shadows(), SoftShadows::Virtual);
    light(&mut r);
    let (mesh, mut instances) = geometry(
        &mut r,
        &[
            [-2., 0., -2.],
            [2., 0., -2.],
            [2., 0., 2.],
            [-2., 0., 2.],
            [-0.4, 0., 0.],
            [0.4, 0., 0.],
            [0., 0.8, 0.],
        ],
        &[0, 2, 1, 0, 3, 2, 4, 5, 6],
    );
    let color = target(&r, 128, 96);
    for lookup in [
        VirtualLookup::Exact,
        VirtualLookup::Map,
        VirtualLookup::Hybrid,
        VirtualLookup::Disk,
        VirtualLookup::Surface,
    ] {
        r.set_virtual_shadow_settings(VirtualShadowSettings {
            pages: 4,
            lookup,
            ..Default::default()
        })
        .unwrap();
        render(
            &mut r,
            &color,
            &camera,
            &mut [Draw::new(&mesh, &mut instances)],
        );
        r.test_drain("FIX1 non-RT Virtual pipelines");
        assert!(r.virtual_shadow_stats().page_work);
    }
    // The request covers every sun-shadow pipeline layout these lookups made, counted from their
    // groups' entries (src/storage_need.rs) rather than a literal: a layout that outgrows
    // `visibility_limits` fails here by name.
    let (label, need) = crate::renderer::storage_need::largest().unwrap();
    eprintln!("FIX1 heaviest sun-shadow layout {label}: {need} storage buffers per compute stage");
    assert!(
        requested.max_storage_buffers_per_shader_stage >= need,
        "{label} sees {need} storage buffers per compute stage; visibility_limits requests {}          (raise virtual_shadow_gpu.rs STORAGE_BUFFERS)",
        requested.max_storage_buffers_per_shader_stage
    );
    assert_eq!(
        need,
        crate::renderer::SUN_SHADOW_STORAGE_BUFFERS,
        "{label}: STORAGE_BUFFERS should name the heaviest layout's count"
    );
    // Surface's still fast path must survive, and any projection change must leave it.
    let depth_changed = Camera::look_at(
        Vec3::new(0.2, 1.2, 2.5),
        Vec3::ZERO,
        Vec3::Y,
        45f32.to_radians(),
        0.02,
        80.,
    )
    .unwrap();
    for (view, page_work) in [
        (camera, false),
        (camera.jittered([0.25, -0.25]), true),
        (camera.jittered([0.25, -0.25]), false),
        (camera.jittered([-0.25, 0.25]), true),
        (camera, true),
        (camera, false),
        (depth_changed, true),
        (depth_changed, false),
    ] {
        render(
            &mut r,
            &color,
            &view,
            &mut [Draw::new(&mesh, &mut instances)],
        );
        r.test_drain("FIX1 view-projection reuse");
        assert_eq!(r.virtual_shadow_stats().page_work, page_work);
    }
    // Compile and run the non-RT reconstruction pipeline too; its jitter invalidates
    // Virtual without the former unconditional rolling reset in renderer.rs.
    r.set_reconstruction(Reconstruction::identity()).unwrap();
    for _ in 0..3 {
        render(
            &mut r,
            &color,
            &camera,
            &mut [Draw::new(&mesh, &mut instances)],
        );
        r.test_drain("FIX1 non-RT reconstruction pipeline");
        assert!(r.virtual_shadow_stats().page_work);
    }
}

/// Showcase's editor-play stall (#192): a cut changed the clip-map ladder while the Disk lookup
/// held its page requests, a trace tile the last full mark had not covered held a longer
/// ladder's receiver level, and the lookup's band walk from that level never ended. Held frames
/// must finish whatever the receiver levels hold: a level of an older mark is chosen anew, and a
/// level past the ladder is dropped and counted.
#[test]
#[ignore = "owned GPU lease; renders the Disk lookup's held frames"]
fn gpu_disk_held_requests_survive_stale_levels() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::debugging(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: crate::RECEIVER_FEATURES
            | crate::VISIBILITY_FEATURES
            | crate::VISIBILITY_RASTER_FEATURES
            | crate::VIRTUAL_FEATURES,
        required_limits: crate::visibility_limits(wgpu::Limits::default()),
        ..Default::default()
    }))
    .unwrap();
    let mut r = Renderer::new_with_options(
        device,
        queue,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        Limits::default(),
        RendererOptions::default(),
        SOURCE,
    )
    .unwrap();
    assert_eq!(r.soft_shadows(), SoftShadows::Virtual);
    light(&mut r);
    r.set_virtual_shadow_settings(VirtualShadowSettings {
        pages: 4,
        lookup: VirtualLookup::Disk,
        ..Default::default()
    })
    .unwrap();
    let (mesh, mut instances) = geometry(
        &mut r,
        &[
            [-2., 0., -2.],
            [2., 0., -2.],
            [2., 0., 2.],
            [-2., 0., 2.],
            [-0.4, 0., 0.],
            [0.4, 0., 0.],
            [0., 0.8, 0.],
        ],
        &[0, 2, 1, 0, 3, 2, 4, 5, 6],
    );
    let color = target(&r, 128, 96);
    let camera = Camera::look_at(
        Vec3::new(0.2, 1.2, 2.5),
        Vec3::ZERO,
        Vec3::Y,
        45f32.to_radians(),
        0.01,
        100.,
    )
    .unwrap();
    render(&mut r, &color, &camera, &mut [Draw::new(&mesh, &mut instances)]);
    r.test_drain("full mark");
    // Held frames: only TR1's jitter moves the camera, so the mark does not run.
    let mut step = 0;
    let mut held_frame = |r: &mut Renderer, label: &str| {
        step += 1;
        let jitter = [0.25 * (step % 3) as f32 - 0.25, -0.25];
        render(r, &color, &camera.jittered(jitter), &mut [Draw::new(&mesh, &mut instances)]);
        r.test_drain(label);
        let (_, held, ladder, generation) = r.test_levels();
        assert!(held, "{label}: only TR1's jitter moved the camera, the requests hold");
        (ladder, generation)
    };
    let (ladder, generation) = held_frame(&mut r, "held");
    // This mark's levels past the ladder (no mark rewrites them while held): every held frame
    // finishes, and the lookup counts what it drops.
    r.test_stale_levels(generation << 8 | (ladder + 2));
    for k in 0..4 {
        held_frame(&mut r, "held over levels past the ladder");
        if k >= 1 && r.virtual_shadow_stats().stale_levels != 0 {
            break;
        }
    }
    assert!(r.virtual_shadow_stats().stale_levels != 0, "the lookup counts the levels it drops");
    // An older mark's levels (the trace tiles a mark did not cover): the lookup chooses them
    // anew, so once the readbacks of the frames above are in, nothing more is dropped.
    r.test_stale_levels((generation.wrapping_sub(1) & 0xff_ffff) << 8 | (ladder + 2));
    for _ in 0..4 {
        held_frame(&mut r, "held over an older mark's levels");
    }
    let settled = r.virtual_shadow_stats().stale_levels;
    for _ in 0..3 {
        held_frame(&mut r, "held over an older mark's levels");
    }
    assert_eq!(
        r.virtual_shadow_stats().stale_levels,
        settled,
        "an older mark's levels are chosen anew, not dropped"
    );
}
