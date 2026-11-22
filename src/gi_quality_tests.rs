//! Proof-grade GI & RT quality, parity, corner-AO, and 1080p stability test harness.
//!
//! Scores Renderlab's RT and Non-RT paths directly against the built-in unbiased GPU path tracer
//! (`crate::reference::render`, `shaders/reference_path_tracer.wgsl`) across five diagnostic
//! scenarios designed to expose the bugs that small uniform-box furnace tests miss:
//!
//! 1. `cornell_box`: classic Cornell Box (red/green walls, tall/short boxes, shadowed point light)
//!    verifying bounce energy, directional color bleed, and RT vs Non-RT parity against ground truth.
//! 2. `inset_corner_ao`: concave 90-degree wall-wall-floor corner under indirect GI verifying crisp
//!    sub-probe contact darkening into the crease against ground truth.
//! 3. `textured_bounce`: split red/blue UV-mapped wall bouncing onto a shaded white receiver,
//!    exposing Non-RT cell shading UV collapse (`GI_NO_PRIMITIVE` 1x1 mip fallback).
//! 4. `glass_sunlit_room`: interior lit through a transparent glass window, exposing see-through
//!    GI transmission starvation on Non-RT.
//! 5. `hd_1080p_convergence_and_stability`: full 1920x1080 stress test (blazing sunlit red wall
//!    facing a shaded neutral arch/ceiling + open sky) checking 30-frame cache convergence
//!    (`claim_deferred`, `external_requests`, `gather_misses`), RT vs Non-RT color parity, and
//!    static frame-to-frame temporal stability.
//!
//! Run:
//!   RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK=I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK \
//!   RENDERLAB_GI_QUALITY_OUT=<optional dir> \
//!   cargo test --lib gi_quality_suite -- --ignored --nocapture
use super::*;
use crate::asset::{BaseColorTexture, Mip, Sampler, Texture as AssetTexture, TextureRole};
use crate::reference;

const TEST_SIZE: [u32; 2] = [384, 216];
const HD_SIZE: [u32; 2] = [1920, 1080];
const REF_SPP: u32 = 256;

struct PartSpec {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    base_color: [f32; 4],
    roughness: f32,
    double_sided: bool,
    transparency: Option<Transparency>,
    texture: Option<usize>,
    casts_shadows: bool,
}

struct SceneSpec {
    name: &'static str,
    size: [u32; 2],
    eye: Vec3,
    target: Vec3,
    up: Vec3,
    fov_y: f32,
    sky: [f32; 3],
    sun: Option<(Vec3, [f32; 3])>, // (to_light, lux)
    point_light: Option<(Vec3, [f32; 3], f32)>, // (pos, candela, range)
    exposure: f32,
    textures: Vec<AssetTexture>,
    parts: Vec<PartSpec>,
}

fn push_quad(
    verts: &mut Vec<Vertex>,
    idx: &mut Vec<u32>,
    centre: Vec3,
    normal: Vec3,
    u_vec: Vec3,
    half_v: f32,
    uv_range: [[f32; 2]; 2],
) {
    let n = normal.normalize();
    let v_vec = n.cross(u_vec).normalize() * half_v;
    let corners = [
        (centre - u_vec - v_vec, [uv_range[0][0], uv_range[0][1]]),
        (centre + u_vec - v_vec, [uv_range[1][0], uv_range[0][1]]),
        (centre + u_vec + v_vec, [uv_range[1][0], uv_range[1][1]]),
        (centre - u_vec + v_vec, [uv_range[0][0], uv_range[1][1]]),
    ];
    let base = verts.len() as u32;
    for (p, uv) in corners {
        verts.push(Vertex {
            position: p.to_array(),
            normal: n.to_array(),
            uv,
            color: [1.0; 4],
        });
    }
    idx.extend([0, 1, 2, 0, 2, 3].map(|k| base + k));
}

fn solid_part(
    quads: Vec<(Vec3, Vec3, Vec3, f32)>,
    base_color: [f32; 3],
    roughness: f32,
) -> PartSpec {
    let mut vertices = vec![];
    let mut indices = vec![];
    for (c, n, u, hv) in quads {
        push_quad(&mut vertices, &mut indices, c, n, u, hv, [[0.0, 0.0], [1.0, 1.0]]);
    }
    PartSpec {
        vertices,
        indices,
        base_color: [base_color[0], base_color[1], base_color[2], 1.0],
        roughness,
        double_sided: true,
        transparency: None,
        texture: None,
        casts_shadows: true,
    }
}

fn box_part(centre: Vec3, half: Vec3, base_color: [f32; 3]) -> PartSpec {
    let quads = vec![
        (centre + Vec3::X * half.x, Vec3::X, Vec3::Z * half.z, half.y),
        (centre - Vec3::X * half.x, -Vec3::X, -Vec3::Z * half.z, half.y),
        (centre + Vec3::Y * half.y, Vec3::Y, Vec3::X * half.x, half.z),
        (centre - Vec3::Y * half.y, -Vec3::Y, Vec3::X * half.x, half.z),
        (centre + Vec3::Z * half.z, Vec3::Z, -Vec3::X * half.x, half.y),
        (centre - Vec3::Z * half.z, -Vec3::Z, Vec3::X * half.x, half.y),
    ];
    solid_part(quads, base_color, 1.0)
}

