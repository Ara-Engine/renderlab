//! Object ids through the backend's engine-level entry (docs/design-id-output.md).
use super::*;
use renderlab::{
    Camera, ID_NONE, ID_UNTAGGED, IdLayer, IdRect, Mat4, MeshData, Vec3, Vertex,
    backend_ara::engine::Engine,
    engine::{Exposure, MaterialDesc, Outputs, Quality, Settings, Sky, Sun, ViewDesc, ViewLook},
};

const SIZE: [u32; 2] = [96, 64];

/// A square of `half` metres a side centred at `centre`, facing +Y.
fn square(centre: Vec3, half: f32) -> Vec<Vertex> {
    [[-half, -half], [half, -half], [half, half], [-half, half]]
        .into_iter()
        .map(|[x, z]| Vertex {
            position: [centre.x + x, centre.y, centre.z + z],
            normal: [0., 1., 0.],
            uv: [0., 0.],
            color: [1.; 4],
        })
        .collect()
}

/// The pixel of the view that shows world point `point`.
fn pixel(camera: &Camera, point: Vec3) -> [u32; 2] {
    let ndc = camera.matrix(SIZE[0], SIZE[1]).project_point3(point);
    [
        ((ndc.x + 1.) * 0.5 * SIZE[0] as f32) as u32,
        ((1. - ndc.y) * 0.5 * SIZE[1] as f32) as u32,
    ]
}

/// The packed id image's bytes, read back on `gpu`.
fn read_packed(gpu: &mut GpuDevice, image: ara_gpu::Image) -> Result<Vec<u8>> {
    let bytes = u64::from(SIZE[0]) * u64::from(SIZE[1]) * 4;
    let buffer = gpu.create_readback_buffer(bytes)?;
    let mut rec = gpu.begin(Queue::Graphics);
    rec.barriers([ara_gpu::Barrier {
        resource: Resource::Buffer(buffer),
        before: Access::Undefined,
        after: Access::CopyDestination,
        handoff: None,
    }]);
    rec.push(Command::ReadImage {
        source: image,
        destination: buffer,
    });
    rec.barriers([ara_gpu::Barrier {
        resource: Resource::Buffer(buffer),
        before: Access::CopyDestination,
        after: Access::HostRead,
        handoff: None,
    }]);
    let point = gpu.submit(rec.finish(), &[])?;
    gpu.resolve(point)?;
    let read = gpu.read(buffer)?;
    gpu.retire(Resource::Buffer(buffer))?;
    Ok(read)
}

/// From above, a floor tagged 7 a dozen metres square, a raised square at its centre
/// tagged 9, an untagged square beside it and the open sky past the floor's edge. On the
/// exact frame, and on the reconstructed one after four still frames:
/// - the readback of the whole view gives 9 at the raised square's centre, 7 on the
///   floor, `ID_UNTAGGED` on the untagged square and `ID_NONE` past the floor, in both
///   layers (no glass: the front id is the opaque one);
/// - the packed opaque layer is the readback's ids, little-endian, at every pixel.
#[test]
fn a_view_with_ids_names_what_each_pixel_shows() -> Result<()> {
    let path = root("engine-ids");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let camera = Camera::look_at(
        Vec3::new(0., 10., 0.),
        Vec3::ZERO,
        Vec3::Z,
        55f32.to_radians(),
        0.1,
        100.,
    )?;
    let points = [
        (Vec3::new(0., 1., 0.), 9),
        (Vec3::new(0., 0., 3.5), 7),
        (Vec3::new(4., 0.5, 0.), ID_UNTAGGED),
        (Vec3::new(7.4, 0., 0.), ID_NONE),
    ];
    for (quality, history) in [(Quality::Reference, false), (Quality::Realtime, true)] {
        let settings = Settings {
            temporal_reconstruction: history,
            volumetric_fog: None,
            ..Default::default()
        };
        let mut engine = Engine::new(ShaderCache::new(path.join("shaders"))?, settings)?;
        let view = engine.create_view(&ViewDesc {
            look: ViewLook {
                exposure: Exposure::Manual { stops: -11. },
                quality,
                ..Default::default()
            },
            outputs: Outputs { ids: true },
            ..ViewDesc::new(SIZE[0], SIZE[1])
        })?;
        engine.set_sky(Sky::Uniform([1000.; 3]))?;
        engine.set_sun(Some(Sun {
            travel: Vec3::new(0.2, -1., 0.3).normalize(),
            illuminance: [50_000.; 3],
            angular_radius: 0.02,
        }))?;
        let material = engine.create_material(&MaterialDesc {
            base_color: [0.6, 0.6, 0.6, 1.],
            double_sided: true,
            ..Default::default()
        })?;
        for (centre, half, id) in [
            (Vec3::ZERO, 6., Some(7)),
            (Vec3::new(0., 1., 0.), 1., Some(9)),
            (Vec3::new(4., 0.5, 0.), 1., None),
        ] {
            let mesh = engine.create_mesh(&MeshData {
                vertices: &square(centre, half),
                indices: &[0, 2, 1, 0, 3, 2],
            })?;
            let object = engine.spawn(mesh, material, Mat4::IDENTITY)?;
            if let Some(id) = id {
                engine.set_object_ids(object, &[(0, u32::MAX, id)])?;
            }
        }
        for _ in 0..if history { 4 } else { 1 } {
            engine.render_view(&mut gpu, view, &camera)?;
        }
        let request = engine.request_ids(
            &mut gpu,
            view,
            IdRect {
                x: 0,
                y: 0,
                width: SIZE[0],
                height: SIZE[1],
            },
        )?;
        let events = engine.poll()?;
        let read = events
            .ids
            .iter()
            .find(|r| r.request == request)
            .context("the request did not arrive at poll")?;
        for (point, expected) in points {
            let [x, y] = pixel(&camera, point);
            let got = read.at(x, y);
            ensure!(
                got.opaque == expected && got.front == expected,
                "{quality:?}: the pixel at {point} ({x}, {y}) reads {got:?}, not {expected:#x}"
            );
        }
        let packed = engine
            .view_ids_packed(&mut gpu, view, IdLayer::Opaque)?
            .image;
        let bytes = read_packed(&mut gpu, packed)?;
        let differing = read
            .ids
            .iter()
            .zip(bytes.as_chunks::<4>().0)
            .filter(|(id, b)| id.opaque.to_le_bytes() != **b)
            .count();
        ensure!(
            differing == 0,
            "{quality:?}: the packed image differs from the readback at {differing} pixels"
        );
        engine.retire(&mut gpu)?;
    }
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
