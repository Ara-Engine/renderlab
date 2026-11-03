#[path = "../../support/limits.rs"]
mod policy;
#[path = "../../support/runtime_policy.rs"]
mod runtime_policy;
use renderlab::asset;
#[path = "../../examples/m4/cameras.rs"]
mod cameras;
mod fixture;
#[path = "../../support/hdr_files.rs"]
mod hdr_files;
#[path = "../../support/lighting_controls.rs"]
mod lighting_settings;
mod prepared;
mod surface_probe;
use renderlab::virtual_geometry::{
    PreparationState, SelectorMode, StreamOptions, VirtualAsset, VirtualScene,
};
#[path = "../../support/validation.rs"]
mod validation;
use fixture::Scene;
use renderlab::{
    Draw, FrameId, Instances, Limits, Mesh, Renderer, RendererOptions, Revision, SampleCount,
    ViewMode, Visibility, wgpu,
};
use std::{
    collections::{HashSet, VecDeque},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

const TITLE: &str = "Renderlab | arrows orbit | WASD move | R reset | Space motion | C cull | N near | 1-5 views | Esc exit";
const RETRY_INTERVAL: Duration = Duration::from_millis(8);

struct PendingResize {
    size: winit::dpi::PhysicalSize<u32>,
    requested: Instant,
    deferrals: u64,
}
impl PendingResize {
    fn new(size: winit::dpi::PhysicalSize<u32>) -> Self {
        Self {
            size,
            requested: Instant::now(),
            deferrals: 0,
        }
    }
}

struct Options {
    policy: policy::Policy,
    shader: PathBuf,
    lighting: Option<PathBuf>,
    capture_hdr: Option<PathBuf>,
    frames: Option<u64>,
    seconds: Option<f64>,
    timestamps: bool,
    log_frames: bool,
    trace_surface: bool,
    validate: bool,
    background: bool,
    instances: u32,
    asset: Option<PathBuf>,
    virtual_manifest: Option<PathBuf>,
    progressive: bool,
    cache: PathBuf,
    cameras: Option<PathBuf>,
    interactive_camera: bool,
    stream: StreamOptions,
    samples: SampleCount,
    anisotropy_clamp: u16,
    view: ViewMode,
}
impl Options {
    fn parse() -> Result<Self, String> {
        let (policy, args) = policy::Policy::args(std::env::args().skip(1).collect())?;
        let stream = runtime_policy::stream(&policy)?;
        let mut options = Self {
            policy,
            lighting: None,
            capture_hdr: None,
            shader: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../shaders/scene.wgsl"),
            frames: None,
            seconds: None,
            timestamps: true,
            log_frames: false,
            trace_surface: false,
            validate: false,
            background: false,
            instances: 2,
            asset: None,
            virtual_manifest: None,
            progressive: false,
            cache: "measurements/m4-cook/root-unit-a/cache".into(),
            cameras: None,
            interactive_camera: false,
            stream,
            samples: SampleCount::One,
            anisotropy_clamp: 1,
            view: ViewMode::BaseColor,
        };
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--progressive" => options.progressive = true,
                "--interactive-camera" => options.interactive_camera = true,
                "--virtual-manifest" => {
                    options.virtual_manifest = Some(args.next().ok_or("missing manifest")?.into())
                }
                "--cache" => options.cache = args.next().ok_or("missing cache")?.into(),
                "--cameras" => options.cameras = Some(args.next().ok_or("missing cameras")?.into()),
                "--geometry-bytes" => {
                    options.stream.geometry_bytes = args
                        .next()
                        .ok_or("missing geometry bytes")?
                        .parse()
                        .map_err(|_| "invalid geometry bytes")?
                }
                "--candidates" => {
                    options.stream.max_candidates = args
                        .next()
                        .ok_or("missing candidates")?
                        .parse()
                        .map_err(|_| "invalid candidates")?
                }
                "--output-clusters" => {
                    options.stream.max_clusters = args
                        .next()
                        .ok_or("missing output clusters")?
                        .parse()
                        .map_err(|_| "invalid output clusters")?
                }
                "--error-pixels" => {
                    options.stream.error_pixels = args
                        .next()
                        .ok_or("missing pixel error")?
                        .parse()
                        .map_err(|_| "invalid error")?
                }
                "--io-delay-ms" => {
                    options.stream.io_delay = Duration::from_millis(
                        args.next()
                            .ok_or("missing delay")?
                            .parse()
                            .map_err(|_| "invalid delay")?,
                    )
                }
                "--selector" => {
                    options.stream.selector = match args.next().as_deref() {
                        Some("baseline") => SelectorMode::Baseline,
                        Some("simple") => SelectorMode::SimpleReference,
                        _ => return Err("selector requires baseline|simple".into()),
                    }
                }
                "--asset" => {
                    options.asset = Some(args.next().ok_or("missing prepared asset path")?.into())
                }
                "--samples" => {
                    options.samples = match args.next().as_deref() {
                        Some("1") => SampleCount::One,
                        Some("4") => SampleCount::Four,
                        _ => return Err("--samples requires 1 or 4".into()),
                    }
                }
                "--anisotropy" => {
                    options.anisotropy_clamp = args
                        .next()
                        .ok_or("missing anisotropy")?
                        .parse()
                        .map_err(|_| "invalid anisotropy")?;
                }
                "--view" => {
                    options.view = match args.next().as_deref() {
                        Some("color") => ViewMode::BaseColor,
                        Some("normal") => ViewMode::Normal,
                        Some("metallic") => ViewMode::Metallic,
                        Some("roughness") => ViewMode::Roughness,
                        Some("lit") => ViewMode::Lit,
                        _ => {
                            return Err(
                                "--view requires color|normal|metallic|roughness|lit".into()
                            );
                        }
                    }
                }
                "--instances" => {
                    options.instances = args
                        .next()
                        .ok_or("missing --instances value")?
                        .parse()
                        .map_err(|_| "invalid instances count")?
                }
                "--frames" => {
                    options.frames = Some(
                        args.next()
                            .ok_or("missing --frames value")?
                            .parse()
                            .map_err(|_| "invalid frame count")?,
                    )
                }
                "--seconds" => {
                    options.seconds = Some(
                        args.next()
                            .ok_or("missing --seconds value")?
                            .parse()
                            .map_err(|_| "invalid seconds")?,
                    )
                }
                "--lighting" => {
                    options.lighting = Some(args.next().ok_or("missing lighting JSON")?.into())
                }
                "--capture-hdr" => {
                    options.capture_hdr =
                        Some(args.next().ok_or("missing HDR output prefix")?.into())
                }
                "--shader" => options.shader = args.next().ok_or("missing shader path")?.into(),
                "--no-timestamps" => options.timestamps = false,
                "--log-frames" => options.log_frames = true,
                "--trace-surface" => options.trace_surface = true,
                "--validate" => options.validate = true,
                "--background" => options.background = true,
                _ if !arg.starts_with('-') => options.shader = arg.into(),
                _ => return Err(format!("unknown argument {arg}")),
            }
        }
        if options.asset.is_some() && options.virtual_manifest.is_some() {
            return Err("choose --asset or --virtual-manifest".into());
        }
        if options.instances < 2
            || options.frames == Some(0)
            || options.seconds.is_some_and(|s| !s.is_finite() || s <= 0.0)
        {
            return Err("limits must be positive".into());
        }
        if options.capture_hdr.is_some() && options.view != ViewMode::Lit {
            return Err("--capture-hdr requires --view lit".into());
        }
        options.stream.validate().map_err(|e| e.to_string())?;
        Ok(options)
    }
}
struct Marker {
    revision: Revision,
    origin: Instant,
    file_age: Duration,
    startup: bool,
    presented: bool,
    completed: bool,
}
impl Marker {
    fn emit(&self, kind: &str, frame: FrameId, at: Instant) {
        let category = if self.startup {
            "startup"
        } else {
            "shader_write"
        };
        let elapsed = at.saturating_duration_since(self.origin) + self.file_age;
        eprintln!(
            "{category}_to_{kind}_proxy_ms={:.3} frame={} revision={} definition=m1_async_not_scanout",
            elapsed.as_secs_f64() * 1000.0,
            frame.0,
            self.revision.0
        );
    }
}
struct View {
    surface_probe: surface_probe::SurfaceProbe,
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    pending_frame: Option<wgpu::SurfaceTexture>,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    meshes: Vec<Mesh>,
    asset: Option<prepared::Prepared>,
    virtual_asset: Option<(VirtualAsset, VirtualScene)>,
    cameras: Option<cameras::Cameras>,
    view_mode: ViewMode,
    title: String,
    instances: Vec<Instances>,
    visibility: Visibility,
    scene: Scene,
    keys: HashSet<KeyCode>,
    input_stamp: Option<Instant>,
    applied_input_stamp: Option<Instant>,
    source: String,
    shader_rejected: bool,
    shader_check: Instant,
    lighting_file: lighting_settings::LightingFile,
    hdr_capture: Option<renderlab::Capture>,
    hdr_written: bool,
    markers: VecDeque<Marker>,
    last: Instant,
    pending_size: Option<PendingResize>,
    retry_redraw_at: Instant,
    size: winit::dpi::PhysicalSize<u32>,
    configured: bool,
    occluded: bool,
    presented: u64,
    finish_at: Option<Instant>,
    startup: Instant,
    preparation_ticks: u64,
    first_cover: bool,
    preparation_status: Option<renderlab::virtual_geometry::ViewPreparation>,
}
impl Drop for View {
    fn drop(&mut self) {
        self.pending_frame = None;
    }
}
impl View {
    fn set_title(&mut self, title: String) {
        if self.title != title {
            self.window.set_title(&title);
            self.title = title;
        }
    }
    fn create(
        events: &ActiveEventLoop,
        options: &Options,
        start: Instant,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let asset = options
            .asset
            .as_deref()
            .map(|path| {
                prepared::Prepared::load_with_limits(
                    path,
                    options.policy.prepared(),
                    options
                        .policy
                        .size("viewer", "max_placement_slots", 1_000_000),
                )
            })
            .transpose()?;
        let cameras = options
            .cameras
            .as_deref()
            .map(cameras::Cameras::load)
            .transpose()?;
        let virtual_asset = options
            .virtual_manifest
            .as_deref()
            .map(|manifest| -> Result<_, Box<dyn std::error::Error>> {
                let asset = VirtualAsset::open(&options.cache, manifest, options.stream)?;
                let scene = VirtualScene::from_package(&asset)?;
                Ok((asset, scene))
            })
            .transpose()?;
        if virtual_asset.is_some() {
            eprintln!("virtual_metadata_ready_us={}", start.elapsed().as_micros());
        }
        let window = Arc::new(
            events.create_window(
                Window::default_attributes()
                    .with_title(TITLE)
                    .with_active(!options.background)
                    .with_inner_size(winit::dpi::PhysicalSize::new(
                        cameras.as_ref().map_or(960, |c| c.frames[0].width),
                        cameras.as_ref().map_or(640, |c| c.frames[0].height),
                    )),
            )?,
        );
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            flags: if options.validate {
                wgpu::InstanceFlags::debugging()
            } else {
                wgpu::InstanceFlags::default()
            },
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance.create_surface(window.clone())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))?;
        eprintln!(
            "adapter={:?} scene={} samples={} view={:?} quality={} debug_build={}",
            adapter.get_info(),
            if options.virtual_manifest.is_some() {
                "virtual_asset"
            } else if options.asset.is_some() {
                "prepared_asset"
            } else {
                "cubes_triangle"
            },
            options.samples as u32,
            options.view,
            if options.view == ViewMode::Lit {
                "GGX_Lambert_constant_environment_optional_hard_shadows_exposed_RGBA16F_resolve_Reinhard_no_normal_maps_no_GI"
            } else {
                "unlit_depth32float"
            },
            cfg!(debug_assertions)
        );
        let required_features = if options.timestamps {
            adapter.features() & wgpu::Features::TIMESTAMP_QUERY
        } else {
            wgpu::Features::empty()
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                required_features,
                ..Default::default()
            }))?;
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("surface has no supported configuration")?;
        config.format = surface
            .get_capabilities(&adapter)
            .formats
            .into_iter()
            .find(|f| f.is_srgb())
            .ok_or("surface has no sRGB format")?;
        config.present_mode = wgpu::PresentMode::Fifo;
        let source = std::fs::read_to_string(&options.shader)?;
        let renderer_limits = runtime_policy::renderer(
            &options.policy,
            Limits {
                max_instances: options
                    .instances
                    .max(1024)
                    .max(asset.as_ref().map_or(1, |a| a.max_instances())),
                ..Default::default()
            },
        );
        eprintln!(
            "effective_limits {}",
            runtime_policy::json(&options.policy, options.stream, renderer_limits)
        );
        let mut renderer = Renderer::new_with_options(
            device,
            queue,
            config.format,
            renderer_limits,
            RendererOptions {
                samples: options.samples,
                anisotropy_clamp: options.anisotropy_clamp,
            },
            &source,
        )?;
        renderer.set_lighting(&lighting_settings::default_lighting())?;
        let mut lighting_file = lighting_settings::LightingFile::default();
        if let Some(path) = &options.lighting {
            lighting_file.poll(path, &mut renderer);
        }
        eprintln!(
            "timing_status={:?} measurement_definition=m1_async_not_m0",
            renderer.stats().timing
        );
        if virtual_asset.is_some() {
            eprintln!(
                "virtual_device_renderer_ready_us={}",
                start.elapsed().as_micros()
            );
        }
        Ok(Self {
            surface_probe: surface_probe::SurfaceProbe::new(options.trace_surface),
            window,
            surface,
            pending_frame: None,
            config,
            renderer,
            meshes: Vec::new(),
            asset,
            virtual_asset,
            cameras,
            view_mode: options.view,
            title: TITLE.into(),
            instances: Vec::new(),
            visibility: Visibility::Frustum,
            scene: Scene::default(),
            keys: HashSet::new(),
            input_stamp: None,
            applied_input_stamp: None,
            source,
            shader_rejected: false,
            shader_check: Instant::now(),
            lighting_file,
            hdr_capture: None,
            hdr_written: false,
            markers: VecDeque::from([Marker {
                revision: Revision(1),
                origin: start,
                file_age: Duration::ZERO,
                startup: true,
                presented: false,
                completed: false,
            }]),
            last: Instant::now(),
            pending_size: Some(PendingResize::new(size)),
            retry_redraw_at: Instant::now(),
            size,
            configured: false,
            occluded: false,
            presented: 0,
            finish_at: None,
            startup: start,
            preparation_ticks: 0,
            first_cover: false,
            preparation_status: None,
        })
    }
    fn wake(&mut self, options: &Options) -> Result<bool, Box<dyn std::error::Error>> {
        let probe = self.surface_probe.begin("poll");
        let events = self.renderer.poll();
        probe.end(if events.is_ok() { "ok" } else { "error" });
        let events = events?;
        for completion in events.completed {
            if let Some(frame) = completion.frame {
                for marker in &mut self.markers {
                    if marker.revision == completion.revision && !marker.completed {
                        marker.emit("completion_observed", frame, completion.observed_at);
                        marker.completed = true;
                    }
                }
            }
        }
        if options.log_frames {
            for sample in events.gpu {
                if let Some(present) = sample.present_pass {
                    eprintln!(
                        "gpu_hdr_present_pass_ms={:.6} frame={} revision={} boundary=present_pass_only_excludes_acquire_present_uploads",
                        present.as_secs_f64() * 1e3,
                        sample.frame.0,
                        sample.revision.0
                    );
                }
                if let Some(shadow) = sample.shadow_generation {
                    eprintln!(
                        "gpu_shadow_generation_ms={:.6} gpu_render_span_ms={:.6} frame={} shadow_filtering=inside_mesh_pass",
                        shadow.as_secs_f64() * 1000.0,
                        sample.render_span.as_secs_f64() * 1000.0,
                        sample.frame.0
                    );
                }
                eprintln!(
                    "gpu_mesh_pass_ms={:.6} frame={} revision={}",
                    sample.mesh_pass.as_secs_f64() * 1000.0,
                    sample.frame.0,
                    sample.revision.0
                );
            }
            if events.dropped > 0 {
                eprintln!("events_dropped={}", events.dropped);
            }
        }
        self.markers.retain(|m| !m.completed || !m.presented);
        if let Some(capture) = &mut self.hdr_capture {
            let info = capture.info();
            if let Some(bytes) = capture.try_read()? {
                hdr_files::write(
                    options.capture_hdr.as_ref().unwrap(),
                    info,
                    &bytes,
                    serde_json::json!({"shader_path":options.shader,"lighting_path":options.lighting,"asset_path":options.asset}),
                )?;
                self.hdr_capture = None;
                self.hdr_written = true;
            }
        }
        if let Some(finish) = self.finish_at {
            if self.renderer.stats().flights == 0 {
                return Ok(true);
            }
            if finish.elapsed() > Duration::from_secs(5) {
                return Err("nonblocking final completion deadline exceeded".into());
            }
            return Ok(false);
        }
        // Do not refill the queue while a surface reconfiguration is draining it.
        if self.pending_size.is_none() {
            if let Some(asset) = &mut self.asset {
                asset.advance(&mut self.renderer)?;
            }
            if self.asset.is_none() && self.virtual_asset.is_none() && self.meshes.len() < 2 {
                fixture::upload_next(&mut self.renderer, &mut self.meshes)?;
            }
            if self.asset.is_none()
                && self.virtual_asset.is_none()
                && self.meshes.len() == 2
                && self.instances.len() < 2
            {
                fixture::upload_instances_next(
                    &mut self.renderer,
                    &mut self.instances,
                    options.instances,
                )?;
            }
        }
        if self.shader_check.elapsed() >= Duration::from_millis(50) {
            self.shader_check = Instant::now();
            if let Some(path) = &options.lighting {
                self.lighting_file.poll(path, &mut self.renderer);
            }
            if let Ok(source) = std::fs::read_to_string(&options.shader)
                && source != self.source
            {
                self.source = source;
                let origin = Instant::now();
                let file_age = std::fs::metadata(&options.shader)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|s| SystemTime::now().duration_since(s).ok())
                    .unwrap_or_default();
                match self.renderer.reload(&self.source) {
                    Ok(report) => {
                        self.shader_rejected = false;
                        eprintln!(
                            "reload_compile_ms={:.3} revision={}",
                            report.compile.as_secs_f64() * 1000.0,
                            report.revision.0
                        );
                        if self.markers.len() == 16 {
                            self.markers.pop_front();
                            eprintln!("proxy_marker_dropped=1");
                        }
                        self.markers.push_back(Marker {
                            revision: report.revision,
                            origin,
                            file_age,
                            startup: false,
                            presented: false,
                            completed: false,
                        });
                        self.set_title(TITLE.into());
                    }
                    Err(renderlab::ReloadError::Rejected {
                        diagnostic,
                        compile,
                    }) => {
                        self.shader_rejected = true;
                        eprintln!(
                            "reload_rejected compile_ms={:.3} diagnostic={diagnostic}",
                            compile.as_secs_f64() * 1000.0
                        );
                        self.set_title("Renderlab | shader rejected; last valid pipeline active | see terminal".into());
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
        let dt = self.last.elapsed().as_secs_f32().min(0.1);
        self.last = Instant::now();
        let axis = |positive, negative| {
            f32::from(self.keys.contains(&positive)) - f32::from(self.keys.contains(&negative))
        };
        self.scene.yaw += axis(KeyCode::ArrowRight, KeyCode::ArrowLeft) * dt;
        self.scene.elevation = (self.scene.elevation
            + axis(KeyCode::ArrowUp, KeyCode::ArrowDown) * dt)
            .clamp(-1.4, 1.4);
        self.scene.translation.x += axis(KeyCode::KeyD, KeyCode::KeyA) * dt;
        self.scene.translation.z += axis(KeyCode::KeyS, KeyCode::KeyW) * dt;
        if self.scene.animated {
            self.scene.phase += dt * 0.8;
        }
        if let Some(input) = self.input_stamp.take() {
            self.applied_input_stamp.get_or_insert(input);
        }
        Ok(false)
    }
    fn redraw(&mut self, options: &Options) -> Result<(), Box<dyn std::error::Error>> {
        if self.finish_at.is_some()
            || self.size.width == 0
            || self.size.height == 0
            || self.occluded
        {
            return Ok(());
        }
        let mut configure_ms = 0.0;
        if let Some(pending) = &mut self.pending_size {
            let probe = self.surface_probe.begin("release_frame");
            self.pending_frame = None;
            probe.end("returned");
            let flights = self.renderer.stats().flights;
            if flights != 0 {
                if pending.deferrals == 0 {
                    eprintln!(
                        "configure_deferred width={} height={} flights={flights}",
                        pending.size.width, pending.size.height
                    );
                }
                pending.deferrals += 1;
                return Ok(());
            }
            let pending = self.pending_size.take().unwrap();
            self.config.width = pending.size.width;
            self.config.height = pending.size.height;
            let wait_ms = pending.requested.elapsed().as_secs_f64() * 1000.0;
            eprintln!(
                "configure_begin width={} height={} flights=0 pending_frame=false wait_ms={wait_ms:.3} deferrals={}",
                pending.size.width, pending.size.height, pending.deferrals
            );
            let probe = self.surface_probe.begin("configure");
            let clock = Instant::now();
            self.surface.configure(self.renderer.device(), &self.config);
            configure_ms = clock.elapsed().as_secs_f64() * 1000.0;
            probe.end("returned");
            self.configured = true;
            eprintln!(
                "configure_ms={configure_ms:.3} width={} height={}",
                pending.size.width, pending.size.height
            );
        }
        if !self.configured
            || (self.virtual_asset.is_none()
                && self
                    .asset
                    .as_ref()
                    .map_or(self.meshes.len() < 2 || self.instances.len() < 2, |a| {
                        !a.ready()
                    }))
        {
            return Ok(());
        }
        let registered_camera = self
            .cameras
            .as_ref()
            .map(|c| {
                let index = if options.interactive_camera {
                    0
                } else {
                    (self.presented as usize).min(c.frames.len() - 1)
                };
                let pose = &c.frames[index];
                if options.log_frames {
                    eprintln!("registered_pose={} phase={}", pose.frame, pose.phase);
                }
                if options.interactive_camera {
                    let defaults = Scene::default();
                    let yaw = self.scene.yaw - defaults.yaw;
                    let pitch = self.scene.elevation - defaults.elevation;
                    if yaw == 0.0 && pitch == 0.0 && self.scene.translation == renderlab::Vec3::ZERO
                    {
                        return pose.camera();
                    }
                    let up = renderlab::Vec3::from_array(pose.up).normalize();
                    let target = renderlab::Vec3::from_array(pose.target) + self.scene.translation;
                    let offset = renderlab::Vec3::from_array(pose.eye)
                        - renderlab::Vec3::from_array(pose.target);
                    let radius = offset.length();
                    let vertical = offset.dot(up);
                    let elevation =
                        ((vertical / radius).clamp(-1.0, 1.0).asin() + pitch).clamp(-1.4, 1.4);
                    let horizontal = (offset - up * vertical).normalize();
                    let direction =
                        renderlab::Mat4::from_axis_angle(up, yaw).transform_vector3(horizontal);
                    let eye =
                        target + radius * (direction * elevation.cos() + up * elevation.sin());
                    renderlab::Camera::look_at(
                        eye,
                        target,
                        up,
                        pose.fov_y_radians,
                        pose.near,
                        pose.far,
                    )
                } else {
                    pose.camera()
                }
            })
            .transpose()?;
        let virtual_camera = registered_camera.unwrap_or_else(|| self.scene.camera(2));
        if self.presented == 0
            && options.progressive
            && let Some((asset, scene)) = &mut self.virtual_asset
            && !scene.ready(asset)
        {
            let status =
                asset.prepare_view(&mut self.renderer, scene, &virtual_camera, self.size.height)?;
            self.preparation_ticks += 1;
            self.preparation_status = Some(status);
            if let PreparationState::Failed(page) = status.state {
                return Err(format!(
                    "initial progressive cover failed: {page:?}: {:?}",
                    asset.failure(page)
                )
                .into());
            }
            if !self.shader_rejected {
                self.set_title("Renderlab | preparing minimum cover".into());
            }
            return Ok(());
        }
        if self.presented == 0
            && !options.progressive
            && let Some((asset, scene)) = &mut self.virtual_asset
        {
            let clock = Instant::now();
            let status =
                asset.prepare_view(&mut self.renderer, scene, &virtual_camera, self.size.height)?;
            self.preparation_ticks += 1;
            if status.cover_ready && !self.first_cover {
                self.first_cover = true;
                eprintln!(
                    "virtual_first_cover_us={} ticks={}",
                    self.startup.elapsed().as_micros(),
                    self.preparation_ticks
                );
            }
            if options.log_frames {
                eprintln!(
                    "virtual_preparation_tick={} advance_us={} status={status:?} stream={:?} renderer={:?}",
                    self.preparation_ticks,
                    clock.elapsed().as_micros(),
                    asset.stats(),
                    self.renderer.stats()
                );
            }
            let changed = self.preparation_status != Some(status);
            self.preparation_status = Some(status);
            match status.state {
                PreparationState::Pending => {
                    if !self.shader_rejected {
                        self.set_title("Renderlab | preparing first view".into());
                    }
                    return Ok(());
                }
                PreparationState::Failed(page) => {
                    return Err(format!(
                        "initial virtual cover failed: {page:?}: {:?}",
                        asset.failure(page)
                    )
                    .into());
                }
                PreparationState::Degraded => {
                    if changed {
                        eprintln!("virtual_initial_degraded={status:?}");
                    }
                    if !self.shader_rejected {
                        self.set_title("Renderlab | initial view limited by capacity or failed pages; see terminal".into());
                    }
                }
                PreparationState::Settled => {
                    if !self.shader_rejected {
                        self.set_title(TITLE.into());
                    }
                }
            }
        }
        let probe = self.surface_probe.begin(if self.pending_frame.is_some() {
            "reuse_frame"
        } else {
            "acquire"
        });
        let acquire = Instant::now();
        let result = self.pending_frame.take().map_or_else(
            || self.surface.get_current_texture(),
            wgpu::CurrentSurfaceTexture::Success,
        );
        let acquire_ms = acquire.elapsed().as_secs_f64() * 1000.0;
        probe.end(match &result {
            wgpu::CurrentSurfaceTexture::Success(_) => "success",
            wgpu::CurrentSurfaceTexture::Suboptimal(_) => "suboptimal",
            wgpu::CurrentSurfaceTexture::Outdated => "outdated",
            wgpu::CurrentSurfaceTexture::Timeout => "timeout",
            wgpu::CurrentSurfaceTexture::Occluded => "occluded",
            wgpu::CurrentSurfaceTexture::Lost => "lost",
            wgpu::CurrentSurfaceTexture::Validation => "validation",
        });
        let (frame, suboptimal) = match result {
            wgpu::CurrentSurfaceTexture::Success(frame) => (frame, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.pending_size = Some(PendingResize::new(self.size));
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                return Err(
                    "surface lost; restart the viewer to recreate its surface and device".into(),
                );
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                self.renderer.poll()?;
                return Err("surface validation failed; restart required".into());
            }
        };
        let (report, caller_ms, render_ms) = if let Some((asset, scene)) = &mut self.virtual_asset {
            let caller_start = Instant::now();
            let camera = virtual_camera;
            let caller_ms = caller_start.elapsed().as_secs_f64() * 1000.;
            let render_start = Instant::now();
            let report = if self.presented == 0 && !options.progressive {
                self.renderer.try_render_prepared_virtual(
                    &frame.texture,
                    &camera,
                    asset,
                    scene,
                    self.view_mode,
                )?
            } else {
                self.renderer.try_render_virtual(
                    &frame.texture,
                    &camera,
                    asset,
                    scene,
                    self.view_mode,
                )?
            };
            if self.presented == 0 && options.progressive {
                let status = scene.preparation(asset, &camera, frame.texture.height())?;
                if let PreparationState::Failed(page) = status.state {
                    return Err(format!(
                        "initial progressive cover failed: {page:?}: {:?}",
                        asset.failure(page)
                    )
                    .into());
                }
                self.preparation_status = Some(status);
            }
            if report.is_some() && self.presented == 0 {
                if !self.first_cover {
                    self.first_cover = true;
                    eprintln!(
                        "virtual_first_cover_us={} ticks={}",
                        self.startup.elapsed().as_micros(),
                        self.preparation_ticks
                    );
                }
                eprintln!(
                    "virtual_first_scene_submitted_us={} ticks={} progressive={} status={:?} stream={:?} renderer={:?}",
                    self.startup.elapsed().as_micros(),
                    self.preparation_ticks,
                    options.progressive,
                    self.preparation_status,
                    asset.stats(),
                    self.renderer.stats()
                );
            }
            if options.log_frames {
                eprintln!("virtual_stats={:?}", asset.stats());
            }
            (
                report,
                caller_ms,
                render_start.elapsed().as_secs_f64() * 1000.,
            )
        } else {
            let caller_start = Instant::now();
            let (camera, mut draws) = if let Some(asset) = &mut self.asset {
                asset.animate(self.scene.phase)?;
                let camera = asset.camera(
                    self.scene.yaw,
                    self.scene.elevation,
                    self.scene.translation,
                    self.scene.close_view,
                    self.config.width as f32 / self.config.height as f32,
                )?;
                (
                    registered_camera.unwrap_or(camera),
                    asset.draws(self.visibility),
                )
            } else {
                self.instances[0].set(0, self.scene.first_model(options.instances))?;
                let (cubes, triangle) = self.instances.split_at_mut(1);
                (
                    registered_camera.unwrap_or_else(|| self.scene.camera(options.instances)),
                    vec![
                        Draw::new(&self.meshes[0], &mut cubes[0]).with_visibility(self.visibility),
                        Draw::new(&self.meshes[1], &mut triangle[0])
                            .with_visibility(self.visibility),
                    ],
                )
            };

            let caller_ms = caller_start.elapsed().as_secs_f64() * 1000.;
            let render_start = Instant::now();
            let result = self.renderer.try_render_view(
                &frame.texture,
                &camera,
                &mut draws,
                self.view_mode,
            )?;
            drop(draws);
            (
                result,
                caller_ms,
                render_start.elapsed().as_secs_f64() * 1000.,
            )
        };
        if options.log_frames {
            eprintln!(
                "caller_ms={caller_ms:.4} render_whole_ms={render_ms:.4} accepted={}",
                report.is_some()
            );
        }
        if let Some(report) = report {
            if options.capture_hdr.is_some()
                && !self.hdr_written
                && self.hdr_capture.is_none()
                && self.view_mode == ViewMode::Lit
            {
                self.hdr_capture = self.renderer.try_capture_hdr()?;
            }
            if options.log_frames {
                eprintln!(
                    "lighting_stats={:?} shadows={:?} lighting_uploads={:?} shadow_prepare_ms={:.6} shadow_upload_ms={:.6} shadow_encode_ms={:.6}",
                    self.renderer.lighting_stats(),
                    self.renderer.shadow_stats(),
                    report.uploads,
                    report.cpu.shadow_preparation.as_secs_f64() * 1000.0,
                    report.cpu.shadow_upload.as_secs_f64() * 1000.0,
                    report.cpu.shadow_encode.as_secs_f64() * 1000.0
                );
            }
            self.set_title(format!(
                "{TITLE} | {:?} {}x {:?} visible {}/{}{}",
                self.view_mode,
                options.samples as u32,
                self.visibility,
                report.scene.visible_instances,
                report.scene.total_instances,
                if self.shader_rejected {
                    " | shader rejected; last valid pipeline active"
                } else {
                    ""
                }
            ));
            if options.log_frames {
                eprintln!(
                    "visibility_ms={:.4} packing_ms={:.4} visible_ids_bytes={} model_writes={} id_writes={} scene={:?}",
                    report.cpu.visibility.as_secs_f64() * 1000.0,
                    report.cpu.packing.as_secs_f64() * 1000.0,
                    report.uploads.visible_ids,
                    report.uploads.model_writes,
                    report.uploads.visible_id_writes,
                    report.scene
                );
            }
            let probe = self.surface_probe.begin("pre_present_notify");
            self.window.pre_present_notify();
            probe.end("returned");
            let probe = self.surface_probe.begin("present");
            let present = Instant::now();
            let result = self.renderer.present(frame);
            let presented_at = Instant::now();
            probe.end(if result.is_ok() { "ok" } else { "error" });
            result?;
            self.presented += 1;
            for marker in &mut self.markers {
                if marker.startup && !marker.presented {
                    marker.revision = report.revision;
                }
                if marker.revision == report.revision && !marker.presented {
                    marker.emit("present_call", report.frame, presented_at);
                    marker.presented = true;
                }
            }
            if let Some(input) = self.applied_input_stamp.take() {
                eprintln!(
                    "input_to_present_call_ms={:.3} frame={}",
                    input.elapsed().as_secs_f64() * 1000.0,
                    report.frame.0
                );
            }
            if options.log_frames {
                eprintln!(
                    "frame={} revision={} prep_ms={:.4} upload_ms={:.4} encode_ms={:.4} submit_ms={:.4} poll_ms={:.4} acquire_ms={acquire_ms:.4} present_ms={:.4} configure_ms={configure_ms:.4} geometry_bytes={} model_bytes={} camera_bytes={} writes={}",
                    report.frame.0,
                    report.revision.0,
                    report.cpu.preparation.as_secs_f64() * 1000.0,
                    report.cpu.upload.as_secs_f64() * 1000.0,
                    report.cpu.encode.as_secs_f64() * 1000.0,
                    report.cpu.submit.as_secs_f64() * 1000.0,
                    report.cpu.poll.as_secs_f64() * 1000.0,
                    presented_at.duration_since(present).as_secs_f64() * 1000.0,
                    report.uploads.geometry,
                    report.uploads.models,
                    report.uploads.camera,
                    report.uploads.write_calls
                );
            }
            if options.frames.is_some_and(|count| self.presented >= count)
                && (options.capture_hdr.is_none() || self.hdr_written)
            {
                self.finish_at = Some(Instant::now());
            }
        } else {
            // Vulkan's native discard is a no-op in wgpu-hal 30.
            self.pending_frame = Some(frame);
        }
        if suboptimal {
            self.pending_size = Some(PendingResize::new(self.size));
        }
        Ok(())
    }
}
struct App {
    view: Option<View>,
    start: Instant,
    options: Options,
    error: Option<String>,
}
impl App {
    fn stop(&mut self, events: &ActiveEventLoop, error: impl std::fmt::Display) {
        eprintln!("viewer_stopped error={error}");
        self.error = Some(error.to_string());
        events.exit();
    }
}
impl ApplicationHandler for App {
    fn resumed(&mut self, events: &ActiveEventLoop) {
        if self.view.is_none() {
            match View::create(events, &self.options, self.start) {
                Ok(view) => self.view = Some(view),
                Err(error) => self.stop(events, error),
            }
        }
    }
    fn window_event(&mut self, events: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(view) = &mut self.view else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => events.exit(),
            WindowEvent::Resized(size) => {
                view.surface_probe.event("resize");
                view.size = size;
                view.pending_size = Some(PendingResize::new(size));
                eprintln!("resize_requested={}x{}", size.width, size.height);
            }
            WindowEvent::Occluded(value) => {
                view.occluded = value;
                eprintln!("occluded={value}");
            }
            WindowEvent::Focused(false) => view.keys.clear(),
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(key) = event.physical_key {
                    if event.state == ElementState::Released {
                        view.keys.remove(&key);
                    } else {
                        view.keys.insert(key);
                        view.input_stamp.get_or_insert(Instant::now());
                        if !event.repeat {
                            match key {
                                KeyCode::Escape => events.exit(),
                                KeyCode::Space => view.scene.animated = !view.scene.animated,
                                KeyCode::KeyN => view.scene.close_view = !view.scene.close_view,
                                KeyCode::KeyC => {
                                    view.visibility = if view.visibility == Visibility::Frustum {
                                        Visibility::Unculled
                                    } else {
                                        Visibility::Frustum
                                    }
                                }
                                KeyCode::Digit1 => view.view_mode = ViewMode::BaseColor,
                                KeyCode::Digit2 => view.view_mode = ViewMode::Normal,
                                KeyCode::Digit3 => view.view_mode = ViewMode::Metallic,
                                KeyCode::Digit4 => view.view_mode = ViewMode::Roughness,
                                KeyCode::Digit5 => view.view_mode = ViewMode::Lit,
                                KeyCode::KeyR => view.scene = Scene::default(),
                                _ => {}
                            }
                            eprintln!("input={key:?} scene={:?}", view.scene);
                        }
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                let before = view.presented;
                let probe = view.surface_probe.begin("redraw");
                let result = view.redraw(&self.options);
                probe.end(if result.is_ok() { "ok" } else { "error" });
                if let Err(error) = result {
                    self.stop(events, error);
                } else if view.presented == before {
                    view.retry_redraw_at = Instant::now() + RETRY_INTERVAL;
                } else {
                    view.retry_redraw_at = Instant::now();
                }
            }
            _ => {}
        }
    }
    fn about_to_wait(&mut self, events: &ActiveEventLoop) {
        if events.exiting() {
            return;
        }
        events.set_control_flow(ControlFlow::WaitUntil(Instant::now() + RETRY_INTERVAL));
        if let Some(view) = &mut self.view {
            if self
                .options
                .seconds
                .is_some_and(|seconds| self.start.elapsed().as_secs_f64() >= seconds)
            {
                view.finish_at.get_or_insert(Instant::now());
            }
            let probe = view.surface_probe.begin("wake");
            let result = view.wake(&self.options);
            probe.end(if result.is_ok() { "ok" } else { "error" });
            match result {
                Ok(true) => {
                    eprintln!(
                        "viewer_finished presented={} stats={:?}",
                        view.presented,
                        view.renderer.stats()
                    );
                    events.exit();
                }
                Ok(false) => {
                    if view.finish_at.is_none() {
                        if Instant::now() >= view.retry_redraw_at {
                            view.window.request_redraw();
                        } else {
                            events.set_control_flow(ControlFlow::WaitUntil(view.retry_redraw_at));
                        }
                    }
                }
                Err(error) => self.stop(events, error),
            }
        }
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now();
    let options = Options::parse()?;
    let _validation = validation::begin(options.validate);
    let mut app = App {
        view: None,
        start,
        options,
        error: None,
    };
    EventLoop::new()?.run_app(&mut app)?;
    if let Some(error) = app.error {
        return Err(error.into());
    }
    Ok(())
}