fn split_red_blue_texture(side: u32) -> AssetTexture {
    let mut mips = vec![];
    let mut w = side;
    loop {
        let mut rgba8 = Vec::with_capacity((w * w * 4) as usize);
        for _y in 0..w {
            for x in 0..w {
                if w == 1 {
                    // Exact average of red and blue halves: purple!
                    rgba8.extend_from_slice(&[130, 20, 130, 255]);
                } else if x < w / 2 {
                    // Left half (u < 0.5): saturated red.
                    rgba8.extend_from_slice(&[240, 20, 20, 255]);
                } else {
                    // Right half (u >= 0.5): saturated blue.
                    rgba8.extend_from_slice(&[20, 20, 240, 255]);
                }
            }
        }
        mips.push(Mip { width: w, height: w, rgba8 });
        if w == 1 {
            break;
        }
        w /= 2;
    }
    AssetTexture { role: TextureRole::BaseColorSrgb, mips }
}

fn cornell_box_scene() -> SceneSpec {
    let white = [0.72, 0.72, 0.72];
    let red = [0.78, 0.08, 0.06];
    let green = [0.08, 0.68, 0.10];
    SceneSpec {
        name: "cornell_box",
        size: TEST_SIZE,
        eye: Vec3::new(0.0, 1.9, 1.85),
        target: Vec3::new(0.0, 1.65, -2.0),
        up: Vec3::Y,
        fov_y: 1.05,
        sky: [0.0; 3],
        sun: None,
        point_light: Some((Vec3::new(0.0, 3.65, -0.35), [320.0, 305.0, 285.0], 12.0)),
        exposure: -4.2,
        textures: vec![],
        parts: vec![
            // Floor, ceiling, back wall, front wall behind camera
            solid_part(
                vec![
                    (Vec3::new(0.0, 0.0, 0.0), Vec3::Y, Vec3::X * 2.0, 2.0),
                    (Vec3::new(0.0, 4.0, 0.0), -Vec3::Y, Vec3::X * 2.0, 2.0),
                    (Vec3::new(0.0, 2.0, -2.0), Vec3::Z, Vec3::X * 2.0, 2.0),
                    (Vec3::new(0.0, 2.0, 2.0), -Vec3::Z, -Vec3::X * 2.0, 2.0),
                ],
                white,
                1.0,
            ),
            // Left red wall
            solid_part(
                vec![(Vec3::new(-2.0, 2.0, 0.0), Vec3::X, Vec3::Z * 2.0, 2.0)],
                red,
                1.0,
            ),
            // Right green wall
            solid_part(
                vec![(Vec3::new(2.0, 2.0, 0.0), -Vec3::X, -Vec3::Z * 2.0, 2.0)],
                green,
                1.0,
            ),
            // Tall box on left (facing red wall) & short box on right (facing green wall)
            box_part(Vec3::new(-0.72, 1.0, -0.85), Vec3::new(0.45, 1.0, 0.45), [0.75; 3]),
            box_part(Vec3::new(0.68, 0.52, -0.35), Vec3::new(0.45, 0.52, 0.45), [0.75; 3]),
        ],
    }
}

fn inset_corner_ao_scene() -> SceneSpec {
    // Deep concave 90-degree corner (walls at x=0 and z=0, floor at y=0, roof overhang at y=2.6)
    // open to sky on +X/+Z with an angled sun hitting the outer floor while the corner walls
    // themselves are shaded and lit purely by GI + sky occlusion.
    let plaster = [0.68, 0.68, 0.68];
    SceneSpec {
        name: "inset_corner_ao",
        size: TEST_SIZE,
        eye: Vec3::new(2.3, 1.3, 2.3),
        target: Vec3::new(0.0, 1.1, 0.0),
        up: Vec3::Y,
        fov_y: 0.95,
        sky: [1.0, 1.0, 1.0],
        sun: Some((Vec3::new(-0.45, 0.78, -0.43).normalize(), [8.0, 7.6, 7.0])),
        point_light: None,
        exposure: -1.4,
        textures: vec![],
        parts: vec![solid_part(
            vec![
                // Floor
                (Vec3::new(2.5, 0.0, 2.5), Vec3::Y, Vec3::X * 2.5, 2.5),
                // Left wall (x = 0, facing +X)
                (Vec3::new(0.0, 1.3, 2.5), Vec3::X, Vec3::Z * 2.5, 1.3),
                // Right wall (z = 0, facing +Z)
                (Vec3::new(2.5, 1.3, 0.0), Vec3::Z, Vec3::X * 2.5, 1.3),
                // Roof overhang over the corner so sky enters from the open +X/+Z sides
                (Vec3::new(1.4, 2.6, 1.4), -Vec3::Y, Vec3::X * 1.4, 1.4),
            ],
            plaster,
            1.0,
        )],
    }
}

