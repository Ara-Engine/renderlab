//! N8 GPU checks (docs/non-rt-n8.md), ignored by default and run under an owned GPU lease
//! with RENDERLAB_PRIMARY_MANIFEST (`output`, optional `virtual` settings, and for the
//! quarry the `load_quarry` fields).
//!
//! - `gpu_virtual_fixture`: a bumpy grid floor, a box resting in it and a thin wire. N8
//!   counts are compared with hardware Interactive counts on the same eight directions,
//!   and the page cache is exercised (static, moved box, turned sun).
//! - `gpu_virtual_quarry`: N8 and Interactive counts at poses 0, 1 and the return, for
//!   the frozen Level B reference, plus static, camera-motion and sun-motion timing.
use super::*;

/// A `cells`-square grid of `size` metres with a height function; shared vertices give the
/// receiver adjacency real vertex fans.
fn grid(cells: u32, size: f32, height: impl Fn(f32, f32) -> f32) -> (Vec<[f32; 3]>, Vec<u32>) {
    let mut positions = vec![];
    for j in 0..=cells {
        for i in 0..=cells {
            let x = (i as f32 / cells as f32 - 0.5) * size;
            let z = (j as f32 / cells as f32 - 0.5) * size;
            positions.push([x, height(x, z), z]);
        }
    }
    let mut indices = vec![];
    let row = cells + 1;
    for j in 0..cells {
        for i in 0..cells {
            let a = j * row + i;
            indices.extend([a, a + row, a + 1, a + 1, a + row, a + row + 1]);
        }
    }
    (positions, indices)
}
/// An axis-aligned box from `lo` to `hi`, twelve triangles over eight shared corners.
fn cuboid(lo: [f32; 3], hi: [f32; 3]) -> (Vec<[f32; 3]>, Vec<u32>) {
    let positions = (0..8)
        .map(|c| {
            [
                if c & 1 == 0 { lo[0] } else { hi[0] },
                if c & 2 == 0 { lo[1] } else { hi[1] },
                if c & 4 == 0 { lo[2] } else { hi[2] },
            ]
        })
        .collect();
    let indices = vec![
        0, 2, 1, 1, 2, 3, 4, 5, 6, 5, 7, 6, 0, 1, 4, 1, 5, 4, 2, 6, 3, 3, 6, 7, 0, 4, 2, 2, 4, 6,
        1, 3, 5, 3, 7, 5,
    ];
    (positions, indices)
}

