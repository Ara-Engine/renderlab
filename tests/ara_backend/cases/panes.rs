//! Transparent panes on the traced frame, against what a thin pane must pass.
use super::*;
use glam::DVec3;
use renderlab::{
    Camera, Light, Lighting, Mat4, MaterialDescriptor, Transparency, Vec3, Vertex,
    backend_ara::forward::{ForwardObject, ForwardScene, Primary, SunShadows, render},
};

const SIZE: [u32; 2] = [128, 96];
const IOR: f64 = 1.5;
/// A pane: its square, its coverage and its tint.
#[derive(Clone, Copy)]
struct Pane<'a> {
    vertices: &'a [Vertex; 4],
    opacity: f32,
    tint: [f32; 3],
}

fn quad(half: f32, z: f32, centre: [f32; 2]) -> [Vertex; 4] {
    [[-half, -half], [half, -half], [half, half], [-half, half]].map(|[x, y]| Vertex {
        position: [centre[0] + x, centre[1] + y, z],
        normal: [0., 0., 1.],
        uv: [0.; 2],
        color: [1.; 4],
    })
}

/// A pane's light is its own shading plus what it passes of the colour behind it:
/// `out = lit + T * behind`. Two frames with different floors behind the same pane
/// differ only in `behind`, so their difference over the two bare floors' difference is
/// `T` alone, per pixel and channel, whatever the pane's own lighting is.
///
/// - Inside a pane `T` is the standard program's transmittance: what its coverage
///   leaves, times what Schlick's Fresnel reflectance at the pixel's view angle leaves,
///   times the tint.
/// - Outside every pane the frame is the bare floor's, bit for bit.
/// - At a pane's edge the four coverage samples leave whole quarters: the pixel passes
///   `1 - c (1 - T)` for `c` a quarter, a half or three quarters.
/// - Where two different panes overlap, the floor's light passes both, and the far
///   pane's own light passes the near one: the frame is the near pane alone plus its
///   transmittance times what the far pane alone adds to the floor. That holds in
///   either submission order, because the pass sorts far to near; drawn in the other
///   order the two panes' own light would swap roles, a difference the test measures
///   and requires to be large against its tolerance.
/// - The mesh pass refuses panes.
#[test]
fn a_pane_passes_what_its_coverage_fresnel_and_tint_leave() -> Result<()> {
    let path = root("panes");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let floor = quad(8., 0., [0.; 2]);
    // The near pane over the middle, the far one under its right half and beyond.
    let (near, far) = (quad(0.6, 1.5, [0.; 2]), quad(0.6, 0.75, [0.7, 0.]));
    let near = Pane {
        vertices: &near,
        opacity: 0.25,
        tint: [0.8, 0.9, 1.0],
    };
    let far = Pane {
        vertices: &far,
        opacity: 0.5,
        tint: [1.0, 0.6, 0.5],
    };
    let lighting = Lighting {
        lights: vec![Light::Directional {
            travel: Vec3::new(0.2, 0.1, -1.).normalize(),
            lux: [1000.; 3],
        }],
        environment: [0.; 3],
        exposure_stops: -8.,
    };
    let shadows = SunShadows {
        split_ends: vec![8.],
        disk_angle: 0.02,
    };
    let eye = Vec3::new(0., 0., 4.);
    let camera = Camera::look_at(eye, Vec3::ZERO, Vec3::Y, 55f32.to_radians(), 0.1, 100.)?;
    let mut frame = |albedo: f32, panes: &[Pane<'_>], primary: Primary| -> Result<Vec<[f32; 3]>> {
        let material = |base: [f32; 3], transparency| MaterialDescriptor {
            base_color: [base[0], base[1], base[2], 1.],
            metallic: 0.,
            roughness: 0.5,
            double_sided: true,
            transparency,
            ..Default::default()
        };
        let objects = [ForwardObject {
            vertices: &floor,
            indices: &[0, 1, 2, 0, 2, 3],
            transform: Mat4::IDENTITY,
            material: material([albedo; 3], None),
        }];
        let transparent: Vec<_> = panes
            .iter()
            .map(|pane| ForwardObject {
                vertices: pane.vertices,
                indices: &[0, 1, 2, 0, 2, 3],
                transform: Mat4::IDENTITY,
                material: material(
                    pane.tint,
                    Some(Transparency {
                        opacity: pane.opacity,
                        ior: IOR as f32,
                        thickness: 0.,
                    }),
                ),
            })
            .collect();
        let images = render(
            &mut gpu,
            &cache,
            &ForwardScene {
                objects: &objects,
                lighting: &lighting,
                camera,
                size: SIZE,
                atmosphere: renderlab::Atmosphere::off(),
                sun_shadows: Some(&shadows),
                primary,
                post: None,
                indirect: None,
                reconstruction: None,
                reflections: None,
                textures: None,
                local_shadows: None,
                transparent: &transparent,
            },
        )?;
        Ok(images
            .hdr
            .as_chunks::<8>()
            .0
            .iter()
            .map(|p| {
                std::array::from_fn(|c| {
                    f16::from_bits(u16::from_le_bytes([p[2 * c], p[2 * c + 1]])).to_f32()
                })
            })
            .collect())
    };
    let bare = [
        frame(0.2, &[], Primary::Traced)?,
        frame(0.6, &[], Primary::Traced)?,
    ];
    // What each pixel's centre ray meets: how far inside a pane's square it crosses the
    // pane's plane (negative outside), in pixels, and the cosine of its angle to the
    // pane's normal.
    let inverse = camera.matrix(SIZE[0], SIZE[1]).as_dmat4().inverse();
    let eye = eye.as_dvec3();
    let pixels_per_metre =
        |z: f64| f64::from(SIZE[1]) / (2. * (eye.z - z) * (0.5 * 55f64.to_radians()).tan());
    let rays: Vec<DVec3> = (0..SIZE[0] * SIZE[1])
        .map(|i| {
            let (x, y) = (i % SIZE[0], i / SIZE[0]);
            let far = inverse.project_point3(DVec3::new(
                (f64::from(x) + 0.5) / f64::from(SIZE[0]) * 2. - 1.,
                1. - (f64::from(y) + 0.5) / f64::from(SIZE[1]) * 2.,
                0.5,
            ));
            (far - eye).normalize()
        })
        .collect();
    let inside = |ray: DVec3, z: f64, centre: [f64; 2]| {
        let at = eye + ray * ((z - eye.z) / ray.z);
        (0.6 - (at.x - centre[0]).abs().max((at.y - centre[1]).abs())) * pixels_per_metre(z)
    };
    let transmittance = |pane: Pane<'_>, ray: DVec3| -> [f64; 3] {
        let f0 = ((IOR - 1.) / (IOR + 1.)).powi(2);
        let fresnel = f0 + (1. - f0) * (1. - ray.z.abs()).powi(5);
        pane.tint
            .map(|tint| (1. - f64::from(pane.opacity)) * (1. - fresnel) * f64::from(tint))
    };
    // Each pane alone over the darker floor, for the far pane's own light.
    let alone = [
        frame(0.2, &[near], Primary::Traced)?,
        frame(0.2, &[far], Primary::Traced)?,
    ];
    let mut report = String::new();
    for (name, panes) in [
        ("one pane", vec![near]),
        ("two panes, near first", vec![near, far]),
        ("two panes, far first", vec![far, near]),
    ] {
        let through = [
            frame(0.2, &panes, Primary::Traced)?,
            frame(0.6, &panes, Primary::Traced)?,
        ];
        let (mut interior, mut outside, mut edges, mut both, mut worst) = (0, 0, 0, 0, 0f64);
        // Under both panes: how far the frame is from the right order, and how far the
        // wrong order would have been from it.
        let (mut ordered, mut swapped) = (0f64, 0f64);
        for (i, &ray) in rays.iter().enumerate() {
            let (x, y) = (i as u32 % SIZE[0], i as u32 / SIZE[0]);
            let depths = [
                inside(ray, 1.5, [0., 0.]),
                if panes.len() == 2 {
                    inside(ray, 0.75, [0.7, 0.])
                } else {
                    f64::NEG_INFINITY
                },
            ];
            let passed: [f64; 3] = std::array::from_fn(|c| {
                f64::from(through[1][i][c] - through[0][i][c])
                    / f64::from(bare[1][i][c] - bare[0][i][c])
            });
            let (one, other) = (transmittance(near, ray), transmittance(far, ray));
            if depths.iter().all(|d| *d < -1.5) {
                outside += 1;
                ensure!(
                    through[0][i] == bare[0][i] && through[1][i] == bare[1][i],
                    "{name}: pixel ({x}, {y}) outside every pane is not the bare floor's"
                );
            } else if depths.iter().all(|d| d.abs() > 1.5) {
                // Whole pixels: inside one pane or both.
                let count = depths.iter().filter(|d| **d > 0.).count() as i32;
                interior += 1;
                both += usize::from(count == 2);
                let under = [depths[0] > 0., depths[1] > 0.];
                for c in 0..3 {
                    let expected = [one[c], other[c]]
                        .into_iter()
                        .zip(under)
                        .map(|(t, covers)| if covers { t } else { 1. })
                        .product::<f64>();
                    if count == 2 {
                        // near alone + T_near (far alone - floor), against the swap.
                        let floor = f64::from(bare[0][i][c]);
                        let (n, f) = (f64::from(alone[0][i][c]), f64::from(alone[1][i][c]));
                        let got = f64::from(through[0][i][c]);
                        ordered = ordered.max((got - (n + one[c] * (f - floor))).abs());
                        swapped = swapped
                            .max(((n + one[c] * (f - floor)) - (f + other[c] * (n - floor))).abs());
                    }
                    worst = worst.max((passed[c] - expected).abs());
                    ensure!(
                        (passed[c] - expected).abs() < 0.01,
                        "{name}: pixel ({x}, {y}) inside {count} pane(s) passes {:.4} of channel {c}, expected {expected:.4}",
                        passed[c]
                    );
                }
            } else if depths[0].abs() < 0.4 && depths[1] < -1.5 {
                // The near pane's own edge over bare floor: quarters of its coverage.
                let covered = (1. - passed[1]) / (1. - one[1]) * 4.;
                ensure!(
                    (covered - covered.round()).abs() < 0.08
                        && (0. ..=4.).contains(&covered.round()),
                    "{name}: edge pixel ({x}, {y}) is covered {covered:.3} quarters"
                );
                edges += usize::from(covered.round() > 0.5 && covered.round() < 3.5);
            }
        }
        ensure!(
            panes.len() == 1 || (ordered < 0.004 && swapped > 10. * ordered.max(1e-3)),
            "{name}: under both panes the frame is {ordered:.4} from far-then-near, and the other order would differ by {swapped:.4}"
        );
        ensure!(
            interior > 1000 && outside > 5000 && edges > 20 && (panes.len() == 1 || both > 200),
            "{name}: the fixture lost its regions: {interior} inside, {outside} outside, {edges} partly covered, {both} under both"
        );
        report += &format!(
            "{name}: {interior} inside within {worst:.4}, {edges} edge pixels on quarters, {both} under both ({ordered:.4} from far-then-near, the other order {swapped:.4} away); "
        );
    }
    ensure!(
        frame(0.2, &[near], Primary::Raster).is_err(),
        "the mesh pass accepted a transparent pane"
    );
    eprintln!("panes: {report}");
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