fn textured_bounce_scene() -> SceneSpec {
    // One single quad mesh at x = -1.6 spanning z in [-3, 3] with a split red (z<0) / blue (z>0)
    // texture, lit by bright sun. A matte white receiver wall at x = 0 facing -X sits strictly
    // beside the red half (z in [-2.6, -0.4]), shielded by a roof & partition from the sky and
    // from the blue half. With exact per-triangle UVs, the white wall receives pure RED bounce;
    // with GI_NO_PRIMITIVE (1x1 mip average), it receives PURPLE bounce.
    let mut wall_v = vec![];
    let mut wall_i = vec![];
    // u runs along +Z from -3 (u=0, red) to +3 (u=1, blue).
    push_quad(
        &mut wall_v,
        &mut wall_i,
        Vec3::new(-1.6, 1.5, 0.0),
        Vec3::X,
        Vec3::Z * 3.0,
        1.5,
        [[0.0, 0.0], [1.0, 1.0]],
    );
    let textured_wall = PartSpec {
        vertices: wall_v,
        indices: wall_i,
        base_color: [1.0, 1.0, 1.0, 1.0],
        roughness: 1.0,
        double_sided: true,
        transparency: None,
        texture: Some(0),
        casts_shadows: true,
    };
    SceneSpec {
        name: "textured_bounce",
        size: TEST_SIZE,
        eye: Vec3::new(-1.25, 1.25, -2.55),
        target: Vec3::new(0.0, 1.25, -1.5),
        up: Vec3::Y,
        fov_y: 0.9,
        sky: [0.05, 0.05, 0.05],
        sun: Some((Vec3::new(0.85, 0.52, 0.0).normalize(), [40.0, 40.0, 40.0])),
        point_light: None,
        exposure: -1.8,
        textures: vec![split_red_blue_texture(64)],
        parts: vec![
            textured_wall,
            // White receiver wall facing -X (toward the red half of the textured wall)
            solid_part(
                vec![(Vec3::new(0.0, 1.5, -1.5), -Vec3::X, -Vec3::Z * 1.4, 1.5)],
                [0.85, 0.85, 0.85],
                1.0,
            ),
            // Floor & dark divider at z = -0.05 so the receiver cannot see the blue half
            solid_part(
                vec![
                    (Vec3::new(-0.8, 0.0, -1.5), Vec3::Y, Vec3::X * 1.2, 1.6),
                    (Vec3::new(-0.8, 1.5, -0.05), -Vec3::Z, Vec3::X * 0.8, 1.5),
                    // Overhang above receiver shielding it from direct sun
                    (Vec3::new(-0.25, 3.0, -1.5), -Vec3::Y, Vec3::X * 0.45, 1.6),
                ],
                [0.2, 0.2, 0.2],
                1.0,
            ),
        ],
    }
}

fn glass_sunlit_room_scene() -> SceneSpec {
    // Closed room except for a large transparent glass window on the +X wall facing bright sun &
    // sky. Verifies Non-RT GI transmits sky + sun through glass cleanly instead of starving dark.
    let mut glass_v = vec![];
    let mut glass_i = vec![];
    push_quad(
        &mut glass_v,
        &mut glass_i,
        Vec3::new(2.0, 1.5, 0.0),
        -Vec3::X,
        Vec3::Z * 1.85,
        1.35,
        [[0.0, 0.0], [1.0, 1.0]],
    );
    let glass = PartSpec {
        vertices: glass_v,
        indices: glass_i,
        base_color: [0.95, 0.95, 0.95, 0.05],
        roughness: 0.05,
        double_sided: true,
        transparency: Some(Transparency { opacity: 0.05, ior: 1.5, thickness: 0.0 }),
        texture: None,
        casts_shadows: false,
    };
    SceneSpec {
        name: "glass_sunlit_room",
        size: TEST_SIZE,
        eye: Vec3::new(0.8, 1.5, 1.7),
        target: Vec3::new(-1.8, 1.4, -1.5),
        up: Vec3::Y,
        fov_y: 1.0,
        sky: [1.5, 1.6, 1.8],
        sun: Some((Vec3::new(0.88, 0.45, 0.12).normalize(), [45.0, 42.0, 36.0])),
        point_light: None,
        exposure: -2.2,
        textures: vec![],
        parts: vec![
            // Room shell (floor, ceiling, -X back wall, -Z wall, +Z wall)
            solid_part(
                vec![
                    (Vec3::new(0.0, 0.0, 0.0), Vec3::Y, Vec3::X * 2.0, 2.0),
                    (Vec3::new(0.0, 3.0, 0.0), -Vec3::Y, Vec3::X * 2.0, 2.0),
                    (Vec3::new(-2.0, 1.5, 0.0), Vec3::X, Vec3::Z * 2.0, 1.5),
                    (Vec3::new(0.0, 1.5, -2.0), Vec3::Z, Vec3::X * 2.0, 1.5),
                    (Vec3::new(0.0, 1.5, 2.0), -Vec3::Z, -Vec3::X * 2.0, 1.5),
                ],
                [0.72, 0.72, 0.72],
                1.0,
            ),
            // Interior divider box to create shaded bounce-only regions
            box_part(Vec3::new(-0.4, 0.7, -0.5), Vec3::new(0.45, 0.7, 0.45), [0.7, 0.68, 0.64]),
            glass,
        ],
    }
}

