mod fixture;
use fixture::Scene;
use renderlab::{FrameId, Limits, Mesh, Renderer, Revision, wgpu};
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

const TITLE: &str = "Renderlab | arrows orbit | WASD translate | R reset | Space motion | Esc exit";
struct Options {
    shader: PathBuf,
    frames: Option<u64>,
    seconds: Option<f64>,
    timestamps: bool,
    log_frames: bool,
}
impl Options {
    fn parse() -> Result<Self, String> {
        let mut options = Self {
            shader: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../shaders/scene.wgsl"),
            frames: None,
            seconds: None,
            timestamps: true,
            log_frames: false,
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
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
                "--shader" => options.shader = args.next().ok_or("missing shader path")?.into(),
                "--no-timestamps" => options.timestamps = false,
                "--log-frames" => options.log_frames = true,
                _ if !arg.starts_with('-') => options.shader = arg.into(),
                _ => return Err(format!("unknown argument {arg}")),
            }
        }
        if options.frames == Some(0) || options.seconds.is_some_and(|s| !s.is_finite() || s <= 0.0)
        {
            return Err("limits must be positive".into());
        }
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
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    meshes: Vec<Mesh>,
    scene: Scene,
    keys: HashSet<KeyCode>,
    input_stamp: Option<Instant>,
    applied_input_stamp: Option<Instant>,
    source: String,
    shader_check: Instant,
    markers: VecDeque<Marker>,
    last: Instant,
    pending_size: Option<winit::dpi::PhysicalSize<u32>>,
    size: winit::dpi::PhysicalSize<u32>,
    configured: bool,
    occluded: bool,
    presented: u64,
    finish_at: Option<Instant>,
}
impl View {
    fn create(
        events: &ActiveEventLoop,
        options: &Options,
        start: Instant,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let window = Arc::new(
            events.create_window(
                Window::default_attributes()
                    .with_title(TITLE)
                    .with_inner_size(winit::dpi::PhysicalSize::new(960, 640)),
            )?,
        );
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance.create_surface(window.clone())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))?;
        eprintln!(
            "adapter={:?} scene=two_cubes_triangle quality=single_sample_depth32float profile=dev_or_selected_cargo_profile",
            adapter.get_info()
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
        let renderer = Renderer::new(device, queue, config.format, Limits::default(), &source)?;
        eprintln!(
            "timing_status={:?} measurement_definition=m1_async_not_m0",
            renderer.stats().timing
        );
        Ok(Self {
            window,
            surface,
            config,
            renderer,
            meshes: Vec::new(),
            scene: Scene::default(),
            keys: HashSet::new(),
            input_stamp: None,
            applied_input_stamp: None,
            source,
            shader_check: Instant::now(),
            markers: VecDeque::from([Marker {
                revision: Revision(1),
                origin: start,
                file_age: Duration::ZERO,
                startup: true,
                presented: false,
                completed: false,
            }]),
            last: Instant::now(),
            pending_size: Some(size),
            size,
            configured: false,
            occluded: false,
            presented: 0,
            finish_at: None,
        })
    }
    fn wake(&mut self, options: &Options) -> Result<bool, Box<dyn std::error::Error>> {
        let events = self.renderer.poll()?;
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
        if let Some(finish) = self.finish_at {
            if self.renderer.stats().flights == 0 {
                return Ok(true);
            }
            if finish.elapsed() > Duration::from_secs(5) {
                return Err("nonblocking final completion deadline exceeded".into());
            }
            return Ok(false);
        }
        if self.meshes.len() < 2 {
            fixture::upload_next(&mut self.renderer, &mut self.meshes)?;
        }
        if self.shader_check.elapsed() >= Duration::from_millis(50) {
            self.shader_check = Instant::now();
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
                        self.window.set_title(TITLE);
                    }
                    Err(renderlab::ReloadError::Rejected {
                        diagnostic,
                        compile,
                    }) => {
                        eprintln!(
                            "reload_rejected compile_ms={:.3} diagnostic={diagnostic}",
                            compile.as_secs_f64() * 1000.0
                        );
                        self.window.set_title("Renderlab | shader rejected; last valid pipeline active | see terminal");
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
            || self.meshes.len() < 2
        {
            return Ok(());
        }
        let mut configure_ms = 0.0;
        if let Some(size) = self.pending_size.take() {
            self.config.width = size.width;
            self.config.height = size.height;
            let clock = Instant::now();
            self.surface.configure(self.renderer.device(), &self.config);
            configure_ms = clock.elapsed().as_secs_f64() * 1000.0;
            self.configured = true;
            eprintln!(
                "configure_ms={configure_ms:.3} width={} height={}",
                size.width, size.height
            );
            self.renderer.poll()?;
        }
        if !self.configured {
            return Ok(());
        }
        let acquire = Instant::now();
        let result = self.surface.get_current_texture();
        let acquire_ms = acquire.elapsed().as_secs_f64() * 1000.0;
        let (frame, suboptimal) = match result {
            wgpu::CurrentSurfaceTexture::Success(frame) => (frame, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.pending_size = Some(self.size);
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
        let report = self.renderer.try_render(
            &frame.texture,
            &self.scene.camera(),
            &self.scene.draws(&self.meshes),
        )?;
        if let Some(report) = report {
            self.window.pre_present_notify();
            let present = Instant::now();
            self.renderer.present(frame)?;
            let presented_at = Instant::now();
            self.presented += 1;
            for marker in &mut self.markers {
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
            if options.frames.is_some_and(|count| self.presented >= count) {
                self.finish_at = Some(Instant::now());
            }
        } else {
            drop(frame);
        }
        if suboptimal {
            self.pending_size = Some(self.size);
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
                view.size = size;
                view.pending_size = Some(size);
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
                                KeyCode::KeyR => view.scene = Scene::default(),
                                _ => {}
                            }
                            eprintln!("input={key:?} scene={:?}", view.scene);
                        }
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = view.redraw(&self.options) {
                    self.stop(events, error);
                }
            }
            _ => {}
        }
    }
    fn about_to_wait(&mut self, events: &ActiveEventLoop) {
        if events.exiting() {
            return;
        }
        events.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(8),
        ));
        if let Some(view) = &mut self.view {
            if self
                .options
                .seconds
                .is_some_and(|seconds| self.start.elapsed().as_secs_f64() >= seconds)
            {
                view.finish_at.get_or_insert(Instant::now());
            }
            match view.wake(&self.options) {
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
                        view.window.request_redraw();
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
