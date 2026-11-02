use renderlab::{Renderer, Scene, wgpu};
use std::{
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

struct View {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    scene: Scene,
    source: String,
    check: Instant,
    first: bool,
    reload_stamp: Option<SystemTime>,
    animated: bool,
    last: Instant,
}
struct App {
    view: Option<View>,
    start: Instant,
    shader: PathBuf,
}
impl ApplicationHandler for App {
    fn resumed(&mut self, events: &ActiveEventLoop) {
        if self.view.is_some() {
            return;
        }
        let window = Arc::new(events.create_window(Window::default_attributes().with_title("Renderlab | arrows camera | A/D light | Q/E roughness | Z/X opacity | Space motion | Esc exit").with_inner_size(winit::dpi::PhysicalSize::new(960,640))).expect("window"));
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance.create_surface(window.clone()).expect("surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .expect("Vulkan adapter");
        eprintln!("adapter={:?}", adapter.get_info());
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).expect("device");
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width, size.height)
            .expect("surface config");
        config.format = surface
            .get_capabilities(&adapter)
            .formats
            .into_iter()
            .find(|f| f.is_srgb())
            .expect("sRGB surface");
        config.present_mode = wgpu::PresentMode::Fifo;
        surface.configure(&device, &config);
        let source = std::fs::read_to_string(&self.shader)
            .expect("read shader (pass its path as first argument)");
        let renderer =
            Renderer::new(device, queue, config.format, &source).expect("initial shader");
        self.view = Some(View {
            window,
            surface,
            config,
            renderer,
            scene: Scene::default(),
            source,
            check: Instant::now(),
            first: true,
            reload_stamp: None,
            animated: false,
            last: Instant::now(),
        });
    }
    fn window_event(&mut self, events: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(v) = &mut self.view else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => events.exit(),
            WindowEvent::Resized(size) => {
                v.config.width = size.width;
                v.config.height = size.height;
                if size.width > 0 && size.height > 0 {
                    v.surface.configure(v.renderer.device(), &v.config);
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::Escape) => events.exit(),
                    PhysicalKey::Code(KeyCode::ArrowLeft) => v.scene.yaw -= 0.08,
                    PhysicalKey::Code(KeyCode::ArrowRight) => v.scene.yaw += 0.08,
                    PhysicalKey::Code(KeyCode::ArrowUp) => {
                        v.scene.elevation = (v.scene.elevation + 0.15).min(8.0)
                    }
                    PhysicalKey::Code(KeyCode::ArrowDown) => {
                        v.scene.elevation = (v.scene.elevation - 0.15).max(0.2)
                    }
                    PhysicalKey::Code(KeyCode::KeyA) => v.scene.light_x -= 0.25,
                    PhysicalKey::Code(KeyCode::KeyD) => v.scene.light_x += 0.25,
                    PhysicalKey::Code(KeyCode::KeyQ) => {
                        v.scene.roughness = (v.scene.roughness - 0.05).max(0.05)
                    }
                    PhysicalKey::Code(KeyCode::KeyE) => {
                        v.scene.roughness = (v.scene.roughness + 0.05).min(1.0)
                    }
                    PhysicalKey::Code(KeyCode::KeyZ) => {
                        v.scene.opacity = (v.scene.opacity - 0.05).max(0.0)
                    }
                    PhysicalKey::Code(KeyCode::KeyX) => {
                        v.scene.opacity = (v.scene.opacity + 0.05).min(1.0)
                    }
                    PhysicalKey::Code(KeyCode::Space) => v.animated = !v.animated,
                    PhysicalKey::Code(KeyCode::KeyR) => v.scene = Scene::default(),
                    _ => {}
                }
                eprintln!("parameters={:?}", v.scene);
            }
            WindowEvent::RedrawRequested if v.config.width > 0 && v.config.height > 0 => {
                if v.check.elapsed() >= Duration::from_millis(50) {
                    v.check = Instant::now();
                    if let Ok(source) = std::fs::read_to_string(&self.shader)
                        && source != v.source
                    {
                        v.source = source;
                        let stamp = std::fs::metadata(&self.shader)
                            .and_then(|m| m.modified())
                            .ok();
                        let started = Instant::now();
                        match v.renderer.reload(&v.source) {
                            Ok(()) => {
                                eprintln!(
                                    "reload_compile_ms={:.3}",
                                    started.elapsed().as_secs_f64() * 1000.0
                                );
                                v.reload_stamp = stamp;
                                v.window.set_title("Renderlab | shader reloaded | arrows camera, A/D light, Q/E roughness, Z/X opacity, Space motion");
                            }
                            Err(error) => {
                                eprintln!("reload rejected; last valid pipeline retained: {error}");
                                v.window.set_title("Renderlab | shader error (see terminal); last valid image retained");
                            }
                        }
                    }
                }
                let dt = v.last.elapsed().as_secs_f32().min(0.1);
                v.last = Instant::now();
                if v.animated {
                    v.scene.motion = (self.start.elapsed().as_secs_f32() * 0.8).sin() * 0.6;
                    v.scene.yaw += dt * 0.12;
                }
                match v.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                        v.renderer.render(
                            &frame.texture.create_view(&Default::default()),
                            [v.config.width, v.config.height],
                            v.scene,
                        );
                        if v.first || v.reload_stamp.is_some() {
                            v.renderer
                                .device()
                                .poll(wgpu::PollType::wait_indefinitely())
                                .expect("GPU completion");
                        }
                        v.window.pre_present_notify();
                        v.renderer.present(frame);
                        if v.first {
                            eprintln!(
                                "startup_to_present_ms={:.3}",
                                self.start.elapsed().as_secs_f64() * 1000.0
                            );
                            v.first = false;
                        }
                        if let Some(stamp) = v.reload_stamp.take() {
                            eprintln!(
                                "shader_write_to_present_ms={:.3}",
                                stamp.elapsed().unwrap_or_default().as_secs_f64() * 1000.0
                            );
                        }
                    }
                    wgpu::CurrentSurfaceTexture::Outdated => {
                        v.surface.configure(v.renderer.device(), &v.config)
                    }
                    wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Validation => {
                        eprintln!("surface lost; restart viewer");
                        events.exit();
                    }
                    wgpu::CurrentSurfaceTexture::Timeout
                    | wgpu::CurrentSurfaceTexture::Occluded => {}
                }
            }
            _ => {}
        }
    }
    fn about_to_wait(&mut self, events: &ActiveEventLoop) {
        events.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(16),
        ));
        if let Some(v) = &self.view {
            v.window.request_redraw();
        }
    }
}
fn main() {
    let start = Instant::now();
    let shader = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../shaders/scene.wgsl"));
    EventLoop::new()
        .expect("event loop")
        .run_app(&mut App {
            view: None,
            start,
            shader,
        })
        .expect("viewer");
}
