//! The final gather's stable probes (gi_receiver_pass.wgsl fg_state) where their wait costs
//! most: a light switched on under settled probes (`light-on`: the emitter below the furnace's
//! slab goes from dark to 20 cd/m^2 by a material swap) and a mover's shadow crossing them
//! (`mover`: a box sweeps over the slab's top under the open sky). Each settles SETTLE frames,
//! then changes, and writes a PFM a frame of its view through STRIP more frames, with the
//! frame's mean and whether the cache saw a change (`GiState::recently_changed`, which puts every
//! probe at the half rate). Run it twice, the second time with RENDERLAB_SHADE_EDITS turning
//! FG_STABLE_RATE to 2u (the half rate everywhere), and compare the strips.
//!   RENDERLAB_FINAL_GATHER=1 RENDERLAB_RATE_OUT=<dir> cargo test --lib fg_rate_strip -- --ignored --nocapture
//!   RENDERLAB_FURNACE_PATH=rt|nort (default nort)
use super::*;

const SETTLE: u32 = 120;
const STRIP: u32 = 48;

#[test]
#[ignore = "GPU lease; RENDERLAB_FINAL_GATHER=1, RENDERLAB_RATE_OUT, RENDERLAB_FURNACE_PATH"]
fn fg_rate_strip() {
    let rt = std::env::var("RENDERLAB_FURNACE_PATH").as_deref() == Ok("rt");
    let out = PathBuf::from(std::env::var("RENDERLAB_RATE_OUT").expect("RENDERLAB_RATE_OUT"));
    for case in ["light-on", "mover"] {
        let dir = out.join(case);
        std::fs::create_dir_all(&dir).unwrap();
        let rows = strip(case, rt, &dir);
        println!("rate {case} {rows}");
        std::fs::write(dir.join("strip.json"), serde_json::to_vec_pretty(&rows).unwrap()).unwrap();
    }
}