fn hd_sponza_stress_scene() -> SceneSpec {
    // Full 1920x1080 Sponza-like courtyard: open sky overhead, blazing sunlit red banner on the
    // left, shaded neutral stone archway ceiling overhead, shaded green banner on the right, and
    // warm sunlit stone floor. Designed to reproduce the exact 1080p cache-starvation & red-arch
    // runaway bug we caught on Sponza.
    SceneSpec {
        name: "hd_1080p_stress",
        size: HD_SIZE,
        eye: Vec3::new(0.0, 2.0, 4.8),
        target: Vec3::new(0.0, 2.3, -4.0),
        up: Vec3::Y,
        fov_y: 1.0,
        sky: [0.85, 0.95, 1.15],
        sun: Some((Vec3::new(0.55, 0.82, 0.15).normalize(), [42.0, 39.0, 34.0])),
        point_light: None,
        exposure: -2.4,
        textures: vec![],
        parts: vec![
            // Stone courtyard floor & back wall & shaded arch ceiling overhead
            solid_part(
                vec![
                    (Vec3::new(0.0, 0.0, 0.0), Vec3::Y, Vec3::X * 4.0, 6.0),
                    (Vec3::new(0.0, 3.0, -5.5), Vec3::Z, Vec3::X * 4.0, 3.0),
                    // Foreground overhead arch/ceiling slab (camera looks under it, as in Sponza)
                    (Vec3::new(0.0, 4.2, 2.6), -Vec3::Y, Vec3::X * 3.8, 2.6),
                    // Left & right colonnade outer walls
                    (Vec3::new(-3.6, 2.5, 0.0), Vec3::X, Vec3::Z * 6.0, 2.5),
                    (Vec3::new(3.6, 2.5, 0.0), -Vec3::X, -Vec3::Z * 6.0, 2.5),
                ],
                [0.64, 0.61, 0.56],
                1.0,
            ),
            // Blazing sunlit red curtain on the left (directly hit by sun!)
            solid_part(
                vec![(Vec3::new(-2.4, 1.8, 0.6), Vec3::X, Vec3::Z * 1.4, 1.5)],
                [0.78, 0.08, 0.08],
                1.0,
            ),
            // Shaded green curtain on the right
            solid_part(
                vec![(Vec3::new(2.4, 1.8, 0.6), -Vec3::X, -Vec3::Z * 1.4, 1.5)],
                [0.09, 0.56, 0.16],
                1.0,
            ),
            // Two flanking stone columns
            box_part(Vec3::new(-2.5, 2.1, 2.4), Vec3::new(0.35, 2.1, 0.35), [0.64, 0.61, 0.56]),
            box_part(Vec3::new(2.5, 2.1, 2.4), Vec3::new(0.35, 2.1, 0.35), [0.64, 0.61, 0.56]),
        ],
    }
}

fn render_reference(spec: &SceneSpec, spp: u32) -> Vec<[f32; 3]> {
    let (device, queue) = rt_device();
    let mut ref_scene = reference::Scene {
        sky: spec.sky,
        textures: spec.textures.clone(),
        ..Default::default()
    };
    if let Some((to_light, lux)) = spec.sun {
        ref_scene.sun = Some(reference::Sun {
            to_light: to_light.normalize(),
            illuminance: lux,
            angular_radius: 0.00465,
        });
    }
    if let Some((pos, candela, range)) = spec.point_light {
        ref_scene.lights.push(reference::LocalLight {
            position: pos,
            candela,
            range,
            spot: None,
        });
    }
    for part in &spec.parts {
        if part.transparency.is_some() {
            continue;
        }
        let mesh_id = ref_scene.meshes.len();
        ref_scene.meshes.push(reference::Mesh {
            positions: part.vertices.iter().map(|v| v.position).collect(),
            normals: part.vertices.iter().map(|v| v.normal).collect(),
            uvs: part.vertices.iter().map(|v| v.uv).collect(),
            indices: part.indices.clone(),
            ..Default::default()
        });
        let mat_id = ref_scene.materials.len();
        ref_scene.materials.push(reference::Material {
            base_color: [part.base_color[0], part.base_color[1], part.base_color[2]],
            metallic: 0.0,
            roughness: part.roughness,
            specular: 1.0,
            emission: [0.0; 3],
            base_color_texture: part.texture.map(|t| BaseColorTexture {
                texture: t as u32,
                sampler: Sampler::default(),
            }),
            metallic_roughness_texture: None,
            normal_texture: None,
        });
        ref_scene.instances.push(reference::Instance {
            casts_shadows: part.casts_shadows,
            mesh: mesh_id,
            material: mat_id,
            transform: Mat4::IDENTITY,
        });
    }
    let view = reference::View {
        open: reference::Pose { eye: spec.eye, target: spec.target },
        close: reference::Pose { eye: spec.eye, target: spec.target },
        up: spec.up,
        fov_y: spec.fov_y,
        width: spec.size[0],
        height: spec.size[1],
    };
    let settings = reference::Settings {
        samples: spp,
        samples_per_dispatch: 32,
        max_depth: 8,
        ..Default::default()
    };
    let img = reference::render(&device, &queue, &ref_scene, &view, &settings).unwrap();
    assert_eq!(img.invalid_pixels, 0, "{}: reference had invalid pixels", spec.name);
    img.mean
}

struct RunOutput {
    radiance: Vec<[f32; 3]>,
    tail_frames: Vec<Vec<[f32; 3]>>,
    counters: IndirectCounters,
}

