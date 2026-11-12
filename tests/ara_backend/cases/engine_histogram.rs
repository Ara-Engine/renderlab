//! A view's luminance histogram binned on the GPU (`Engine::view_histogram`).
use super::*;
use renderlab::{
    Camera, Mat4, MeshData, Vec3, Vertex,
    backend_ara::{engine::Engine, histogram::HistogramDesc},
    engine::{Exposure, MaterialDesc, Quality, Settings, Sky, Sun, ViewDesc, ViewLook},
};

const SIZE: [u32; 2] = [96, 64];
/// An exposure meter's settings: 512 bins from a luminance of 1e-4, every other pixel.
const DESC: HistogramDesc = HistogramDesc {
    bins: 512,
    log_min: -9.210_34,
    stride: 2,
};

/// The histogram `DESC` asks for, of display-encoded `rgba8` `SIZE[0]` pixels wide, on the CPU.
fn histogram_of(rgba8: &[u8]) -> Vec<u32> {
    let decode = |v: u8| {
        let s = f32::from(v) / 255.;
        if s <= 0.040_45 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    let mut bins = vec![0u32; DESC.bins as usize];
    for (i, p) in rgba8.as_chunks::<4>().0.iter().enumerate() {
        let (x, y) = (i as u32 % SIZE[0], i as u32 / SIZE[0]);
        if x % DESC.stride != 0 || y % DESC.stride != 0 {
            continue;
        }
        let l = (0.2126 * decode(p[0]) + 0.7152 * decode(p[1]) + 0.0722 * decode(p[2])).max(1e-4);
        let x = (l.ln() - DESC.log_min) / -DESC.log_min;
        bins[(x * DESC.bins as f32)
            .floor()
            .clamp(0., DESC.bins as f32 - 1.) as usize] += 1;
    }
    bins
}

/// A sunlit grey floor under a uniform sky, from above at an angle: the GPU histogram of
/// the displayed frame is the CPU histogram of the same frame read back, sample for sample
/// (a sample may move one bin where the GPU's and the CPU's power functions round apart),
/// handed over by `histograms` once the GPU has finished it, with no wait in between.
#[test]
fn a_view_histogram_bins_its_displayed_image_on_the_gpu() -> Result<()> {
    let path = root("engine-histogram");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let camera = Camera::look_at(
        Vec3::new(0., 4., 8.),
        Vec3::ZERO,
        Vec3::Y,
        55f32.to_radians(),
        0.1,
        100.,
    )?;
    let mut engine = Engine::new(
        ShaderCache::new(path.join("shaders"))?,
        Settings {
            volumetric_fog: None,
            ..Default::default()
        },
    )?;
    let view = engine.create_view(&ViewDesc {
        look: ViewLook {
            exposure: Exposure::Manual { stops: -11. },
            quality: Quality::Reference,
            ..Default::default()
        },
        ..ViewDesc::new(SIZE[0], SIZE[1])
    })?;
    engine.set_sky(Sky::Uniform([1000.; 3]))?;
    engine.set_sun(Some(Sun {
        travel: Vec3::new(0.2, -1., 0.3).normalize(),
        illuminance: [50_000.; 3],
        angular_radius: 0.02,
    }))?;
    let floor = [[-6., -6.], [6., -6.], [6., 6.], [-6., 6.]].map(|[x, z]| Vertex {
        position: [x, 0., z],
        normal: [0., 1., 0.],
        uv: [0., 0.],
        color: [1.; 4],
    });
    let mesh = engine.create_mesh(&MeshData {
        vertices: &floor,
        indices: &[0, 2, 1, 0, 3, 2],
    })?;
    let material = engine.create_material(&MaterialDesc {
        base_color: [0.5, 0.5, 0.5, 1.],
        ..Default::default()
    })?;
    engine.spawn(mesh, material, Mat4::IDENTITY)?;
    engine.render_view(&mut gpu, view, &camera)?;
    let request = engine.view_histogram(&mut gpu, view, DESC)?;
    gpu.collect()?;
    let mut readings = engine.histograms(&mut gpu)?;
    // Not waited for: the GPU finishes it later, and the engine hands it over then.
    for _ in 0..200 {
        if !readings.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        readings = engine.histograms(&mut gpu)?;
    }
    let reading = readings.first().context("the histogram never finished")?;
    ensure!(reading.request == request && reading.view == view);
    let expected = histogram_of(&engine.capture_view(view)?.rgba8);
    let total: u32 = expected.iter().sum();
    ensure!(
        reading.bins.iter().sum::<u32>() == total
            && total == SIZE[0].div_ceil(2) * SIZE[1].div_ceil(2),
        "the histogram counts {} samples, not {total}",
        reading.bins.iter().sum::<u32>()
    );
    // Each sample lies within one bin of its CPU bin: the cumulative counts agree to within
    // the samples one bin can hold.
    let (mut gpu_sum, mut cpu_sum, mut worst) = (0i64, 0i64, 0i64);
    for (g, c) in reading.bins.iter().zip(&expected) {
        gpu_sum += i64::from(*g);
        cpu_sum += i64::from(*c);
        worst = worst.max((gpu_sum - cpu_sum).abs());
    }
    let largest = i64::from(*expected.iter().max().unwrap_or(&0));
    ensure!(
        worst <= largest,
        "the GPU histogram strays more than a bin from the CPU's: {worst} samples"
    );
    let distinct = expected.iter().filter(|&&c| c > 0).count();
    ensure!(distinct >= 2, "the frame should hold sky and floor levels");
    engine.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