fn strip(case: &str, rt: bool, dir: &Path) -> Json {
    let (device, queue) = if rt { rt_device() } else { no_rt_device() };
    let options = RendererOptions {
        samples: SampleCount::One,
        ..Default::default()
    };
    let mut r = if rt {
        let mut r = Renderer::new_with_hardware_shadows(
            device,
            queue,
            FORMAT,
            Limits::default(),
            options,
            HardwareShadowOptions::default(),
            SOURCE,
        )
        .unwrap();
        r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
        r
    } else {
        Renderer::new_with_options(device, queue, FORMAT, Limits::default(), options, SOURCE).unwrap()
    };
    r.set_lighting(&Lighting {
        lights: vec![],
        environment: [1.0; 3],
        exposure_stops: 0.0,
    })
    .unwrap();
    r.set_reconstruction(Reconstruction::off()).unwrap();
    r.set_indirect(IndirectSettings::default()).unwrap();
    let target = color_target(&r, SIZE);
    let program = r
        .try_create_program(&ProgramDescriptor {
            label: "rate",
            source: FURNACE_PROGRAM,
        })
        .unwrap();
    // The furnace's slab and its emitter below; `mover` adds a box above the slab's top.
    let slab = box_faces(Vec3::ZERO, Vec3::new(1.5, 0.01, 1.5), false);
    let emitter = vec![quad(Vec3::new(0.0, -0.4, 0.0), Vec3::Y, Vec3::X * 0.75, 0.75)];
    let blocker = box_faces(Vec3::ZERO, Vec3::splat(0.15), false);
    let mut parts: Vec<(Vec<([Vec3; 4], Vec3)>, [f32; 3], [f32; 3])> =
        vec![(slab, [0.5; 3], [0.0; 3]), (emitter, [0.0; 3], [0.0; 3])];
    if case == "mover" {
        parts.push((blocker, [0.5; 3], [0.0; 3]));
    }
    let material = |r: &mut Renderer, albedo: [f32; 3], glow: [f32; 3]| {
        let parameters = [("glow", crate::Value::Vec3(glow))];
        loop {
            let descriptor = MaterialDescriptor {
                base_color: [albedo[0], albedo[1], albedo[2], 1.0],
                metallic: 0.0,
                roughness: 1.0,
                double_sided: true,
                program: Some(&program),
                parameters: &parameters,
                ..Default::default()
            };
            if let Some(m) = r.try_create_material(&descriptor).unwrap() {
                break m;
            }
            r.test_drain("rate material admission");
        }
    };
    let mut lit = material(&mut r, [0.0; 3], [20.0; 3]);
    let mut meshes = vec![];
    for (faces, albedo, glow) in &parts {
        let mut vertices = vec![];
        let mut indices = vec![];
        for (corners, n) in faces {
            let base = vertices.len() as u32;
            vertices.extend(corners.iter().map(|p| Vertex {
                position: p.to_array(),
                normal: n.to_array(),
                uv: [0.0; 2],
                color: [1.0; 4],
            }));
            indices.extend([0, 1, 2, 0, 2, 3].map(|k| base + k));
        }
        let positions: Vec<[f32; 3]> = vertices.iter().map(|v| v.position).collect();
        let mesh = loop {
            if let Some(mesh) = r.try_upload_mesh(&MeshData { vertices: &vertices, indices: &indices }).unwrap() {
                break mesh;
            }
            r.test_drain("rate mesh upload");
        };
        r.set_indirect_geometry(&mesh, &positions, &indices).unwrap();
        let instances = loop {
            if let Some(i) = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap() {
                break i;
            }
            r.test_drain("rate instance upload");
        };
        let m = material(&mut r, *albedo, *glow);
        meshes.push((mesh, instances, m));
    }
    // light-on looks at the slab's underside (lit by the emitter once it glows); mover at its top.
    let camera = if case == "light-on" {
        Camera::look_at(Vec3::new(0.0, -0.25, 0.0), Vec3::ZERO, Vec3::Z, FOV, 0.01, 100.0)
    } else {
        Camera::look_at(Vec3::new(0.0, 1.2, 0.0), Vec3::ZERO, Vec3::Z, FOV, 0.01, 100.0)
    }
    .unwrap();
    let mut frames = vec![];
    for frame in 1..=SETTLE + STRIP {
        let t = frame as i64 - SETTLE as i64;
        if case == "light-on" && t == 1 {
            std::mem::swap(&mut meshes[1].2, &mut lit);
        }
        if case == "mover" {
            // Over the slab's top at 0.25 m, from x = -1.2 to 1.2 over the strip (5 cm a frame).
            let x = (-1.2 + 0.05 * t.max(0) as f32).min(1.2);
            meshes[2].1.set(0, Mat4::from_translation(Vec3::new(x, 0.25, 0.0))).unwrap();
        }
        let mut draws: Vec<Draw<'_>> = meshes
            .iter_mut()
            .map(|(m, i, material)| Draw::new(m, i).with_material(material))
            .collect();
        let until = Instant::now() + Duration::from_secs(60);
        while r.try_render_view(&target, &camera, &mut draws, ViewMode::Lit).unwrap().is_none() {
            assert!(Instant::now() < until, "rate frame admission");
            r.test_drain("rate admission");
        }
        drop(draws);
        if t >= -2 {
            let pixels = capture_radiance(&mut r, 0.0);
            let lum: f64 = pixels
                .iter()
                .map(|p| f64::from(0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]))
                .sum::<f64>()
                / pixels.len() as f64;
            write_pfm(&dir.join(format!("frame-{t:+03}.pfm")), &pixels);
            // The external list's spare words (fg_rays, fg_shadow_rays): what a counting shader edit put there.
            let c = r.indirect_counters().unwrap().unwrap_or_default();
            frames.push(json!({"t": t, "mean": (lum * 1e4).round() / 1e4, "full_rate": r.gi.recently_changed(),
                "words": [c.fg_rays, c.fg_shadow_rays]}));
        }
    }
    json!(frames)
}
