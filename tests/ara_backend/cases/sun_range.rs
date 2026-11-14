//! The sun's 1 km cascade on the port (#96, `Settings` sun splits `[4, 12, 40, 1000]`): a
//! column 300 m out shadows the ground past it, about 90 m from the camera. With the 1 km end
//! the shadow is there; with the ends before it (`[4, 12, 40]`) that ground is lit.
use super::*;
use renderlab::{
    Camera, Light, Lighting, Mat4, MaterialDescriptor, Vec3, Vertex,
    backend_ara::forward::{ForwardObject, ForwardScene, Primary, SunShadows, render},
};

const SIZE: [u32; 2] = [160, 96];

/// An axis-aligned box from `low` to `high`, outward normals, as two triangles a face.
fn cuboid(low: [f32; 3], high: [f32; 3]) -> (Vec<Vertex>, Vec<u32>) {
    let (mut vertices, mut indices) = (Vec::new(), Vec::new());
    for axis in 0..3 {
        for high_side in [false, true] {
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            let base = vertices.len() as u32;
            for [a, b] in [[false, false], [true, false], [true, true], [false, true]] {
                let mut p = [0.0; 3];
                p[axis] = if high_side { high[axis] } else { low[axis] };
                p[u] = if a { high[u] } else { low[u] };
                p[v] = if b { high[v] } else { low[v] };
                let mut normal = [0.0; 3];
                normal[axis] = if high_side { 1.0 } else { -1.0 };
                vertices.push(Vertex {
                    position: p,
                    normal,
                    uv: [0.0; 2],
                    color: [1.0; 4],
                });
            }
            let quad = [0, 1, 2, 0, 2, 3].map(|i| base + i);
            if high_side {
                indices.extend(quad);
            } else {
                indices.extend([quad[0], quad[2], quad[1], quad[3], quad[5], quad[4]]);
            }
        }
    }
    (vertices, indices)
}

#[test]
fn the_sun_shadows_ground_past_40_m_with_the_1_km_cascade() -> Result<()> {
    let path = root("sun-range");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let (ground, ground_indices) = cuboid([-1000., -1000., -1.], [1000., 1000., 0.]);
    let (column, column_indices) = cuboid([-5., 295., 0.], [5., 305., 100.]);
    let material = || MaterialDescriptor {
        base_color: [0.6, 0.6, 0.6, 1.],
        roughness: 0.8,
        double_sided: true,
        ..Default::default()
    };
    let objects =
        [(&ground, &ground_indices), (&column, &column_indices)].map(|(vertices, indices)| {
            ForwardObject {
                vertices,
                indices,
                transform: Mat4::IDENTITY,
                material: material(),
            }
        });
    // The sun travels north and down at 45 degrees: the column's shadow runs from y = 305 to
    // about 405 m. The camera looks at its middle from 90 m away.
    let lighting = Lighting {
        lights: vec![Light::Directional {
            travel: Vec3::new(0., 1., -1.).normalize(),
            lux: [1000.; 3],
        }],
        environment: [20.; 3],
        exposure_stops: -6.,
    };
    let camera = Camera::look_at(
        Vec3::new(60., 300., 30.),
        Vec3::new(0., 360., 0.),
        Vec3::Z,
        30f32.to_radians(),
        0.1,
        3000.,
    )?;
    // Each pixel's exposed luminance.
    let mut frame = |split_ends: Vec<f32>| -> Result<Vec<f32>> {
        let images = render(
            &mut gpu,
            &cache,
            &ForwardScene {
                meshes: None,
                materials: None,
                instances: None,
                objects: &objects,
                lighting: &lighting,
                camera,
                size: SIZE,
                atmosphere: renderlab::Atmosphere::off(),
                sun_shadows: Some(&SunShadows {
                    split_ends,
                    disk_angle: 0.0093,
                }),
                primary: Primary::Traced,
                post: None,
                indirect: None,
                reconstruction: None,
                reflections: None,
                textures: None,
                local_shadows: None,
                transparent: &[],
                programs: None,
                sky_map: None,
                skins: None,
                casts: None,
                colour_lut: None,
                ids: None,
            },
        )?;
        Ok(images
            .hdr
            .as_chunks::<8>()
            .0
            .iter()
            .map(|p| {
                let c = |k: usize| {
                    f16::from_bits(u16::from_le_bytes([p[2 * k], p[2 * k + 1]])).to_f32()
                };
                0.2126 * c(0) + 0.7152 * c(1) + 0.0722 * c(2)
            })
            .collect())
    };
    let far = frame(vec![4., 12., 40., 1000.])?;
    let near = frame(vec![4., 12., 40.])?;
    // Pixels the 1 km cascade at least halves (the column's shadow past 40 m), and pixels the
    // shorter set halves (none: it only drops shadows).
    let shadowed = far
        .iter()
        .zip(&near)
        .filter(|(f, n)| **f < 0.5 * **n)
        .count();
    let darker = far
        .iter()
        .zip(&near)
        .filter(|(f, n)| **n < 0.5 * **f)
        .count();
    println!(
        "sun range: {shadowed} pixels shadowed only with the 1 km cascade, {darker} only without it"
    );
    ensure!(
        shadowed > 300,
        "the 1 km cascade shadows the ground past 40 m ({shadowed} px)"
    );
    ensure!(darker == 0, "the 40 m set darkened {darker} px");
    Ok(())
}