fn render_renderlab(spec: &SceneSpec, rt: bool, frames: u32, capture_tail: u32) -> RunOutput {
    let (device, queue) = if rt { rt_device() } else { no_rt_device_with(true) };
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
        Renderer::new_with_options(device, queue, FORMAT, Limits::default(), options, SOURCE)
            .unwrap()
    };
    let mut lights = vec![];
    let mut requests = vec![];
    if let Some((to_light, lux)) = spec.sun {
        lights.push(Light::Directional {
            travel: -to_light.normalize(),
            lux,
        });
        requests.push(ShadowRequest::Directional {
            split_ends: vec![4.0, 12.0, 40.0],
        });
    }
    if let Some((position, candela, range)) = spec.point_light {
        lights.push(Light::Point {
            position,
            candela,
            range: Some(range),
            source_radius: 0.0,
            cast_shadows: Some(true),
        });
        requests.push(ShadowRequest::None);
    }
    let shadows = if spec.sun.is_some() {
        Shadows {
            requests,
            directional_disk_angle: 0.00465,
            ..Shadows::default()
        }
    } else {
        Shadows::default()
    };
    r.set_lighting_and_shadows(
        &Lighting {
            lights,
            environment: spec.sky,
            exposure_stops: spec.exposure,
        },
        &shadows,
    )
    .unwrap();
    if rt && spec.sun.is_some() {
        r.set_soft_shadows(SoftShadows::Interactive);
    }
    r.set_reconstruction(Reconstruction::off()).unwrap();
    r.set_indirect(IndirectSettings::default()).unwrap();
    let target = color_target(&r, spec.size);

    let mut textures = vec![];
    for tex in &spec.textures {
        let uploaded = loop {
            if let Some(t) = r.try_upload_texture(tex).unwrap() {
                break t;
            }
            r.test_drain("gi quality texture upload");
        };
        textures.push(uploaded);
    }

    let mut draws_owned = vec![];
    for part in &spec.parts {
        let positions: Vec<[f32; 3]> = part.vertices.iter().map(|v| v.position).collect();
        let mesh = loop {
            if let Some(m) = r
                .try_upload_mesh(&MeshData {
                    vertices: &part.vertices,
                    indices: &part.indices,
                })
                .unwrap()
            {
                break m;
            }
            r.test_drain("gi quality mesh upload");
        };
        r.set_indirect_geometry(&mesh, &positions, &part.indices).unwrap();
        let mut instances = loop {
            if let Some(i) = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap() {
                break i;
            }
            r.test_drain("gi quality instances upload");
        };
        if !part.casts_shadows {
            instances.set_casts_shadows(0, false).unwrap();
        }
        let binding = part.texture.map(|ti| TextureBinding {
            texture: &textures[ti],
            sampler: Sampler::default(),
        });
        let material = loop {
            let desc = MaterialDescriptor {
                base_color: part.base_color,
                metallic: 0.0,
                roughness: part.roughness,
                double_sided: part.double_sided,
                transparency: part.transparency,
                base_color_texture: binding,
                ..Default::default()
            };
            if let Some(m) = r.try_create_material(&desc).unwrap() {
                break m;
            }
            r.test_drain("gi quality material upload");
        };
        draws_owned.push((mesh, instances, material));
    }

    let camera = Camera::look_at(spec.eye, spec.target, spec.up, spec.fov_y, 0.05, 100.0).unwrap();
    let mut tail_frames = vec![];
    let mut last = vec![];
    for frame in 1..=frames {
        let mut draws: Vec<Draw<'_>> = draws_owned
            .iter_mut()
            .map(|(m, i, mat)| Draw::new(m, i).with_material(mat))
            .collect();
        let until = Instant::now() + Duration::from_secs(60);
        while r
            .try_render_view(&target, &camera, &mut draws, ViewMode::Lit)
            .unwrap()
            .is_none()
        {
            assert!(Instant::now() < until, "{} frame admission", spec.name);
            r.test_drain("gi quality frame admission");
        }
        drop(draws);
        if frame + capture_tail > frames {
            let img = capture_radiance(&mut r, spec.exposure);
            if frame == frames {
                last = img.clone();
            }
            tail_frames.push(img);
        }
    }
    let counters = r.indirect_counters().unwrap().unwrap_or_default();
    RunOutput { radiance: last, tail_frames, counters }
}

