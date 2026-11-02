//! A small owned renderer for the development-loop scene. Windowing lives in the viewer.
pub use wgpu;

#[derive(Clone, Copy, Debug)]
pub struct Scene {
    pub yaw: f32,
    pub elevation: f32,
    pub light_x: f32,
    pub roughness: f32,
    pub opacity: f32,
    pub motion: f32,
}
impl Default for Scene {
    fn default() -> Self {
        Self {
            yaw: 0.35,
            elevation: 2.8,
            light_x: -3.0,
            roughness: 0.35,
            opacity: 0.35,
            motion: 0.0,
        }
    }
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::PipelineLayout,
    bind: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    pipeline: wgpu::RenderPipeline,
    format: wgpu::TextureFormat,
}
impl Renderer {
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        source: &str,
    ) -> Result<Self, wgpu::Error> {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene"),
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(48),
                },
                count: None,
            }],
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bindings,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bindings)],
            immediate_size: 0,
        });
        let pipeline = Self::compile(&device, &layout, format, source)?;
        Ok(Self {
            device,
            queue,
            layout,
            bind,
            uniform,
            pipeline,
            format,
        })
    }
    fn compile(
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        format: wgpu::TextureFormat,
        source: &str,
    ) -> Result<wgpu::RenderPipeline, wgpu::Error> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("live scene"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        match pollster::block_on(scope.pop()) {
            Some(error) => Err(error),
            None => Ok(pipeline),
        }
    }
    /// Publish only a fully validated pipeline. On error the current pipeline and bindings survive.
    pub fn reload(&mut self, source: &str) -> Result<(), wgpu::Error> {
        self.pipeline = Self::compile(&self.device, &self.layout, self.format, source)?;
        Ok(())
    }
    pub fn present(&self, frame: wgpu::SurfaceTexture) {
        self.queue.present(frame);
    }
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }
    pub fn render(&self, view: &wgpu::TextureView, size: [u32; 2], scene: Scene) {
        let values = [
            size[0] as f32,
            size[1] as f32,
            scene.yaw,
            scene.elevation,
            scene.light_x,
            scene.roughness.clamp(0.05, 1.0),
            scene.opacity.clamp(0.0, 1.0),
            scene.motion,
            0.0,
            0.0,
            0.0,
            0.0,
        ];
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_ne_bytes()).collect();
        self.queue.write_buffer(&self.uniform, 0, &bytes);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn capture(renderer: &Renderer, scene: Scene) -> Vec<u8> {
        let width = 960;
        let height = 640;
        let texture = renderer.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: renderer.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        renderer.render(
            &texture.create_view(&Default::default()),
            [width, height],
            scene,
        );
        let buffer = renderer.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(width * height * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = renderer.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
            },
            texture.size(),
        );
        renderer.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
        renderer
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        rx.recv().unwrap().unwrap();
        buffer
            .slice(..)
            .get_mapped_range()
            .unwrap()
            .chunks_exact(4)
            .flat_map(|v| [v[0], v[1], v[2]])
            .collect()
    }
    fn save(name: &str, rgb: &[u8]) {
        use std::io::Write;
        std::fs::create_dir_all("captures").unwrap();
        let mut f = std::fs::File::create(format!("captures/{name}.ppm")).unwrap();
        f.write_all(b"P6\n960 640\n255\n").unwrap();
        f.write_all(rgb).unwrap();
    }
    #[test]
    #[ignore = "requires a Vulkan GPU; writes full-frame captures"]
    fn reload_is_transactional_and_parameters_change_image() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let source = std::fs::read_to_string("shaders/scene.wgsl").unwrap();
        let mut renderer =
            Renderer::new(device, queue, wgpu::TextureFormat::Rgba8UnormSrgb, &source).unwrap();
        let baseline = capture(&renderer, Scene::default());
        save("baseline", &baseline);
        assert!(renderer.reload("this is invalid WGSL").is_err());
        assert_eq!(baseline, capture(&renderer, Scene::default()));
        assert!(
            renderer
                .reload(&source.replace("@binding(0)", "@binding(1)"))
                .is_err()
        );
        assert_eq!(baseline, capture(&renderer, Scene::default()));
        for (name, scene) in [
            (
                "camera",
                Scene {
                    yaw: 0.8,
                    ..Default::default()
                },
            ),
            (
                "light",
                Scene {
                    light_x: 3.0,
                    ..Default::default()
                },
            ),
            (
                "roughness",
                Scene {
                    roughness: 0.9,
                    ..Default::default()
                },
            ),
            (
                "transparent",
                Scene {
                    opacity: 0.0,
                    ..Default::default()
                },
            ),
            (
                "opaque",
                Scene {
                    opacity: 1.0,
                    ..Default::default()
                },
            ),
            (
                "motion",
                Scene {
                    motion: 0.5,
                    ..Default::default()
                },
            ),
        ] {
            let rgb = capture(&renderer, scene);
            assert_ne!(baseline, rgb, "{name}");
            save(name, &rgb);
        }
        renderer
            .reload(&source.replace("0.75,0.16,0.055", "0.055,0.16,0.75"))
            .unwrap();
        let changed = capture(&renderer, Scene::default());
        assert_ne!(baseline, changed);
        save("reload", &changed);
        renderer.reload(&source).unwrap();
        assert_eq!(baseline, capture(&renderer, Scene::default()));
    }
}
