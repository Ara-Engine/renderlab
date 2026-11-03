//! Finite registered-camera replay, with explicit full RLA and independent legal-selector controls.
#[path = "m4/cameras.rs"]
mod cameras;
#[path = "m4/diagnostics.rs"]
mod diagnostics;
#[allow(dead_code)]
#[path = "../viewer/src/prepared.rs"]
mod prepared;
#[path = "m4/selection_timing.rs"]
mod selection_timing;
#[path = "../support/validation.rs"]
mod validation;
use renderlab::{virtual_geometry::*, *};
use std::{
    collections::BTreeSet,
    fs,
    io::{BufWriter, Write},
    path::PathBuf,
    time::{Duration, Instant},
};
struct Options {
    cache: PathBuf,
    manifest: PathBuf,
    cameras: PathBuf,
    asset: Option<PathBuf>,
    output: PathBuf,
    mode: String,
    frames: usize,
    start: usize,
    warmup: usize,
    settle_final: usize,
    hold_pose: bool,
    page_log: bool,
    frame_period: Duration,
    repeats: usize,
    cpu_stride: usize,
    captures: BTreeSet<usize>,
    samples: SampleCount,
    stream: StreamOptions,
    validate: bool,
    view: ViewMode,
}
fn parse() -> Result<Options, Box<dyn std::error::Error>> {
    let mut o = Options {
        cache: "measurements/m4-cook/root-unit-a/cache".into(),
        manifest: "measurements/m4-cook/root-unit-a/closed-sphere.manifest".into(),
        cameras: "measurements/m4-cook/registration-initial/cameras.json".into(),
        asset: None,
        output: "measurements/m4-stream/manual".into(),
        mode: "stream".into(),
        frames: 631,
        start: 0,
        warmup: 0,
        settle_final: 0,
        hold_pose: false,
        page_log: false,
        frame_period: Duration::ZERO,
        repeats: 5,
        cpu_stride: 30,
        captures: BTreeSet::new(),
        samples: SampleCount::One,
        stream: StreamOptions::default(),
        validate: false,
        view: ViewMode::BaseColor,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--hold-pose" {
            o.hold_pose = true;
            continue;
        }
        if arg == "--page-log" {
            o.page_log = true;
            continue;
        }
        if arg == "--validate" {
            o.validate = true;
            continue;
        }
        let v = args.next().ok_or("missing argument value")?;
        match arg.as_str() {
            "--cache" => o.cache = v.into(),
            "--manifest" => o.manifest = v.into(),
            "--cameras" => o.cameras = v.into(),
            "--asset" => o.asset = Some(v.into()),
            "--output" => o.output = v.into(),
            "--mode" => o.mode = v,
            "--frames" => o.frames = v.parse()?,
            "--frame-period-ms" => {
                o.frame_period = Duration::try_from_secs_f64(v.parse::<f64>()? / 1000.)?
            }
            "--repeats" => o.repeats = v.parse()?,
            "--cpu-stride" => o.cpu_stride = v.parse()?,
            "--start" => o.start = v.parse()?,
            "--warmup" => o.warmup = v.parse()?,
            "--settle-final" => o.settle_final = v.parse()?,
            "--capture" => o.captures = v.split(',').map(str::parse).collect::<Result<_, _>>()?,
            "--geometry-bytes" => o.stream.geometry_bytes = v.parse()?,
            "--candidates" => o.stream.max_candidates = v.parse()?,
            "--output-clusters" => o.stream.max_clusters = v.parse()?,
            "--error-pixels" => o.stream.error_pixels = v.parse()?,
            "--io-delay-ms" => o.stream.io_delay = Duration::from_millis(v.parse()?),
            "--samples" => {
                o.samples = match v.as_str() {
                    "1" => SampleCount::One,
                    "4" => SampleCount::Four,
                    _ => return Err("samples must be 1 or 4".into()),
                }
            }
            "--view" => {
                o.view = match v.as_str() {
                    "color" => ViewMode::BaseColor,
                    "normal" => ViewMode::Normal,
                    "metallic" => ViewMode::Metallic,
                    "roughness" => ViewMode::Roughness,
                    _ => return Err("unknown view".into()),
                }
            }
            _ => return Err(format!("unknown argument {arg}").into()),
        }
    }
    match o.mode.as_str() {
        "stream" | "cpu" => {}
        "simple" => o.stream.selector = SelectorMode::SimpleReference,
        "full" => {
            if o.asset.is_none() {
                return Err("full mode requires original --asset RLA".into());
            }
        }
        _ => return Err("mode must be stream|simple|full|cpu".into()),
    }
    if o.frames == 0 || o.repeats == 0 || o.repeats > 100 || o.cpu_stride == 0 {
        return Err("frames must be positive".into());
    }
    Ok(o)
}
fn guard() -> Result<(), Box<dyn std::error::Error>> {
    if std::path::Path::new("D:/AraBuild/render-v3/quiet-window.lock").exists() {
        return Err("GPU quiet-window lock present".into());
    }
    let status=std::process::Command::new("powershell.exe").args(["-NoProfile","-Command","if (Get-Process -Name 'VALORANT-Win64-Shipping' -ErrorAction SilentlyContinue) { exit 9 }"]).creation_flags_hidden().status()?;
    if !status.success() {
        return Err("gameplay active; GPU work deferred".into());
    }
    Ok(())
}
trait Hidden {
    fn creation_flags_hidden(&mut self) -> &mut Self;
}
impl Hidden for std::process::Command {
    fn creation_flags_hidden(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            self.creation_flags(0x08000000);
        }
        self
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let o = parse()?;
    guard()?;
    let _validation = validation::begin(o.validate);
    let cameras = cameras::Cameras::load(&o.cameras)?;
    if o.start >= cameras.frames.len()
        || (!o.hold_pose && o.start + o.frames > cameras.frames.len())
    {
        return Err("frame range exceeds registered camera file".into());
    }
    if o.output.exists() {
        return Err("output directory already exists; preserve prior evidence".into());
    }
    fs::create_dir_all(&o.output)?;
    fs::write(
        o.output.join("run.txt"),
        format!(
            "profile={} oracle={} shadow={} profiling={} validation={} period_ms={} page_log={}\nargs={:?}\n",
            if cfg!(debug_assertions) {
                "dev"
            } else {
                "release"
            },
            cfg!(feature = "selection-oracle"),
            cfg!(feature = "selection-shadow"),
            cfg!(feature = "selection-profile"),
            o.validate,
            o.frame_period.as_secs_f64() * 1000.,
            o.page_log,
            std::env::args().collect::<Vec<_>>()
        ),
    )?;
    if o.mode == "cpu" {
        return selection_timing::run(&o, &cameras);
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
    let info = adapter.get_info();
    eprintln!("adapter={info:?} samples={:?}", o.samples);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
        ..Default::default()
    }))?;
    let mut r = Renderer::new_with_options(
        device,
        queue,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        Limits::default(),
        RendererOptions { samples: o.samples },
        include_str!("../shaders/scene.wgsl"),
    )?;
    let mut full = o
        .asset
        .as_deref()
        .filter(|_| o.mode == "full")
        .map(prepared::Prepared::load)
        .transpose()?;
    let mut virtual_asset = if full.is_none() {
        Some(VirtualAsset::open(&o.cache, &o.manifest, o.stream)?)
    } else {
        None
    };
    let mut virtual_scene = virtual_asset
        .as_ref()
        .map(VirtualScene::from_package)
        .transpose()?;
    let mut csv = fs::File::create(o.output.join("frames.csv"))?;
    writeln!(
        csv,
        "frame,phase,prep_us,encode_us,submit_us,projection_total_us,selection_total_us,admission_total_us,reads,uploads,geometry,peak,published,retiring,staging,pinned,payload,owned_buffers,metadata,scratch,cpu_ready,cut_clusters,candidates,visits,unresolved,max_error,candidate_debt,output_debt,residency_debt,selection_runs,io_queue_total_us,io_read_total_us,io_verify_total_us,failures,retries,evictions,retained_cuts,selection_calls,completion_total_us,resources_total_us,protection_total_us,stale_ready_total_us,make_room_total_us,upload_admission_total_us,requests_total_us,snapshot_total_us,dropped_ready,page_log_total_us"
    )?;
    let mut gpu = fs::File::create(o.output.join("gpu.csv"))?;
    writeln!(gpu, "frame,mesh_pass_us")?;
    let mut page_log = if o.page_log {
        virtual_asset
            .as_ref()
            .map(|a| {
                diagnostics::PageLog::new(&o.output.join("pages.csv"), a.package().pages.len())
            })
            .transpose()?
    } else {
        None
    };
    let mut tick = 0;
    let mut color = None;
    let mut accepted = 0;
    let start = Instant::now();
    let total = o.frames + o.warmup + o.settle_final;
    for n in 0..total {
        let frame_start = Instant::now();
        if n % 60 == 0 {
            guard()?;
        }
        let index = if n < o.warmup || o.hold_pose {
            o.start
        } else {
            o.start + (n - o.warmup).min(o.frames - 1)
        };
        let pose = &cameras.frames[index];
        let camera = pose.camera()?;
        if color
            .as_ref()
            .is_none_or(|t: &wgpu::Texture| t.width() != pose.width || t.height() != pose.height)
        {
            color = Some(r.device().create_texture(&wgpu::TextureDescriptor {
                label: Some("registered camera offscreen"),
                size: wgpu::Extent3d {
                    width: pose.width,
                    height: pose.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            }));
        }
        let color = color.as_ref().unwrap();
        let deadline = Instant::now() + Duration::from_secs(120);
        let report = loop {
            let report = if let Some(full) = &mut full {
                full.advance(&mut r)?;
                if full.ready() {
                    r.try_render_view(
                        color,
                        &camera,
                        &mut full.draws(Visibility::Unculled),
                        o.view,
                    )?
                } else {
                    None
                }
            } else {
                r.try_render_virtual(
                    color,
                    &camera,
                    virtual_asset.as_mut().unwrap(),
                    virtual_scene.as_mut().unwrap(),
                    o.view,
                )?
            };
            if let (Some(log), Some(asset), Some(scene)) =
                (&mut page_log, &virtual_asset, &virtual_scene)
            {
                log.sample(tick, index, asset, scene)?;
            }
            tick += 1;
            for sample in r.poll()?.gpu {
                writeln!(
                    gpu,
                    "{},{}",
                    sample.frame.0,
                    sample.mesh_pass.as_secs_f64() * 1e6
                )?;
            }
            if let Some(report) = report {
                break report;
            }
            if Instant::now() > deadline {
                return Err("frame/startup admission deadline exceeded".into());
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        if n < o.warmup {
            std::thread::sleep(o.frame_period.saturating_sub(frame_start.elapsed()));
            continue;
        }
        accepted += 1;
        let s = virtual_asset
            .as_ref()
            .map(|a| a.stats().clone())
            .unwrap_or_default();
        writeln!(
            csv,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            pose.frame,
            pose.phase,
            report.cpu.preparation.as_micros(),
            report.cpu.encode.as_micros(),
            report.cpu.submit.as_micros(),
            s.projection.as_micros(),
            s.selection.as_micros(),
            s.admission.as_micros(),
            s.reads,
            s.uploads,
            s.geometry_bytes,
            s.geometry_peak,
            s.published_bytes,
            s.retiring_bytes,
            s.staging_bytes,
            s.pinned_bytes,
            s.payload_bytes,
            s.owned_gpu_geometry_bytes,
            s.metadata_bytes,
            s.scratch_bound_bytes,
            s.cpu_ready_bytes,
            s.cut_clusters,
            s.candidates,
            s.cluster_visits,
            s.unresolved_groups,
            s.maximum_denied_error,
            s.candidate_debt,
            s.output_debt,
            s.residency_debt,
            s.selection_runs,
            s.io_queue.as_micros(),
            s.io_read.as_micros(),
            s.io_verify.as_micros(),
            s.failures,
            s.retries,
            s.evictions,
            s.retained_cuts,
            s.selection_calls,
            s.completion.as_micros(),
            s.resources.as_micros(),
            s.protection.as_micros(),
            s.stale_ready.as_micros(),
            s.make_room.as_micros(),
            s.upload_admission.as_micros(),
            s.requests.as_micros(),
            s.snapshot.as_micros(),
            s.dropped_ready,
            page_log.as_ref().map_or(0, |l| l.overhead.as_micros())
        )?;
        let final_pose = o.start + o.frames - 1;
        let final_capture = o.hold_pose || (o.settle_final > 0 && index == final_pose);
        if o.captures.contains(&index) && (!final_capture || n == total - 1) {
            let capture_start = Instant::now();
            let mut capture = loop {
                if let Some(c) = r.try_capture(color)? {
                    break c;
                }
                r.poll()?;
                std::thread::sleep(Duration::from_millis(1));
            };
            let rgba = loop {
                r.poll()?;
                if let Some(b) = capture.try_read()? {
                    break b;
                }
                if Instant::now() > deadline {
                    return Err("capture deadline".into());
                }
                std::thread::sleep(Duration::from_millis(1));
            };
            let readback_wall = capture_start.elapsed();
            let write_start = Instant::now();
            fs::write(o.output.join(format!("frame-{index:04}.rgba8")), &rgba)?;
            let mut ppm = BufWriter::new(fs::File::create(
                o.output.join(format!("frame-{index:04}.ppm")),
            )?);
            write!(ppm, "P6\n{} {}\n255\n", pose.width, pose.height)?;
            for p in rgba.chunks_exact(4) {
                ppm.write_all(&p[..3])?;
            }
            ppm.flush()?;
            eprintln!(
                "capture pose={index} explicit_readback_wall_us={} image_write_wall_us={}",
                readback_wall.as_micros(),
                write_start.elapsed().as_micros()
            );
        }
        std::thread::sleep(o.frame_period.saturating_sub(frame_start.elapsed()));
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    while r.stats().flights > 0 {
        for sample in r.poll()?.gpu {
            writeln!(
                gpu,
                "{},{}",
                sample.frame.0,
                sample.mesh_pass.as_secs_f64() * 1e6
            )?;
        }
        if Instant::now() > deadline {
            return Err("final completion deadline".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    fs::write(
        o.output.join("summary.txt"),
        format!(
            "mode={} frames={accepted} wall={:?}\nstream={:#?}\nrenderer={:#?}\nadapter={info:?}\n",
            o.mode,
            start.elapsed(),
            virtual_asset.as_ref().map(|a| a.stats()),
            r.stats()
        ),
    )?;
    if let (Some(asset), Some(scene)) = (&virtual_asset, &virtual_scene) {
        let index = if o.hold_pose {
            o.start
        } else {
            o.start + o.frames - 1
        };
        let pose = &cameras.frames[index];
        diagnostics::groups(&o.output.join("groups.csv"), asset, scene, pose, o.stream)?;
    }

    eprintln!(
        "replay frames={accepted} output={} stream={:?}",
        o.output.display(),
        virtual_asset.as_ref().map(|a| a.stats())
    );
    Ok(())
}
