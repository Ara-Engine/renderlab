use anyhow::{Context, Result, ensure};
use ara_gpu::{Access, Device, GpuDevice, Queue, Resource, descriptors::Format};
use renderlab::backend_ara::{
    bloom::Bloom,
    cache::ShaderCache,
    graph::Graph,
    image::Texture,
    post::{PostPasses, SettingsRecord, ToneInputs},
    transfer,
};
use std::path::{Path, PathBuf};

pub fn run() -> Result<()> {
    let mut input = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ara-post.pfm");
    let mut output = PathBuf::from("target/ara-backend/first-metal.png");
    let mut seconds = 8f32;
    let mut window = true;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input" => input = args.next().context("--input needs a PFM path")?.into(),
            "--output" => output = args.next().context("--output needs a PNG path")?.into(),
            "--seconds" => seconds = args.next().context("--seconds needs a value")?.parse()?,
            "--headless" => window = false,
            _ => anyhow::bail!("unknown option {arg}"),
        }
    }
    ensure!(
        seconds.is_finite() && (0.0..=300.0).contains(&seconds),
        "seconds must be 0..300"
    );
    let (size, pixels) = read_pfm(&input)?;
    let cache_root = Path::new("target/ara-backend/example-cache");
    let mut gpu = GpuDevice::new_raster(&[], cache_root.join("device"))?;
    let cache = ShaderCache::new(cache_root.join("shaders"))?;
    let passes = PostPasses::new(&mut gpu, &cache, Format::Rgba32Float)?;
    let hdr = Texture::new(&mut gpu, size, Format::Rgba16Float, false, false)?;
    let linear = Texture::new(&mut gpu, size, Format::Rgba32Float, false, true)?;
    let display = Texture::new(&mut gpu, size, Format::Rgba8, false, true)?;
    let lut = Texture::volume(&mut gpu, 2, Format::Rgba16Float)?;
    let settings = SettingsRecord::new(
        &mut gpu,
        &[
            0.10, 0., 0., 0., 0., 0., 0., 0., 1.0, 1.0, 0.06, 0., 1., 1., 1., 1., 0., 0., 2., 0.,
        ],
    )?;
    let bloom = Bloom::new(&mut gpu, size)?;
    let mut graph = Graph::default();
    graph.import(Resource::Buffer(settings.buffer), Access::HostWrite);
    let mut rec = gpu.begin(Queue::Graphics);
    let bytes: Vec<_> = pixels
        .iter()
        .flatten()
        .flat_map(|v| half::f16::from_f32(*v).to_bits().to_le_bytes())
        .collect();
    let lut_bytes: Vec<_> = (0..8)
        .flat_map(|i| {
            [
                f32::from((i & 1) != 0),
                f32::from((i & 2) != 0),
                f32::from((i & 4) != 0),
                1.,
            ]
        })
        .flat_map(|v| half::f16::from_f32(v).to_bits().to_le_bytes())
        .collect();
    let staging = [
        transfer::upload(&mut gpu, &mut graph, &mut rec, hdr.image, &bytes)?,
        transfer::upload(&mut gpu, &mut graph, &mut rec, lut.image, &lut_bytes)?,
    ];
    bloom.record(&passes, &mut graph, &mut rec, &hdr, &settings)?;
    passes.tone(
        &mut graph,
        &mut rec,
        &ToneInputs {
            colour: &hdr,
            bloom: bloom.output(),
            lut: &lut,
            settings: &settings,
        },
        &linear,
    )?;
    passes.display(&mut graph, &mut rec, &linear, &display)?;
    let read = transfer::readback(
        &mut gpu,
        &mut graph,
        &mut rec,
        display.image,
        u64::from(size[0]) * u64::from(size[1]) * 4,
    )?;
    let point = gpu.submit(rec.finish(), &[])?;
    for b in staging {
        gpu.retire(Resource::Buffer(b))?;
    }
    gpu.wait(point)?;
    let bytes = gpu.read(read)?;
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut encoder = png::Encoder::new(std::fs::File::create(&output)?, size[0], size[1]);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    encoder.write_header()?.write_image_data(&bytes)?;
    println!(
        "Ara Metal: {}×{} stored HDR → bloom → tone → GPU display transfer → {}",
        size[0],
        size[1],
        output.display()
    );
    if window {
        show(&gpu, display.image, seconds)?;
    }
    gpu.assert_clean()?;
    bloom.retire(&mut gpu)?;
    passes.retire(&mut gpu)?;
    for t in [hdr, linear, display, lut] {
        gpu.retire(Resource::Image(t.image))?;
    }
    gpu.retire(Resource::Buffer(settings.buffer))?;
    gpu.retire(Resource::Buffer(read))?;
    gpu.collect()?;
    Ok(())
}
fn read_pfm(path: &Path) -> Result<([u32; 2], Vec<[f32; 4]>)> {
    use std::io::{BufRead, Read};
    let mut reader = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    ensure!(line.trim() == "PF", "expected RGB PFM");
    line.clear();
    reader.read_line(&mut line)?;
    let shape: Vec<u32> = line
        .split_whitespace()
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()?;
    ensure!(
        shape.len() == 2 && shape.iter().all(|v| *v > 0),
        "invalid PFM extent"
    );
    let size = [shape[0], shape[1]];
    line.clear();
    reader.read_line(&mut line)?;
    let scale: f32 = line.trim().parse()?;
    ensure!(scale.is_finite() && scale != 0., "invalid PFM scale");
    let count = (size[0] as usize)
        .checked_mul(size[1] as usize)
        .context("PFM extent overflow")?;
    let expected = count.checked_mul(12).context("PFM byte size overflow")?;
    let mut bytes = Vec::new();
    reader
        .take((expected as u64).saturating_add(1))
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() == expected, "PFM payload length mismatch");
    let mut pixels = vec![[0., 0., 0., 1.]; count];
    for (i, rgb) in bytes.as_chunks::<12>().0.iter().enumerate() {
        let at =
            (size[1] as usize - 1 - i / size[0] as usize) * size[0] as usize + i % size[0] as usize;
        for (c, v) in rgb.as_chunks::<4>().0.iter().enumerate() {
            let value = if scale < 0. {
                f32::from_le_bytes(*v)
            } else {
                f32::from_be_bytes(*v)
            } * scale.abs();
            ensure!(
                value.is_finite() && value.abs() <= 65504.,
                "PFM sample cannot be represented in RGBA16Float"
            );
            pixels[at][c] = value;
        }
    }
    Ok((size, pixels))
}
fn show(gpu: &GpuDevice, image: ara_gpu::Image, seconds: f32) -> Result<()> {
    use ara_window::{events::*, init::*, metal::*, video::*};
    use std::ptr::NonNull;
    // SAFETY: the example's main thread is the sole SDL video owner.
    unsafe {
        ensure!(SDL_Init(SDL_INIT_VIDEO), "SDL initialization failed");
    }
    struct Window {
        window: *mut SDL_Window,
        view: SDL_MetalView,
    }
    impl Drop for Window {
        fn drop(&mut self) {
            // SAFETY: these resources belong to this main-thread window; the presenter
            // is dropped before this guard, and null construction states are accepted.
            unsafe {
                if !self.view.is_null() {
                    SDL_Metal_DestroyView(self.view);
                }
                if !self.window.is_null() {
                    SDL_DestroyWindow(self.window);
                }
                SDL_Quit();
            }
        }
    }
    // SAFETY: SDL initialized on this thread; title is NUL terminated, sizes valid.
    let mut window = Window {
        window: unsafe {
            SDL_CreateWindow(
                c"Renderlab · ara-gpu · Metal · stored HDR".as_ptr(),
                960,
                600,
                SDL_WINDOW_METAL,
            )
        },
        view: std::ptr::null_mut(),
    };
    ensure!(!window.window.is_null(), "SDL window failed");
    // SAFETY: the window is live and Metal-capable until the guard is dropped.
    window.view = unsafe { SDL_Metal_CreateView(window.window) };
    ensure!(!window.view.is_null(), "SDL Metal view failed");
    // SAFETY: the live view owns a CAMetalLayer; the presenter is dropped first.
    let layer =
        NonNull::new(unsafe { SDL_Metal_GetLayer(window.view) }).context("no CAMetalLayer")?;
    // SAFETY: the layer remains owned by this window and is not used by another presenter.
    let mut presenter =
        unsafe { ara_gpu::metal::present::Presenter::from_layer(gpu, layer, [960, 600]) }?;
    let start = std::time::Instant::now();
    let mut frames = 0;
    while start.elapsed().as_secs_f32() < seconds {
        // SAFETY: events are pumped only by the owning main thread.
        unsafe {
            SDL_PumpEvents();
        }
        presenter.display(gpu, image)?;
        frames += 1;
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    ensure!(
        frames > 0 && presenter.skipped() < frames,
        "no Metal frame presented"
    );
    println!(
        "Presented {frames} Metal frames; skipped {}",
        presenter.skipped()
    );
    Ok(())
}
