//! PP1 owns its output textures. Nothing here writes HDR or reconstruction history.
use super::primary_ray_gpu::Buffer;
use super::*;
use crate::post::{ColourLut, PostSettings, Tonemapper};

pub(super) struct PostGpu {
    size: wgpu::Extent3d,
    settings: Buffer,
    frame: Buffer,
    aux: Depth,
    tile: Depth,
    ping: [Depth; 2],
    down: Vec<Depth>,
    up: Vec<Depth>,
    compute_layout: Owned<wgpu::BindGroupLayout>,
    kernels: [Owned<wgpu::ComputePipeline>; 5],
    primary_layout: Option<Owned<wgpu::BindGroupLayout>>,
    primary_pipeline: Option<Owned<wgpu::ComputePipeline>>,
    raster_layout: Owned<wgpu::BindGroupLayout>,
    raster_pipeline: Owned<wgpu::ComputePipeline>,
    tone_layout: Owned<wgpu::BindGroupLayout>,
    tone: Owned<wgpu::RenderPipeline>,
    sampler: Owned<wgpu::Sampler>,
    lut: Depth,
    retired_luts: Vec<(u64, Depth)>,
    lut_custom: bool,
    previous: Option<Mat4>,
    serial: u32,
    // Bindings are rebuilt only when input textures or enabled stage topology change.
    groups: Option<Groups>,
}
struct Groups {
    key: (wgpu::Texture, Option<wgpu::Texture>, bool, bool, bool),
    surface: Owned<wgpu::BindGroup>,
    tile: Owned<wgpu::BindGroup>,
    motion: Owned<wgpu::BindGroup>,
    dof: Owned<wgpu::BindGroup>,
    down: Vec<Owned<wgpu::BindGroup>>,
    up: Vec<Owned<wgpu::BindGroup>>,
    tone: Owned<wgpu::BindGroup>,
}
fn entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::FRAGMENT,
        ty,
        count: None,
    }
}
fn sampled(binding: u32, kind: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Texture {
            sample_type: kind,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
    )
}
fn uniform(binding: u32, size: u64) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(size),
        },
    )
}
fn storage(binding: u32, format: wgpu::TextureFormat) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
    )
}
fn layout(
    r: &Renderer,
    label: &str,
    entries: &[wgpu::BindGroupLayoutEntry],
) -> Owned<wgpu::BindGroupLayout> {
    r.ledger.own(
        r.device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            }),
        ResourceKind::BindGroupLayout,
    )
}
fn group(
    r: &Renderer,
    label: &str,
    layout: &wgpu::BindGroupLayout,
    resources: &[(u32, wgpu::BindingResource<'_>)],
) -> Owned<wgpu::BindGroup> {
    let entries: Vec<_> = resources
        .iter()
        .map(|(binding, resource)| wgpu::BindGroupEntry {
            binding: *binding,
            resource: resource.clone(),
        })
        .collect();
    r.ledger.own(
        r.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &entries,
        }),
        ResourceKind::BindGroup,
    )
}
fn pipeline(
    r: &Renderer,
    label: &str,
    layouts: &[&wgpu::BindGroupLayout],
    source: &str,
    entry: &str,
) -> Owned<wgpu::ComputePipeline> {
    let module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let layout = r
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &layouts.iter().map(|l| Some(*l)).collect::<Vec<_>>(),
            immediate_size: 0,
        });
    r.ledger.own(
        r.device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            }),
        ResourceKind::ComputePipeline,
    )
}
fn texture(
    r: &Renderer,
    label: &str,
    size: wgpu::Extent3d,
    format: wgpu::TextureFormat,
    dimension: wgpu::TextureDimension,
) -> Depth {
    let texture = r.ledger.own(
        r.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | if dimension == wgpu::TextureDimension::D3 {
                    wgpu::TextureUsages::COPY_DST
                } else {
                    wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC
                },
            view_formats: &[],
        }),
        ResourceKind::Texture,
    );
    let view = r.ledger.own(
        texture.create_view(&Default::default()),
        ResourceKind::TextureView,
    );
    Depth {
        texture,
        view,
        _allocation: r.ledger.allocate(
            AllocationKind::Target,
            u64::from(size.width)
                * u64::from(size.height)
                * u64::from(size.depth_or_array_layers)
                * u64::from(format.block_copy_size(None).unwrap()),
        ),
    }
}
impl PostGpu {
    fn new(r: &Renderer, size: wgpu::Extent3d) -> Result<Self, Error> {
        use wgpu::{TextureFormat as F, TextureSampleType as T};
        let float = T::Float { filterable: false };
        let compute_layout = layout(
            r,
            "post filters",
            &[
                sampled(0, T::Float { filterable: true }),
                sampled(1, T::Float { filterable: true }),
                sampled(2, float),
                sampled(3, float),
                storage(4, F::Rgba16Float),
                uniform(5, 80),
                entry(
                    6,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
            ],
        );
        let kernels = [
            "downsample",
            "upsample",
            "tile_max",
            "motion_blur",
            "depth_of_field",
        ]
        .map(|e| {
            pipeline(
                r,
                e,
                &[&compute_layout],
                include_str!("../shaders/post.wgsl"),
                e,
            )
        });
        let (primary_layout, primary_pipeline) = if let Some(primary) = &r.primary {
            let bindings = layout(
                r,
                "post primary surface",
                &[
                    sampled(7, T::Uint),
                    // M7-R1n: the frame uniform's own size (it grew past 112 bytes).
                    uniform(10, primary_ray_gpu::FRAME_BYTES as u64),
                    sampled(21, T::Uint),
                    uniform(40, 272),
                    storage(41, F::Rgba32Float),
                ],
            );
            let head =
                primary_ray_gpu::non_rt_heads(include_str!("../shaders/scene.wgsl"))?[0].clone();
            let pipe = pipeline(
                r,
                "post primary surface",
                &[&bindings, primary.array_layout()],
                &(head + include_str!("../shaders/post_surface.wgsl")),
                "post_primary_surface",
            );
            (Some(bindings), Some(pipe))
        } else {
            (None, None)
        };
        let mut depth = sampled(0, T::Depth);
        let mut raster_source = include_str!("../shaders/post_raster_surface.wgsl").to_owned();
        if r.options.samples == SampleCount::Four {
            if let wgpu::BindingType::Texture { multisampled, .. } = &mut depth.ty {
                *multisampled = true;
            }
            raster_source =
                raster_source.replace("texture_depth_2d;", "texture_depth_multisampled_2d;");
        }
        let raster_layout = layout(
            r,
            "post raster surface",
            &[depth, uniform(1, 272), storage(2, F::Rgba32Float)],
        );
        let raster_pipeline = pipeline(
            r,
            "post raster surface",
            &[&raster_layout],
            &raster_source,
            "surface_raster",
        );
        let tone_layout = layout(
            r,
            "post tone",
            &[
                sampled(0, T::Float { filterable: true }),
                sampled(1, T::Float { filterable: true }),
                entry(
                    2,
                    wgpu::BindingType::Texture {
                        sample_type: T::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                ),
                entry(
                    3,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                uniform(4, 80),
            ],
        );
        let pl = r
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("post tone"),
                bind_group_layouts: &[Some(&tone_layout)],
                immediate_size: 0,
            });
        let module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post tone"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/post_tone.wgsl").into()),
        });
        let tone = r.ledger.own(
            r.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("post tone"),
                    layout: Some(&pl),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("vs"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some("fs"),
                        compilation_options: Default::default(),
                        targets: &[Some(r.format.into())],
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                }),
            ResourceKind::RenderPipeline,
        );
        let sampler = r.ledger.own(
            r.device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("post linear clamp"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            ResourceKind::Sampler,
        );
        let image =
            |label, size| texture(r, label, size, F::Rgba16Float, wgpu::TextureDimension::D2);
        let mut level = size;
        let mut down = vec![];
        let mut up = vec![];
        for _ in 0..6 {
            level.width = level.width.div_ceil(2);
            level.height = level.height.div_ceil(2);
            down.push(image("post bloom down", level));
            up.push(image("post bloom up", level));
            if level.width == 1 && level.height == 1 {
                break;
            }
        }
        let mut result = Self {
            size,
            settings: Buffer::new(r, "post settings", 80, wgpu::BufferUsages::UNIFORM),
            frame: Buffer::new(r, "post camera", 272, wgpu::BufferUsages::UNIFORM),
            aux: texture(
                r,
                "post depth velocity",
                size,
                F::Rgba32Float,
                wgpu::TextureDimension::D2,
            ),
            tile: image(
                "post velocity max",
                wgpu::Extent3d {
                    width: size.width.div_ceil(16),
                    height: size.height.div_ceil(16),
                    ..size
                },
            ),
            ping: [image("post motion", size), image("post DOF", size)],
            down,
            up,
            compute_layout,
            kernels,
            primary_layout,
            primary_pipeline,
            raster_layout,
            raster_pipeline,
            tone_layout,
            tone,
            sampler,
            lut: texture(
                r,
                "post identity LUT",
                wgpu::Extent3d {
                    width: 2,
                    height: 2,
                    depth_or_array_layers: 2,
                },
                F::Rgba8Unorm,
                wgpu::TextureDimension::D3,
            ),
            lut_custom: false,
            retired_luts: vec![],
            previous: None,
            serial: 0,
            groups: None,
        };
        result.write_lut(r, &ColourLut::identity(2)?);
        Ok(result)
    }
    fn write_lut(&mut self, r: &Renderer, lut: &ColourLut) {
        let size = wgpu::Extent3d {
            width: lut.size,
            height: lut.size,
            depth_or_array_layers: lut.size,
        };
        if self.lut.texture.size() != size {
            let next = texture(
                r,
                "post colour LUT",
                size,
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureDimension::D3,
            );
            self.retired_luts
                .push((r.submissions, std::mem::replace(&mut self.lut, next)));
        }
        let bytes: Vec<u8> = lut
            .rgb
            .iter()
            .flat_map(|v| {
                v.map(|x| (x * 255.0).round() as u8)
                    .into_iter()
                    .chain([255])
            })
            .collect();
        r.queue.write_texture(
            self.lut.texture.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(lut.size * 4),
                rows_per_image: Some(lut.size),
            },
            size,
        );
        self.groups = None;
    }
    fn prepare(&mut self, r: &Renderer, camera: &Camera, primary: bool) {
        self.retired_luts
            .retain(|(last, _)| *last > r.completed_submission);
        let s = r.post_settings;
        let internal = r.lighting.hdr.as_ref().unwrap().resolved.texture.size();
        let current = camera.unjittered().matrix(internal.width, internal.height);
        let previous = self.previous.unwrap_or(current);
        let mut frame = Vec::with_capacity(272);
        for m in [
            previous,
            current,
            camera.matrix(internal.width, internal.height).inverse(),
            camera.view(),
        ] {
            frame.extend_from_slice(&matrix_bytes(m));
        }
        for f in [
            s.shutter_angle_degrees / 720.0,
            s.max_motion_radius_px,
            if self.previous.is_some() { 1.0 } else { 0.0 },
            0.0,
        ] {
            frame.extend_from_slice(&f.to_ne_bytes());
        }
        r.queue.write_buffer(&self.frame.value, 0, &frame);
        self.previous = Some(current);
        self.serial = self.serial.wrapping_add(1);
        let d = s.depth_of_field.unwrap_or_default();
        let focal = d.focal_length_mm * 0.001;
        let scale = focal * focal / (d.f_number * (d.focus_distance_m - focal))
            * self.size.height as f32
            / (d.sensor_height_mm * 0.001)
            * 0.5;
        // Neutral at 6500 K; bounded RGB camera adaptation, separate from exposure.
        let warmth = (6500.0 / s.white_balance_kelvin).ln();
        let white = [
            (warmth * 0.35).exp(),
            (-s.tint * 0.15).exp(),
            (-warmth * 0.35).exp(),
        ];
        let values = [
            s.bloom,
            s.shutter_angle_degrees / 720.0,
            s.max_motion_radius_px,
            (self.serial & 0x00ff_ffff) as f32,
            d.focus_distance_m,
            scale,
            d.max_radius_px,
            u32::from(s.depth_of_field.is_some()) as f32,
            s.contrast,
            s.saturation,
            s.vignette,
            s.grain,
            white[0],
            white[1],
            white[2],
            u32::from(s.tonemapper == Tonemapper::Filmic) as f32,
            s.chromatic_aberration_px,
            if self.lut_custom { s.lut_strength } else { 0.0 },
            self.lut.texture.width() as f32,
            0.0,
        ];
        let bytes: Vec<_> = values.into_iter().flat_map(f32::to_ne_bytes).collect();
        r.queue.write_buffer(&self.settings.value, 0, &bytes);
        use wgpu::BindingResource as R;
        let hdr = r.post_hdr();
        let hit = primary.then(|| {
            (*r.primary
                .as_ref()
                .unwrap()
                .targets
                .as_ref()
                .unwrap()
                .hit
                .texture)
                .clone()
        });
        let key = (
            (*hdr.texture).clone(),
            hit,
            s.motion_blur,
            s.depth_of_field.is_some(),
            s.bloom > 0.0,
        );
        if self.groups.as_ref().is_some_and(|g| g.key == key) {
            return;
        }
        let surface = if primary {
            let p = r.primary.as_ref().unwrap();
            let t = p.targets.as_ref().unwrap();
            group(
                r,
                "post primary surface",
                self.primary_layout.as_ref().unwrap(),
                &[
                    (7, R::TextureView(&t.hit.view)),
                    (10, p.frame_buffer().as_entire_binding()),
                    (21, R::TextureView(&t.tiles.view)),
                    (40, self.frame.value.as_entire_binding()),
                    (41, R::TextureView(&self.aux.view)),
                ],
            )
        } else {
            group(
                r,
                "post raster surface",
                &self.raster_layout,
                &[
                    (0, R::TextureView(&r.depth.as_ref().unwrap().view)),
                    (1, self.frame.value.as_entire_binding()),
                    (2, R::TextureView(&self.aux.view)),
                ],
            )
        };
        let filter = |a: &Depth, b: &Depth, out: &Depth| {
            group(
                r,
                "post filter",
                &self.compute_layout,
                &[
                    (0, R::TextureView(&a.view)),
                    (1, R::TextureView(&b.view)),
                    (2, R::TextureView(&self.aux.view)),
                    (3, R::TextureView(&self.tile.view)),
                    (4, R::TextureView(&out.view)),
                    (5, self.settings.value.as_entire_binding()),
                    (6, R::Sampler(&self.sampler)),
                ],
            )
        };
        // The tile pass must not bind its destination as a sampled texture.
        let tile = group(
            r,
            "post tile max",
            &self.compute_layout,
            &[
                (0, R::TextureView(&hdr.view)),
                (1, R::TextureView(&hdr.view)),
                (2, R::TextureView(&self.aux.view)),
                (3, R::TextureView(&hdr.view)),
                (4, R::TextureView(&self.tile.view)),
                (5, self.settings.value.as_entire_binding()),
                (6, R::Sampler(&self.sampler)),
            ],
        );
        let motion = filter(hdr, hdr, &self.ping[0]);
        let after_motion = if s.motion_blur { &self.ping[0] } else { hdr };
        let dof = filter(after_motion, after_motion, &self.ping[1]);
        let colour = if s.depth_of_field.is_some() {
            &self.ping[1]
        } else {
            after_motion
        };
        let down = self
            .down
            .iter()
            .enumerate()
            .map(|(i, out)| filter(if i == 0 { colour } else { &self.down[i - 1] }, colour, out))
            .collect();
        let up = self
            .up
            .iter()
            .enumerate()
            .map(|(i, out)| {
                filter(
                    &self.down[i],
                    if i + 1 == self.down.len() {
                        &self.down[i]
                    } else {
                        &self.up[i + 1]
                    },
                    out,
                )
            })
            .collect();
        let bloom = if s.bloom > 0.0 { &self.up[0] } else { colour };
        let tone = group(
            r,
            "post tone",
            &self.tone_layout,
            &[
                (0, R::TextureView(&colour.view)),
                (1, R::TextureView(&bloom.view)),
                (2, R::TextureView(&self.lut.view)),
                (3, R::Sampler(&self.sampler)),
                (4, self.settings.value.as_entire_binding()),
            ],
        );
        self.groups = Some(Groups {
            key,
            surface,
            tile,
            motion,
            dof,
            down,
            up,
            tone,
        });
    }
    pub(super) fn encode(&self, r: &Renderer, encoder: &mut wgpu::CommandEncoder) {
        let g = self.groups.as_ref().unwrap();
        let s = r.post_settings;
        if s.motion_blur || s.depth_of_field.is_some() {
            let primary = g.key.1.is_some();
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("post surface"),
                timestamp_writes: None,
            });
            pass.set_pipeline(if primary {
                self.primary_pipeline.as_ref().unwrap()
            } else {
                &self.raster_pipeline
            });
            pass.set_bind_group(0, &*g.surface, &[]);
            if primary {
                pass.set_bind_group(
                    1,
                    r.primary.as_ref().unwrap().virtual_geometry().unwrap().1,
                    &[],
                );
            }
            pass.dispatch_workgroups(self.size.width.div_ceil(8), self.size.height.div_ceil(8), 1);
        }
        let dispatch = |encoder: &mut wgpu::CommandEncoder,
                        k: usize,
                        group: &wgpu::BindGroup,
                        size: wgpu::Extent3d| {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("post filter"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.kernels[k]);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(size.width.div_ceil(8), size.height.div_ceil(8), 1);
        };
        if s.motion_blur {
            dispatch(encoder, 2, &g.tile, self.tile.texture.size());
            dispatch(encoder, 3, &g.motion, self.size);
        }
        if s.depth_of_field.is_some() {
            dispatch(encoder, 4, &g.dof, self.size);
        }
        if s.bloom > 0.0 {
            for (i, group) in g.down.iter().enumerate() {
                dispatch(encoder, 0, group, self.down[i].texture.size());
            }
            for (i, group) in g.up.iter().enumerate().rev() {
                dispatch(encoder, 1, group, self.up[i].texture.size());
            }
        }
    }
    pub(super) fn tone<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        pass.set_pipeline(&self.tone);
        pass.set_bind_group(0, &*self.groups.as_ref().unwrap().tone, &[]);
    }
}
impl Renderer {
    /// The image after reconstruction: native HDR, or TR1b's target-size output.
    fn post_hdr(&self) -> &Depth {
        self.upscale
            .as_ref()
            .map_or(&self.lighting.hdr.as_ref().unwrap().resolved, |u| &u.hdr)
    }
    pub fn post_settings(&self) -> PostSettings {
        self.post_settings
    }
    /// Presentation settings never invalidate TR1. Disabling releases post resources.
    pub fn set_post_settings(&mut self, settings: PostSettings) -> Result<(), Error> {
        settings.validate()?;
        self.post_settings = settings;
        if !settings.enabled {
            if let Some(post) = self.post.take() {
                self.retired_post.push((self.submissions, post));
            }
        }
        Ok(())
    }
    /// Set or clear the display-linear LUT. The owned CPU data survives target resizes.
    pub fn set_colour_lut(&mut self, lut: Option<ColourLut>) -> Result<(), Error> {
        if let Some(lut) = &lut {
            lut.validate()?;
            if lut.size > self.device.limits().max_texture_dimension_3d {
                return Err(Error::InvalidInput(
                    "LUT exceeds device 3D texture dimension",
                ));
            }
        }
        self.post_lut = lut;
        if let Some(mut post) = self.post.take() {
            post.write_lut(
                self,
                self.post_lut.as_ref().unwrap_or(&ColourLut::identity(2)?),
            );
            post.lut_custom = self.post_lut.is_some();
            self.post = Some(post);
        }
        Ok(())
    }
    /// Suppress shutter motion across an explicit camera cut.
    pub fn cut_post_motion(&mut self) {
        if let Some(post) = &mut self.post {
            post.previous = None;
        }
    }
    pub(super) fn prepare_post(&mut self, camera: &Camera, primary: bool) -> Result<(), Error> {
        if !self.post_settings.enabled {
            return Ok(());
        }
        let size = self.post_hdr().texture.size();
        let mut post = match self.post.take() {
            Some(p) if p.size == size && (!primary || p.primary_pipeline.is_some()) => p,
            old => {
                if let Some(old) = old {
                    self.retired_post.push((self.submissions, old));
                }
                let post = PostGpu::new(self, size)?;
                self.pipelines.created += post.kernels.len() as u64
                    + 2 // raster auxiliary and tone
                    + u64::from(post.primary_pipeline.is_some());
                post
            }
        };
        if post.serial == 0 {
            if let Some(lut) = &self.post_lut {
                post.write_lut(self, lut);
                post.lut_custom = true;
            }
        }
        post.prepare(self, camera, primary);
        self.post = Some(post);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn post_shader_boundaries_validate() {
        let primary = primary_ray_gpu::non_rt_heads(include_str!("../shaders/scene.wgsl")).unwrap()
            [0]
        .clone()
            + include_str!("../shaders/post_surface.wgsl");
        for source in [
            include_str!("../shaders/post.wgsl"),
            include_str!("../shaders/post_tone.wgsl"),
            include_str!("../shaders/post_raster_surface.wgsl"),
            &primary,
        ] {
            let module = naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }
}