fn lum(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn tonemap_u8(c: [f32; 3], exposure: f32) -> [f32; 3] {
    let s = exposure.exp2();
    std::array::from_fn(|k| {
        let x = (c[k] * s).max(0.0);
        let mapped = x / (1.0 + x);
        mapped.powf(1.0 / 2.2) * 255.0
    })
}

fn region_mean(
    pixels: &[[f32; 3]],
    size: [u32; 2],
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
) -> [f32; 3] {
    let [w, h] = size;
    let ix0 = ((x0 * w as f32) as u32).min(w);
    let ix1 = ((x1 * w as f32) as u32).min(w);
    let iy0 = ((y0 * h as f32) as u32).min(h);
    let iy1 = ((y1 * h as f32) as u32).min(h);
    let mut sum = [0.0f64; 3];
    let mut n = 0u64;
    for y in iy0..iy1 {
        for x in ix0..ix1 {
            let p = pixels[(y * w + x) as usize];
            for k in 0..3 {
                sum[k] += f64::from(p[k]);
            }
            n += 1;
        }
    }
    std::array::from_fn(|k| (sum[k] / n.max(1) as f64) as f32)
}

fn tonemapped_rms(a: &[[f32; 3]], b: &[[f32; 3]], exposure: f32) -> (f32, f32) {
    let mut sq = 0.0f64;
    let mut deltas = Vec::with_capacity(a.len());
    for (&pa, &pb) in a.iter().zip(b) {
        let ta = tonemap_u8(pa, exposure);
        let tb = tonemap_u8(pb, exposure);
        let d = ((ta[0] - tb[0]).powi(2) + (ta[1] - tb[1]).powi(2) + (ta[2] - tb[2]).powi(2)) / 3.0;
        sq += f64::from(d);
        deltas.push(d.sqrt());
    }
    deltas.sort_by(f32::total_cmp);
    let p99 = deltas[((deltas.len() - 1) as f64 * 0.99) as usize];
    ((sq / a.len().max(1) as f64).sqrt() as f32, p99)
}

fn save_ppm(dir: &Path, name: &str, pixels: &[[f32; 3]], size: [u32; 2], exposure: f32) {
    let [w, h] = size;
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for &p in pixels {
        let t = tonemap_u8(p, exposure);
        out.extend(t.map(|v| v.round().clamp(0.0, 255.0) as u8));
    }
    std::fs::write(dir.join(format!("{name}.ppm")), out).unwrap();
}

/// Comprehensive reference-backed GI & RT verification harness across all 5 diagnostic scenes.
#[test]
#[ignore = "GPU lease; requires RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK"]
fn gi_quality_suite() {
    let out_dir = std::env::var("RENDERLAB_GI_QUALITY_OUT").ok().map(PathBuf::from);
    if let Some(dir) = &out_dir {
        std::fs::create_dir_all(dir).unwrap();
    }
    let only = std::env::var("RENDERLAB_GI_QUALITY_CASES").ok();
    let run_case = |name: &str| {
        only.as_deref()
            .is_none_or(|list| list.split(',').any(|s| s.trim() == name))
    };
    let mut failures: Vec<String> = vec![];

    // ---- Case 1: Classic Cornell Box ----
    if run_case("cornell_box") {
        let spec = cornell_box_scene();
        let ref_img = render_reference(&spec, REF_SPP);
        let rt = render_renderlab(&spec, true, 30, 5);
        let nrt = render_renderlab(&spec, false, 30, 5);
        if let Some(dir) = &out_dir {
            save_ppm(dir, "cornell_ref", &ref_img, spec.size, spec.exposure);
            save_ppm(dir, "cornell_rt", &rt.radiance, spec.size, spec.exposure);
            save_ppm(dir, "cornell_nonrt", &nrt.radiance, spec.size, spec.exposure);
        }
        // Left-box front/left area catches red bleed; right-box area catches green bleed.
        let ref_all = region_mean(&ref_img, spec.size, 0.1, 0.9, 0.1, 0.9);
        let rt_all = region_mean(&rt.radiance, spec.size, 0.1, 0.9, 0.1, 0.9);
        let nrt_all = region_mean(&nrt.radiance, spec.size, 0.1, 0.9, 0.1, 0.9);
        let ref_lb = region_mean(&ref_img, spec.size, 0.25, 0.42, 0.42, 0.72);
        let rt_lb = region_mean(&rt.radiance, spec.size, 0.25, 0.42, 0.42, 0.72);
        let nrt_lb = region_mean(&nrt.radiance, spec.size, 0.25, 0.42, 0.42, 0.72);
        let ref_rb = region_mean(&ref_img, spec.size, 0.56, 0.74, 0.58, 0.80);
        let rt_rb = region_mean(&rt.radiance, spec.size, 0.56, 0.74, 0.58, 0.80);
        let nrt_rb = region_mean(&nrt.radiance, spec.size, 0.56, 0.74, 0.58, 0.80);
        let (rt_ref_rms, _) = tonemapped_rms(&rt.radiance, &ref_img, spec.exposure);
        let (nrt_ref_rms, _) = tonemapped_rms(&nrt.radiance, &ref_img, spec.exposure);
        let (parity_rms, _) = tonemapped_rms(&rt.radiance, &nrt.radiance, spec.exposure);
        let rt_stab = rt.tail_frames.windows(2).map(|w| tonemapped_rms(&w[0], &w[1], spec.exposure).0).fold(0.0f32, f32::max);
        let nrt_stab = nrt.tail_frames.windows(2).map(|w| tonemapped_rms(&w[0], &w[1], spec.exposure).0).fold(0.0f32, f32::max);
        println!(
            "[cornell_box] all ref={ref_all:.2?} rt={rt_all:.2?} nrt={nrt_all:.2?} | \
             left_box R/G ref={:.2} rt={:.2} nrt={:.2} | right_box G/R ref={:.2} rt={:.2} nrt={:.2} | \
             rms(rt-ref)={rt_ref_rms:.2} rms(nrt-ref)={nrt_ref_rms:.2} parity={parity_rms:.2} stab(rt/nrt)={rt_stab:.2}/{nrt_stab:.2} | \
             rt(cr={} g={} gm={} orph={}) nrt(cr={} g={} gm={} orph={})",
            ref_lb[0] / ref_lb[1].max(1e-4), rt_lb[0] / rt_lb[1].max(1e-4), nrt_lb[0] / nrt_lb[1].max(1e-4),
            ref_rb[1] / ref_rb[0].max(1e-4), rt_rb[1] / rt_rb[0].max(1e-4), nrt_rb[1] / nrt_rb[0].max(1e-4),
            rt.counters.created, rt.counters.gathers, rt.counters.gather_misses, rt.counters.orphans,
            nrt.counters.created, nrt.counters.gathers, nrt.counters.gather_misses, nrt.counters.orphans,
        );
        if rt_ref_rms > 16.0 {
            failures.push(format!("cornell_box RT vs ref RMS {rt_ref_rms:.2} > 16.0"));
        }
        if nrt_ref_rms > 16.0 {
            failures.push(format!("cornell_box Non-RT vs ref RMS {nrt_ref_rms:.2} > 16.0"));
        }
        if parity_rms > 8.0 {
            failures.push(format!("cornell_box RT vs Non-RT parity RMS {parity_rms:.2} > 8.0"));
        }
        if rt_lb[0] / rt_lb[1].max(1e-4) < 1.15 || nrt_lb[0] / nrt_lb[1].max(1e-4) < 1.15 {
            failures.push(format!(
                "cornell_box lost red bleed on left box: rt={:.2} nrt={:.2} (ref={:.2})",
                rt_lb[0] / rt_lb[1].max(1e-4),
                nrt_lb[0] / nrt_lb[1].max(1e-4),
                ref_lb[0] / ref_lb[1].max(1e-4)
            ));
        }
        if rt_rb[1] / rt_rb[0].max(1e-4) < 1.05 || nrt_rb[1] / nrt_rb[0].max(1e-4) < 1.05 {
            failures.push(format!(
                "cornell_box lost green bleed on right box: rt={:.2} nrt={:.2} (ref={:.2})",
                rt_rb[1] / rt_rb[0].max(1e-4),
                nrt_rb[1] / nrt_rb[0].max(1e-4),
                ref_rb[1] / ref_rb[0].max(1e-4)
            ));
        }
        if rt_stab > 2.5 || nrt_stab > 2.5 {
            failures.push(format!("cornell_box temporal instability rt={rt_stab:.2} nrt={nrt_stab:.2} > 2.5"));
        }
    }

    // ---- Case 2: Inset 90-degree Corner Ambient Occlusion ----
    if run_case("inset_corner_ao") {
        let spec = inset_corner_ao_scene();
        let ref_img = render_reference(&spec, REF_SPP);
        let rt = render_renderlab(&spec, true, 30, 3);
        let nrt = render_renderlab(&spec, false, 30, 3);
        if let Some(dir) = &out_dir {
            save_ppm(dir, "corner_ref", &ref_img, spec.size, spec.exposure);
            save_ppm(dir, "corner_rt", &rt.radiance, spec.size, spec.exposure);
            save_ppm(dir, "corner_nonrt", &nrt.radiance, spec.size, spec.exposure);
        }
        // Same-wall crease vs open wall ratio averaged over both walls (so flat uniform walls
        // always score 1.000 and cannot fake corner AO via left/right brightness differences).
        let ratio = |img: &[[f32; 3]]| {
            let l_crease = lum(region_mean(img, spec.size, 0.45, 0.495, 0.35, 0.65));
            let l_open = lum(region_mean(img, spec.size, 0.08, 0.22, 0.35, 0.65));
            let r_crease = lum(region_mean(img, spec.size, 0.505, 0.55, 0.35, 0.65));
            let r_open = lum(region_mean(img, spec.size, 0.78, 0.92, 0.35, 0.65));
            let c = 0.5 * (l_crease + r_crease);
            let o = 0.5 * (l_open + r_open);
            let r = 0.5 * (l_crease / l_open.max(1e-5) + r_crease / r_open.max(1e-5));
            (c, o, r)
        };
        let (rc, ro, rr) = ratio(&ref_img);
        let (tc, to, tr) = ratio(&rt.radiance);
        let (nc, no, nr) = ratio(&nrt.radiance);
        let (rt_ref_rms, _) = tonemapped_rms(&rt.radiance, &ref_img, spec.exposure);
        let (nrt_ref_rms, _) = tonemapped_rms(&nrt.radiance, &ref_img, spec.exposure);
        let (parity_rms, _) = tonemapped_rms(&rt.radiance, &nrt.radiance, spec.exposure);
        println!(
            "[inset_corner_ao] crease/open: ref={rr:.3} ({rc:.2}/{ro:.2}) rt={tr:.3} ({tc:.2}/{to:.2}) nrt={nr:.3} ({nc:.2}/{no:.2}) | \
             rms(rt-ref)={rt_ref_rms:.2} rms(nrt-ref)={nrt_ref_rms:.2} parity={parity_rms:.2}"
        );
        if (tr - rr).abs() > 0.14 {
            failures.push(format!("inset_corner_ao RT crease/open {tr:.3} differs from ref {rr:.3} by > 0.14"));
        }
        if (nr - rr).abs() > 0.14 {
            failures.push(format!("inset_corner_ao Non-RT crease/open {nr:.3} differs from ref {rr:.3} by > 0.14"));
        }
        if parity_rms > 8.0 {
            failures.push(format!("inset_corner_ao RT vs Non-RT parity RMS {parity_rms:.2} > 8.0"));
        }
    }

    // ---- Case 3: Textured Bounce (catches GI_NO_PRIMITIVE UV collapse) ----
    if run_case("textured_bounce") {
        let spec = textured_bounce_scene();
        let ref_img = render_reference(&spec, REF_SPP);
        let rt = render_renderlab(&spec, true, 30, 1);
        let nrt = render_renderlab(&spec, false, 30, 1);
        if let Some(dir) = &out_dir {
            save_ppm(dir, "texbounce_ref", &ref_img, spec.size, spec.exposure);
            save_ppm(dir, "texbounce_rt", &rt.radiance, spec.size, spec.exposure);
            save_ppm(dir, "texbounce_nonrt", &nrt.radiance, spec.size, spec.exposure);
        }
        let ref_m = region_mean(&ref_img, spec.size, 0.3, 0.7, 0.3, 0.7);
        let rt_m = region_mean(&rt.radiance, spec.size, 0.3, 0.7, 0.3, 0.7);
        let nrt_m = region_mean(&nrt.radiance, spec.size, 0.3, 0.7, 0.3, 0.7);
        let ref_rb = ref_m[0] / ref_m[2].max(1e-4);
        let rt_rb = rt_m[0] / rt_m[2].max(1e-4);
        let nrt_rb = nrt_m[0] / nrt_m[2].max(1e-4);
        let (parity_rms, _) = tonemapped_rms(&rt.radiance, &nrt.radiance, spec.exposure);
        println!(
            "[textured_bounce] receiver mean ref={ref_m:.3?} (R/B={ref_rb:.2}) rt={rt_m:.3?} (R/B={rt_rb:.2}) nrt={nrt_m:.3?} (R/B={nrt_rb:.2}) | parity={parity_rms:.2}"
        );
        if rt_rb < 3.0 {
            failures.push(format!("textured_bounce RT lost red UV bounce: R/B={rt_rb:.2} < 3.0"));
        }
        if nrt_rb < 3.0 {
            failures.push(format!("textured_bounce Non-RT lost red UV bounce (GI_NO_PRIMITIVE): R/B={nrt_rb:.2} < 3.0"));
        }
        if parity_rms > 10.0 {
            failures.push(format!("textured_bounce RT vs Non-RT parity RMS {parity_rms:.2} > 10.0"));
        }
    }

    // ---- Case 4: Glass Sunlit Room (catches Non-RT see-through GI starvation) ----
    if run_case("glass_sunlit_room") {
        let spec = glass_sunlit_room_scene();
        let rt = render_renderlab(&spec, true, 30, 1);
        let nrt = render_renderlab(&spec, false, 30, 1);
        if let Some(dir) = &out_dir {
            save_ppm(dir, "glass_rt", &rt.radiance, spec.size, spec.exposure);
            save_ppm(dir, "glass_nonrt", &nrt.radiance, spec.size, spec.exposure);
        }
        let rt_m = region_mean(&rt.radiance, spec.size, 0.1, 0.9, 0.1, 0.9);
        let nrt_m = region_mean(&nrt.radiance, spec.size, 0.1, 0.9, 0.1, 0.9);
        let lum_ratio = lum(nrt_m) / lum(rt_m).max(1e-5);
        let (parity_rms, _) = tonemapped_rms(&rt.radiance, &nrt.radiance, spec.exposure);
        println!(
            "[glass_sunlit_room] mean rt={rt_m:.3?} nrt={nrt_m:.3?} lum_ratio(nrt/rt)={lum_ratio:.3} parity={parity_rms:.2} | \
             rt_ext={} nrt_ext={}",
            rt.counters.external_requests, nrt.counters.external_requests
        );
        if !(0.85..=1.15).contains(&lum_ratio) {
            failures.push(format!("glass_sunlit_room Non-RT/RT luminance ratio {lum_ratio:.3} outside [0.85, 1.15]"));
        }
        if parity_rms > 10.0 {
            failures.push(format!("glass_sunlit_room RT vs Non-RT parity RMS {parity_rms:.2} > 10.0"));
        }
    }

    // ---- Case 5: Full 1920x1080 Convergence, Parity & Stability Gate ----
    if run_case("hd_1080p_stress") {
        let spec = hd_sponza_stress_scene();
        let rt = render_renderlab(&spec, true, 35, 5);
        let nrt = render_renderlab(&spec, false, 35, 5);
        if let Some(dir) = &out_dir {
            save_ppm(dir, "hd_rt", &rt.radiance, spec.size, spec.exposure);
            save_ppm(dir, "hd_nonrt", &nrt.radiance, spec.size, spec.exposure);
        }
        let rt_arch = region_mean(&rt.radiance, spec.size, 0.50, 0.85, 0.02, 0.16);
        let nrt_arch = region_mean(&nrt.radiance, spec.size, 0.50, 0.85, 0.02, 0.16);
        let (parity_rms, _) = tonemapped_rms(&rt.radiance, &nrt.radiance, spec.exposure);
        let rt_stab = rt.tail_frames.windows(2).map(|w| tonemapped_rms(&w[0], &w[1], spec.exposure)).fold((0.0f32, 0.0f32), |a, b| (a.0.max(b.0), a.1.max(b.1)));
        let nrt_stab = nrt.tail_frames.windows(2).map(|w| tonemapped_rms(&w[0], &w[1], spec.exposure)).fold((0.0f32, 0.0f32), |a, b| (a.0.max(b.0), a.1.max(b.1)));
        let rt_miss_rate = rt.counters.gather_misses as f32 / rt.counters.gather_rays.max(1) as f32;
        let nrt_miss_rate = nrt.counters.gather_misses as f32 / nrt.counters.gather_rays.max(1) as f32;
        println!(
            "[hd_1080p_stress] arch_R rt={rt_arch:.3?} nrt={nrt_arch:.3?} parity={parity_rms:.2} | \
             stab_rms(rt/nrt)={:.2}/{:.2} stab_p99(rt/nrt)={:.2}/{:.2} | \
             RT(deferred={} ext={} g_miss={:.1}%) NonRT(deferred={} ext={} g_miss={:.1}%)",
            rt_stab.0, nrt_stab.0, rt_stab.1, nrt_stab.1,
            rt.counters.claim_deferred, rt.counters.external_requests, rt_miss_rate * 100.0,
            nrt.counters.claim_deferred, nrt.counters.external_requests, nrt_miss_rate * 100.0,
        );
        if rt.counters.claim_deferred > 0 {
            failures.push(format!("hd_1080p RT still deferring claims at frame 35: {}", rt.counters.claim_deferred));
        }
        if nrt.counters.claim_deferred > 0 {
            failures.push(format!("hd_1080p Non-RT still deferring claims at frame 35: {}", nrt.counters.claim_deferred));
        }
        if rt.counters.external_requests > 256 || nrt.counters.external_requests > 256 {
            failures.push(format!(
                "hd_1080p unconverged external_requests at frame 35: rt={} nrt={}",
                rt.counters.external_requests, nrt.counters.external_requests
            ));
        }
        if parity_rms > 10.0 {
            failures.push(format!("hd_1080p RT vs Non-RT parity RMS {parity_rms:.2} > 10.0"));
        }
        if rt_stab.0 > 1.8 || nrt_stab.0 > 1.8 || rt_stab.1 > 7.0 || nrt_stab.1 > 7.0 {
            failures.push(format!(
                "hd_1080p temporal instability: rt=({:.2}, p99={:.2}) nrt=({:.2}, p99={:.2})",
                rt_stab.0, rt_stab.1, nrt_stab.0, nrt_stab.1
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "gi_quality_suite failed ({} issues):\n  - {}",
        failures.len(),
        failures.join("\n  - ")
    );
}