type Fixture = Vec<(Mesh, Instances, Material)>;
fn upload(r: &mut Renderer, meshes: &[(Vec<[f32; 3]>, Vec<u32>)]) -> Fixture {
    meshes
        .iter()
        .map(|(positions, indices)| {
            let vertices: Vec<_> = positions
                .iter()
                .map(|&position| Vertex {
                    position,
                    normal: [0., 1., 0.],
                    uv: [0.; 2],
                    color: [1.; 4],
                })
                .collect();
            let mesh = r
                .try_upload_mesh(&MeshData {
                    vertices: &vertices,
                    indices,
                })
                .unwrap()
                .unwrap();
            r.test_drain("N8 mesh");
            let instances = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap().unwrap();
            r.test_drain("N8 instances");
            let material = r
                .try_create_material(&MaterialDescriptor {
                    base_color: [0.5, 0.5, 0.5, 1.],
                    roughness: 0.6,
                    double_sided: true,
                    ..Default::default()
                })
                .unwrap()
                .unwrap();
            r.test_drain("N8 material");
            (mesh, instances, material)
        })
        .collect()
}
fn sun(r: &mut Renderer, to_light: Vec3) {
    r.set_lighting_and_shadows(
        &Lighting {
            lights: vec![Light::Directional {
                travel: -to_light.normalize(),
                lux: [1000.; 3],
            }],
            environment: [0.; 3],
            exposure_stops: -8.,
        },
        &Shadows {
            directional_disk_angle: 0.04,
            requests: vec![ShadowRequest::Directional {
                split_ends: vec![10., 40.],
            }],
            ..Default::default()
        },
    )
    .unwrap();
}
/// SHA-256 of an output (N8d identity checks between builds and variants).
pub(super) fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub(super) fn counts(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}
/// Per-ray agreement of N8 against hardware counts on the same directions, as a JSON row.
/// Counts are 32 per visible ray; only covered receivers are compared.
pub(super) fn agreement(n8: &[u32], rt: &[u32], hit: &[u8]) -> serde_json::Value {
    let (mut pixels, mut equal, mut sum, mut flips, mut lost, mut invented) =
        (0u64, 0, 0u64, 0, 0, 0);
    for ((&a, &b), h) in n8.iter().zip(rt).zip(hit.chunks_exact(16)) {
        if h[..4] == [0; 4] {
            continue;
        }
        pixels += 1;
        equal += u64::from(a == b);
        sum += u64::from(a.abs_diff(b));
        // A flip moves the pixel by at least half the disk.
        flips += u64::from(a.abs_diff(b) >= 128);
        lost += u64::from(b == 0 && a > 0);
        invented += u64::from(b == 256 && a < 256);
    }
    serde_json::json!({"covered":pixels,"equal":equal,"mean_abs_visibility":sum as f64/256./pixels.max(1) as f64,
        "flips_ge_half":flips,"umbra_with_light":lost,"lit_with_shadow":invented})
}
/// Rows of `rect` from a full image with `stride` bytes per pixel.
fn crop_rows(bytes: &[u8], width: u32, rect: [u32; 4], stride: usize) -> Vec<u8> {
    let mut out = vec![];
    for y in rect[1]..rect[3] {
        let row = (y * width) as usize * stride;
        out.extend_from_slice(
            &bytes[row + rect[0] as usize * stride..row + rect[2] as usize * stride],
        );
    }
    out
}
/// N8b diagnosis of the frame `draw` renders: Interactive counts, then N8 counts and the
/// `VSM_DEBUG` decision trace on the same receivers. Writes `{label}-interactive.u32`,
/// `{label}-n8.u32`, `{label}-trace.u32x4` and `{label}-hit.u32`, cropped to `rect`.
/// Leaves the renderer in `SoftShadows::Virtual` with the trace enabled, so a sequence
/// keeps N8's page cache history (an Interactive frame does not touch it).
pub(super) fn diagnose(
    r: &mut Renderer,
    out: &std::path::Path,
    label: &str,
    rect: Option<[u32; 4]>,
    draw: &mut dyn FnMut(&mut Renderer),
) {
    if r.virtual_shadows.as_ref().is_none_or(|s| !s.debug) {
        r.set_soft_shadows(SoftShadows::Virtual);
        draw(r);
        r.virtual_shadows.as_mut().unwrap().debug = true;
        // The manifest's screen constants, if any (variant traces).
        set_screen_constants(r, screen_constants(&manifest()));
    }
    r.set_soft_shadows(SoftShadows::Interactive);
    draw(r);
    let [hit, rt] = interactive_outputs(r);
    r.set_soft_shadows(SoftShadows::Virtual);
    draw(r);
    let [hit_n8, n8] = interactive_outputs(r);
    assert!(hit_n8 == hit, "N8 diagnosis traces the same camera hits");
    let state = r.virtual_shadows.as_ref().unwrap();
    let buffer = state.debug_buffer().unwrap().clone();
    let mut trace = read_buffer(r, &buffer, buffer.size());
    let width = r
        .fresh
        .as_ref()
        .unwrap()
        .targets
        .as_ref()
        .unwrap()
        .visibility
        .texture
        .width();
    let mut outputs = [hit, rt, n8];
    if let Some(rect) = rect {
        for (bytes, stride) in outputs.iter_mut().zip([16, 4, 4]) {
            *bytes = crop_rows(bytes, width, rect, stride);
        }
        trace = crop_rows(&trace, width, rect, 32);
    }
    for (name, bytes) in ["hit", "interactive", "n8"].iter().zip(&outputs) {
        fs::write(out.join(format!("{label}-{name}.u32")), bytes).unwrap();
    }
    fs::write(out.join(format!("{label}-trace.u32x4")), trace).unwrap();
}
/// Lookup override constants a variant screen may set (shaders/virtual_shadow.wgsl).
const SCREEN_CONSTANTS: [&str; 17] = [
    "VSM_SKIP",
    "VSM_CAP_RULE",
    "VSM_WALK_SHIFT",
    "VSM_LEAN",
    "VSM_LEAN_STEPS",
    "VSM_SHARE",
    "VSM_MIP",
    "VSM_HORIZON",
    "VSM_FAR",
    "VSM_ORIGIN_STEP",
    "VSM_SPAN",
    "VSM_CONTINUE",
    "VSM_PEEL",
    "VSM_ASCEND",
    "VSM_REFINE",
    "VSM_WALK_STEPS",
    "VSM_EXTRA_SAMPLES",
];
/// `{"constants": {name: number}}` as the state's override list.
pub(super) fn screen_constants(v: &serde_json::Value) -> Vec<(&'static str, f64)> {
    v["constants"].as_object().map_or_else(Vec::new, |object| {
        object
            .iter()
            .map(|(name, value)| {
                let name = SCREEN_CONSTANTS
                    .into_iter()
                    .find(|c| c == name)
                    .unwrap_or_else(|| panic!("unknown screen constant {name}"));
                (name, value.as_f64().unwrap())
            })
            .collect()
    })
}
/// Sets the lookup constants; the state must exist (after one Virtual frame).
pub(super) fn set_screen_constants(r: &mut Renderer, constants: Vec<(&'static str, f64)>) {
    r.virtual_shadows.as_mut().unwrap().constants = constants;
}
/// A traced renderer with the manifest's `virtual` settings, for the diagnosis tests.
pub(super) fn diagnosis_renderer(m: &serde_json::Value) -> Renderer {
    let mut r = renderer_with_features(false, crate::VIRTUAL_FEATURES);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    if let Some(settings) = virtual_settings(m) {
        r.set_virtual_shadow_settings(settings).unwrap();
    }
    r
}

/// N8b variant screen on quarry pose 0 (see motion_bench_tests::gpu_motion_virtual_screen):
/// counts against Interactive and shadow GPU times, static and with the sun turning.
#[test]
#[ignore = "N8b GPU lease, quarry upload; 4 GiB start/2 GiB abort; offscreen variant screen"]
fn gpu_virtual_quarry_screen() {
    let m = manifest();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let mut r = diagnosis_renderer(&m);
    let (mut asset, cameras) = load_quarry(&m, &mut r);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let color = target(&r, 1920, 1080);
    let camera = cameras.frames[0].camera().unwrap();
    let lighting =
        lighting_settings::parse(&fs::read_to_string(m["lighting"].as_str().unwrap()).unwrap())
            .unwrap();
    let shadows = r.shadows.settings.clone();
    let Light::Directional { travel, lux } = lighting.lights[0] else {
        panic!("quarry sun");
    };
    let turn = m["turn_degrees"].as_f64().unwrap_or(0.03) as f32;
    r.set_soft_shadows(SoftShadows::Interactive);
    traced_frame(&mut r, &color, &camera, &mut asset, true);
    let [hit, rt] = interactive_outputs(&r);
    if m["dump"].as_bool() == Some(true) {
        fs::write(out.join("hit.u32x4"), &hit).unwrap();
        fs::write(out.join("interactive.counts"), &rt).unwrap();
    }
    // Bring the GPU to its working clocks before the first variant's timing.
    for _ in 0..30 {
        traced_frame(&mut r, &color, &camera, &mut asset, true);
    }
    let ms = |r: &mut Renderer| -> Vec<f64> {
        r.test_drain("N8b quarry screen");
        r.poll()
            .unwrap()
            .gpu
            .iter()
            .filter_map(|s| s.visibility_compute.map(|d| d.as_secs_f64() * 1e3))
            .collect()
    };
    let median = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v.get(v.len() / 2).copied()
    };
    let mut rows = vec![];
    for (name, v) in m["variants"].as_object().unwrap() {
        let settings =
            virtual_settings(&serde_json::json!({ "virtual": v["virtual"] })).unwrap_or_default();
        r.set_virtual_shadow_settings(settings).unwrap();
        r.set_lighting_and_shadows(&lighting, &shadows).unwrap();
        r.set_soft_shadows(SoftShadows::Virtual);
        traced_frame(&mut r, &color, &camera, &mut asset, true);
        set_screen_constants(&mut r, screen_constants(v));
        for _ in 0..4 {
            traced_frame(&mut r, &color, &camera, &mut asset, true);
        }
        let [_, n8] = interactive_outputs(&r);
        let mut row = agreement(&counts(&n8), &counts(&rt), &hit);
        row["hash"] = serde_json::json!(sha256(&n8));
        if m["dump"].as_bool() == Some(true) {
            fs::write(out.join(format!("{name}.counts")), &n8).unwrap();
        }
        let _ = ms(&mut r);
        for _ in 0..24 {
            traced_frame(&mut r, &color, &camera, &mut asset, true);
        }
        let fixed = ms(&mut r);
        for i in 1..=16 {
            let mut lit = lighting.clone();
            lit.lights[0] = Light::Directional {
                travel: Mat4::from_rotation_y((turn * i as f32).to_radians())
                    .transform_vector3(travel),
                lux,
            };
            r.set_lighting_and_shadows(&lit, &shadows).unwrap();
            traced_frame(&mut r, &color, &camera, &mut asset, true);
        }
        let turning = ms(&mut r);
        let [_, turned] = interactive_outputs(&r);
        row["turned_hash"] = serde_json::json!(sha256(&turned));
        // N8e: the turned counts against Interactive at the turned sun (kept tiles lag it).
        r.set_soft_shadows(SoftShadows::Interactive);
        traced_frame(&mut r, &color, &camera, &mut asset, true);
        let [turned_hit, turned_rt] = interactive_outputs(&r);
        row["turned"] = agreement(&counts(&turned), &counts(&turned_rt), &turned_hit);
        let mut sorted = turning.clone();
        sorted.sort_by(f64::total_cmp);
        row["turning_shadow_ms_max"] = serde_json::json!(sorted.last());
        // N8e: the camera moving 1 mm a frame (sun fixed): every frame is a full lookup.
        r.set_lighting_and_shadows(&lighting, &shadows).unwrap();
        r.set_soft_shadows(SoftShadows::Virtual);
        traced_frame(&mut r, &color, &camera, &mut asset, true);
        let _ = ms(&mut r);
        let pose = &cameras.frames[0];
        for i in 1..=16 {
            let step = Vec3::X * (0.001 * i as f32);
            let moved = Camera::look_at(
                Vec3::from_array(pose.eye) + step,
                Vec3::from_array(pose.target) + step,
                Vec3::from_array(pose.up),
                pose.fov_y_radians,
                pose.near,
                pose.far,
            )
            .unwrap();
            traced_frame(&mut r, &color, &moved, &mut asset, true);
        }
        row["moving_shadow_ms_p50"] = serde_json::json!(median(ms(&mut r)));
        row["variant"] = serde_json::json!(name);
        row["static_shadow_ms_p50"] = serde_json::json!(median(fixed));
        row["turning_shadow_ms_p50"] = serde_json::json!(median(turning));
        row["stats"] = serde_json::json!(format!("{:?}", r.virtual_shadow_stats()));
        eprintln!("N8b quarry screen {row}");
        rows.push(row);
    }
    fs::write(
        out.join("screen-quarry.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "N8b GPU lease, quarry upload; 4 GiB start/2 GiB abort; offscreen decision trace"]
fn gpu_virtual_quarry_diagnose() {
    let m = manifest();
    let out = PathBuf::from(m["output"].as_str().unwrap()).join("quarry");
    fs::create_dir_all(&out).unwrap();
    let mut r = diagnosis_renderer(&m);
    let (mut asset, cameras) = load_quarry(&m, &mut r);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let color = target(&r, 1920, 1080);
    // Optional `variants`: {name: {settings}} screens on the one loaded asset; each writes
    // quarry-<name>/ beside the base quarry/ trace.
    let base = virtual_settings(&m).unwrap_or_default();
    let mut variants = vec![(out.clone(), base)];
    if let Some(list) = m["variants"].as_object() {
        for (name, v) in list {
            let s = virtual_settings(&serde_json::json!({ "virtual": v })).unwrap();
            let dir = out.with_file_name(format!("quarry-{name}"));
            fs::create_dir_all(&dir).unwrap();
            variants.push((dir, s));
        }
    }
    for (dir, settings) in variants {
        r.set_virtual_shadow_settings(settings).unwrap();
        for pose in [0usize, 1] {
            let camera = cameras.frames[pose].camera().unwrap();
            diagnose(&mut r, &dir, &format!("pose-{pose}"), None, &mut |r| {
                traced_frame(r, &color, &camera, &mut asset, true);
            });
            eprintln!(
                "N8b quarry {dir:?} pose {pose}: {:?}",
                r.virtual_shadow_stats()
            );
        }
    }
}

fn gpu_ms(r: &mut Renderer, frames: &mut Vec<serde_json::Value>, label: &str) {
    for s in r.poll().unwrap().gpu {
        let ms = |d: Duration| d.as_secs_f64() * 1e3;
        frames.push(
            serde_json::json!({"label":label,"frame":s.frame.0,"span_ms":ms(s.render_span),
            "receiver_ms":s.receiver_pass.map(ms),"shadow_ms":s.visibility_compute.map(ms)}),
        );
    }
}

#[test]
#[ignore = "N8 GPU lease, 4 GiB start/2 GiB abort; offscreen fixture screen"]
fn gpu_virtual_fixture() {
    let m = manifest();
    let out = PathBuf::from(m["output"].as_str().unwrap()).join("fixture");
    fs::create_dir_all(&out).unwrap();
    let mut r = renderer_with_features(false, crate::VIRTUAL_FEATURES);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    if let Some(settings) = virtual_settings(&m) {
        r.set_virtual_shadow_settings(settings).unwrap();
    }
    // 1 cm cells with 2-6 mm relief: ring contacts, map contacts and grazing self-shadow.
    let floor = grid(200, 2.0, |x, z| {
        0.004 * (x * 47.).sin() * (z * 31.).cos() + 0.002 * ((x + z) * 113.).sin()
    });
    let resting = cuboid([-0.3, -0.003, -0.2], [-0.1, 0.2, 0.0]);
    let wire = cuboid([-0.6, 0.08, 0.25], [0.6, 0.082, 0.252]);
    let mut scene = upload(&mut r, &[floor, resting, wire]);
    let color = target(&r, 960, 540);
    let camera = Camera::look_at(
        Vec3::new(0.2, 0.9, 1.4),
        Vec3::new(-0.1, 0., 0.),
        Vec3::Y,
        45f32.to_radians(),
        0.01,
        100.,
    )
    .unwrap();
    let frame = |r: &mut Renderer, scene: &mut Fixture| {
        let mut draws: Vec<_> = scene
            .iter_mut()
            .map(|(mesh, instances, material)| Draw::new(mesh, instances).with_material(material))
            .collect();
        render(r, &color, &camera, &mut draws)
    };
    let mut rows = vec![];
    let mut timing = vec![];
    for (label, to_light) in [
        ("sun-40deg", Vec3::new(0.5, 0.75, 0.4)),
        ("sun-15deg", Vec3::new(0.55, 0.26, 0.79)),
    ] {
        sun(&mut r, to_light);
        r.set_soft_shadows(SoftShadows::Interactive);
        frame(&mut r, &mut scene);
        let [hit, rt] = interactive_outputs(&r);
        r.set_soft_shadows(SoftShadows::Virtual);
        let first = frame(&mut r, &mut scene);
        assert_eq!(first.soft_shadows, SoftShadows::Virtual);
        let cold = r.virtual_shadow_stats();
        let [hit_n8, n8] = interactive_outputs(&r);
        assert!(hit_n8 == hit, "N8 frames trace the same camera hits");
        // A static repeat renders no page and must reproduce the counts.
        frame(&mut r, &mut scene);
        let warm = r.virtual_shadow_stats();
        let [_, again] = interactive_outputs(&r);
        assert!(again == n8, "static N8 repeat changed counts");
        assert_eq!(warm.invalidated_levels, 0, "static frame invalidated pages");
        assert!(!warm.page_work, "static frame ran page passes");
        fs::write(out.join(format!("{label}-n8.u32")), &n8).unwrap();
        fs::write(out.join(format!("{label}-interactive.u32")), &rt).unwrap();
        fs::write(out.join(format!("{label}-hit.u32")), &hit).unwrap();
        let mut row = agreement(&counts(&n8), &counts(&rt), &hit);
        row["case"] = serde_json::json!(label);
        row["cold"] = serde_json::json!(format!("{cold:?}"));
        row["warm"] = serde_json::json!(format!("{warm:?}"));
        eprintln!("N8 fixture {row}");
        rows.push(row);
        // Moving the resting box re-rasterizes only the pages under it.
        scene[1]
            .1
            .set(0, Mat4::from_translation(Vec3::new(0.05, 0., 0.)))
            .unwrap();
        frame(&mut r, &mut scene);
        let moved = r.virtual_shadow_stats();
        assert_eq!(moved.moved_rects, 1);
        r.set_soft_shadows(SoftShadows::Interactive);
        frame(&mut r, &mut scene);
        let [hit, rt] = interactive_outputs(&r);
        r.set_soft_shadows(SoftShadows::Virtual);
        frame(&mut r, &mut scene);
        let [_, n8] = interactive_outputs(&r);
        let mut row = agreement(&counts(&n8), &counts(&rt), &hit);
        row["case"] = serde_json::json!(format!("{label}-moved-box"));
        row["moved"] = serde_json::json!(format!("{moved:?}"));
        rows.push(row);
        scene[1].1.set(0, Mat4::IDENTITY).unwrap();
        // Timing: warm cache, then a sun turning every frame (all pages stale).
        for _ in 0..8 {
            frame(&mut r, &mut scene);
        }
        r.test_drain("N8 fixture warm");
        r.poll().unwrap();
        for _ in 0..16 {
            frame(&mut r, &mut scene);
        }
        r.test_drain("N8 fixture static");
        gpu_ms(&mut r, &mut timing, &format!("{label}-n8-static"));
        for i in 0..16 {
            let turn = Mat4::from_rotation_y(0.002 * (i + 1) as f32);
            sun(&mut r, turn.transform_vector3(to_light));
            frame(&mut r, &mut scene);
        }
        r.test_drain("N8 fixture sun motion");
        gpu_ms(&mut r, &mut timing, &format!("{label}-n8-sun-motion"));
        sun(&mut r, to_light);
        r.set_soft_shadows(SoftShadows::Interactive);
        for _ in 0..8 {
            frame(&mut r, &mut scene);
        }
        r.test_drain("N8 fixture interactive warm");
        r.poll().unwrap();
        for _ in 0..16 {
            frame(&mut r, &mut scene);
        }
        r.test_drain("N8 fixture interactive");
        gpu_ms(&mut r, &mut timing, &format!("{label}-interactive"));
    }
    fs::write(
        out.join("fixture.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"rows":rows,"timing":timing,
            "settings":format!("{:?}", virtual_settings(&m).unwrap_or_default())}))
        .unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "N8 GPU lease, quarry upload; 4 GiB start/2 GiB abort; offscreen"]
fn gpu_virtual_quarry() {
    let m = manifest();
    let out = PathBuf::from(m["output"].as_str().unwrap()).join("quarry");
    fs::create_dir_all(&out).unwrap();
    let mut r = renderer_with_features(false, crate::VIRTUAL_FEATURES);
    if let Some(settings) = virtual_settings(&m) {
        r.set_virtual_shadow_settings(settings).unwrap();
    }
    let (mut asset, cameras) = load_quarry(&m, &mut r);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let color = target(&r, 1920, 1080);
    let poses = [0, 1].map(|i| cameras.frames[i].camera().unwrap());
    let mut stats = vec![];
    let mut timing = vec![];
    let lighting =
        lighting_settings::parse(&fs::read_to_string(m["lighting"].as_str().unwrap()).unwrap())
            .unwrap();
    let shadows = r.shadows.settings.clone();
    let Light::Directional { travel, lux } = lighting.lights[0] else {
        panic!("quarry sun");
    };
    let base = virtual_settings(&m).unwrap_or_default();
    // A reference on today's traced receivers, as gpu_primary_traced_quarry_acceptance
    // builds it: the mean of 256-ray Exact seeds 1-64 (16,384 rays), its per-seed sigma,
    // coverage and depth. The frozen Level B reference predates the current receivers.
    let module = seeded_module(&r);
    let seeded: Vec<_> = (1..=64u32)
        .map(|s| {
            r.primary.as_ref().unwrap().fresh_pipeline(
                &r.device,
                &r.ledger,
                &module,
                "fresh_compute",
                &[("FRESH_SEED", f64::from(s))],
            )
        })
        .collect();
    r.set_soft_shadows(SoftShadows::Exact);
    // A timing-only rerun can skip it (manifest "reference": false), and a rerun into the
    // same output reuses the one already built there (N8b).
    let built = (0..2).all(|i| out.join(format!("pose-{i}-traced-reference.bin")).is_file());
    let poses_with_reference = if m["reference"].as_bool() == Some(false) || built {
        0
    } else {
        2
    };
    for (index, pose) in [0usize, 1]
        .into_iter()
        .enumerate()
        .take(poses_with_reference)
    {
        traced_frame(&mut r, &color, &poses[pose], &mut asset, true);
        let [world, _, _] = receivers(&r);
        let hit = hit_record(&r);
        let pixels = world.len() / 16;
        let (mut sum, mut square) = (vec![0f64; pixels], vec![0f64; pixels]);
        for pipeline in &seeded {
            let mut encoder = r.device.create_command_encoder(&Default::default());
            r.encode_primary_fresh(&mut encoder, pipeline, None);
            r.queue.submit([encoder.finish()]);
            for (i, c) in receivers(&r)[2].chunks_exact(4).enumerate() {
                let v = f64::from(u32::from_le_bytes(c.try_into().unwrap())) / 256.;
                sum[i] += v;
                square[i] += v * v;
            }
        }
        let mut bytes = Vec::with_capacity(pixels * 13);
        for i in 0..pixels {
            let mean = sum[i] / 64.;
            let sigma = (square[i] / 64. - mean * mean).max(0.).sqrt();
            bytes.extend_from_slice(&(mean as f32).to_le_bytes());
            bytes.extend_from_slice(&(sigma as f32).to_le_bytes());
            bytes.extend_from_slice(&world[i * 16 + 12..i * 16 + 16]);
        }
        bytes.extend(hit.chunks_exact(16).map(|h| u8::from(h[..4] != [0; 4])));
        fs::write(
            out.join(format!("pose-{index}-traced-reference.bin")),
            bytes,
        )
        .unwrap();
    }
    // Interactive (hardware rays) is the same-direction control; N8 runs with its full
    // vertex 1-ring and with the receiver triangle alone.
    let selected = |name: &str| {
        m["modes"]
            .as_array()
            .is_none_or(|names| names.iter().any(|n| n.as_str() == Some(name)))
    };
    for (name, mode, ring) in [
        ("interactive", SoftShadows::Interactive, true),
        ("n8", SoftShadows::Virtual, true),
        ("n8-ring0", SoftShadows::Virtual, false),
    ] {
        if !selected(name) {
            continue;
        }
        r.set_virtual_shadow_settings(crate::VirtualShadowSettings { ring, ..base })
            .unwrap();
        r.set_soft_shadows(mode);
        for (index, pose) in [0usize, 1, 0].into_iter().enumerate() {
            let report = traced_frame(&mut r, &color, &poses[pose], &mut asset, true);
            assert_eq!(report.soft_shadows, mode);
            let [_, counts] = interactive_outputs(&r);
            fs::write(out.join(format!("pose-{index}-{name}.u32")), &counts).unwrap();
            if mode == SoftShadows::Virtual {
                stats.push(format!(
                    "{name} pose {index}: {:?}",
                    r.virtual_shadow_stats()
                ));
            }
        }
        for motion in ["static", "camera-alternate", "sun-motion"] {
            if mode == SoftShadows::Interactive && motion == "sun-motion" {
                continue;
            }
            let label = format!("{name}-{motion}");
            for _ in 0..8 {
                traced_frame(&mut r, &color, &poses[0], &mut asset, true);
            }
            r.test_drain("N8 quarry warm");
            r.poll().unwrap();
            let mut page_frames = 0;
            for i in 0..32usize {
                let pose = if motion == "camera-alternate" {
                    i % 2
                } else {
                    0
                };
                if motion == "sun-motion" {
                    let turn = Mat4::from_rotation_y(0.0005 * (i + 1) as f32);
                    let mut lit = lighting.clone();
                    lit.lights[0] = Light::Directional {
                        travel: turn.transform_vector3(travel),
                        lux,
                    };
                    r.set_lighting_and_shadows(&lit, &shadows).unwrap();
                }
                traced_frame(&mut r, &color, &poses[pose], &mut asset, true);
                page_frames += u32::from(r.virtual_shadow_stats().page_work);
                gpu_ms(&mut r, &mut timing, &label);
            }
            r.test_drain("N8 quarry timing");
            gpu_ms(&mut r, &mut timing, &label);
            stats.push(format!("{label}: frames with page work {page_frames}/32"));
            if motion == "sun-motion" {
                r.set_lighting_and_shadows(&lighting, &shadows).unwrap();
            }
        }
    }
    fs::write(
        out.join("quarry.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"stats":stats,"timing":timing,
            "settings":format!("{:?}", virtual_settings(&m).unwrap_or_default()),
            "scope":"traced primaries; shadow_ms is N8's page update plus lookup (Interactive: none, fused in receiver_ms)"}))
        .unwrap(),
    )
    .unwrap();
}
