//! `Engine::set_casts_shadows` on the port: a point light 1 m above a floor inside its own
//! closed 1 m box (a lamp in its shade). With the box casting, the box blocks all of the
//! light; with it not casting, the floor is lit as if the box were not there; switching the
//! flag on a running engine flips the frame. Shadow rays cull the box's acceleration instance
//! by `SHADOW_CASTER_MASK`; camera rays still see it.
use super::*;
use renderlab::{
    Camera, Light, LocalShadows, Mat4, MeshData, ReflectionSettings, Vec3, Vertex,
    backend_ara::engine::Engine,
    engine::{Exposure, MaterialDesc, ObjectId, Settings, Sky, ViewDesc, ViewLook},
};

const SIZE: [u32; 2] = [160, 96];

/// An axis-aligned box about `centre` with half extents `half`, outward normals.
fn cube(centre: [f32; 3], half: [f32; 3]) -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for axis in 0..3 {
        for sign in [-1.0f32, 1.0] {
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            let base = vertices.len() as u32;
            for [a, b] in [[-1.0f32, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]] {
                let mut p = centre;
                p[axis] += sign * half[axis];
                p[u] += a * half[u];
                p[v] += b * half[v];
                let mut normal = [0.0; 3];
                normal[axis] = sign;
                vertices.push(Vertex {
                    position: p,
                    normal,
                    uv: [0.0, 0.0],
                    color: [1.0; 4],
                });
            }
            let quad = [0, 1, 2, 0, 2, 3].map(|i| base + i);
            if sign > 0.0 {
                indices.extend(quad);
            } else {
                indices.extend([quad[0], quad[2], quad[1], quad[3], quad[5], quad[4]]);
            }
        }
    }
    (vertices, indices)
}

/// The floor's mean exposed luminance beside the box: the image's left and right thirds.
fn floor_luminance(hdr: &[u8]) -> f32 {
    let (mut sum, mut n) = (0.0, 0);
    for (i, p) in hdr.as_chunks::<8>().0.iter().enumerate() {
        let x = i as u32 % SIZE[0];
        if x >= SIZE[0] / 3 && x < 2 * SIZE[0] / 3 {
            continue;
        }
        let c = |k: usize| f16::from_bits(u16::from_le_bytes([p[2 * k], p[2 * k + 1]])).to_f32();
        sum += 0.2126 * c(0) + 0.7152 * c(1) + 0.0722 * c(2);
        n += 1;
    }
    sum / n as f32
}

/// The lamp scene's engine and view, with the box placed (`Some(casts)`) or left out.
fn lamp(path: &std::path::Path, shade: Option<bool>) -> Result<(Engine, renderlab::engine::ViewId, Option<ObjectId>)> {
    let settings = Settings {
        global_illumination: false,
        temporal_reconstruction: false,
        local_shadows: LocalShadows::default(),
        ..Default::default()
    };
    let mut engine = Engine::new(ShaderCache::new(path.join("shaders"))?, settings)?;
    let view = engine.create_view(&ViewDesc {
        look: ViewLook {
            exposure: Exposure::Manual { stops: -8.0 },
            reflections: ReflectionSettings::off(),
            ..Default::default()
        },
        ..ViewDesc::new(SIZE[0], SIZE[1])
    })?;
    engine.set_sky(Sky::Uniform([0.0; 3]))?;
    engine.set_sun(None)?;
    let mut spawn = |engine: &mut Engine, centre, half, colour| -> Result<ObjectId> {
        let (vertices, indices) = cube(centre, half);
        let mesh = engine.create_mesh(&MeshData {
            vertices: &vertices,
            indices: &indices,
        })?;
        let material = engine.create_material(&MaterialDesc {
            base_color: colour,
            double_sided: true,
            ..Default::default()
        })?;
        Ok(engine.spawn(mesh, material, Mat4::IDENTITY)?)
    };
    spawn(&mut engine, [0.0, -0.05, 0.0], [6.0, 0.05, 6.0], [0.7, 0.7, 0.7, 1.0])?;
    let shade = match shade {
        Some(casts) => {
            let shade = spawn(&mut engine, [0.0, 1.0, 0.0], [0.5, 0.5, 0.5], [0.8, 0.8, 0.8, 1.0])?;
            engine.set_casts_shadows(shade, casts)?;
            Some(shade)
        }
        None => None,
    };
    engine.set_lights(&[Light::Point {
        position: Vec3::new(0.0, 1.0, 0.0),
        candela: [1000.0; 3],
        range: None,
        source_radius: 0.0,
        cast_shadows: Some(true),
    }])?;
    Ok((engine, view, shade))
}

/// Four frames of `engine`'s view; the last one's floor luminance.
fn frames(engine: &mut Engine, gpu: &mut GpuDevice, view: renderlab::engine::ViewId) -> Result<f32> {
    let camera = Camera::look_at(Vec3::new(0.0, 7.0, 5.0), Vec3::ZERO, Vec3::Y, 0.9, 0.05, 100.0)?;
    for _ in 0..4 {
        engine.render_view(gpu, view, &camera)?;
    }
    let hdr = &engine.images(view).context("no frame")?.hdr;
    Ok(floor_luminance(hdr))
}

#[test]
fn a_lamp_in_a_box_that_casts_no_shadow_lights_the_floor() -> Result<()> {
    let path = root("casts-shadows");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let mut measure = |shade: Option<bool>| -> Result<f32> {
        let (mut engine, view, _) = lamp(&path, shade)?;
        let lit = frames(&mut engine, &mut gpu, view)?;
        let refusals = engine.refusals().to_vec();
        engine.retire(&mut gpu)?;
        ensure!(refusals.is_empty(), "refused: {refusals:?}");
        Ok(lit)
    };
    let open = measure(None)?;
    let blocked = measure(Some(true))?;
    let through = measure(Some(false))?;
    println!("casts shadows: floor beside the lamp, open {open:.4}, casting box {blocked:.4}, non-casting box {through:.4}");
    ensure!(open > 0.002, "the open lamp lights the floor ({open})");
    ensure!(blocked < 0.05 * open, "a casting box blocks the lamp ({blocked} against {open})");
    ensure!(
        (through - open).abs() < 0.05 * open,
        "a non-casting box lets the lamp light the floor as if absent ({through} against {open})"
    );
    // The flag on a running engine: casting, then not.
    let (mut engine, view, shade) = lamp(&path, Some(true))?;
    let before = frames(&mut engine, &mut gpu, view)?;
    engine.set_casts_shadows(shade.context("placed")?, false)?;
    let after = frames(&mut engine, &mut gpu, view)?;
    engine.retire(&mut gpu)?;
    println!("casts shadows: toggled mid-run, {before:.4} then {after:.4}");
    ensure!(before < 0.05 * open && (after - open).abs() < 0.05 * open, "the toggle flips the frame");
    Ok(())
}
