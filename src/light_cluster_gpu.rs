//! GPU half of LC1 (src/light_cluster.rs): the cluster buffer and its culling kernel, and
//! the pooled local shadow maps with their cached redraws.
use super::*;
use crate::light_cluster::{self as lc, LightClusterStats, LocalShadows};
use sha2::{Digest, Sha256};

/// Six Depth32Float atlas layers, with the same tile rectangle on each cube face.
pub(super) struct Atlas {
    _texture: Owned<wgpu::Texture>,
    pub view: Owned<wgpu::TextureView>,
    layers: Vec<Owned<wgpu::TextureView>>,
    /// One shadow-view uniform per layer at `stride`, read by the shared shadow raster.
    uniforms: Option<Owned<wgpu::Buffer>>,
    binds: Vec<Owned<wgpu::BindGroup>>,
    stride: u64,
    resolution: u32,
    slots: u32,
    clear: Option<Owned<wgpu::RenderPipeline>>,
    _allocation: Allocation,
}
impl Atlas {
    /// `slots` 0 is the 1x1 placeholder bound while no frame needs maps.
    fn new(
        device: &wgpu::Device,
        ledger: &Ledger,
        bindings: Option<&wgpu::BindGroupLayout>,
        resolution: u32,
        slots: u32,
    ) -> Self {
        let (size, layers) = if slots == 0 {
            (1, 1)
        } else {
            (resolution * 4, 6)
        };
        let texture = ledger.own(
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("LC1 local shadow maps"),
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: layers,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            }),
            ResourceKind::Texture,
        );
        let view = ledger.own(
            texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            }),
            ResourceKind::TextureView,
        );
        let alignment = u64::from(device.limits().min_uniform_buffer_offset_alignment);
        let stride = crate::shadows::VIEW_BYTES.div_ceil(alignment) * alignment;
        let (mut views, mut binds, mut uniforms) = (vec![], vec![], None);
        if slots > 0 {
            let buffer = ledger.own(
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("LC1 local shadow view matrices"),
                    size: u64::from(slots * 6) * stride,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            );
            for layer in 0..layers {
                views.push(ledger.own(
                    texture.create_view(&wgpu::TextureViewDescriptor {
                        dimension: Some(wgpu::TextureViewDimension::D2),
                        base_array_layer: layer,
                        array_layer_count: Some(1),
                        ..Default::default()
                    }),
                    ResourceKind::TextureView,
                ));
            }
            for layer in 0..slots * 6 {
                binds.push(ledger.own(
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("LC1 one local shadow view"),
                        layout: bindings.unwrap(),
                        entries: &[wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: &buffer,
                                offset: u64::from(layer) * stride,
                                size: wgpu::BufferSize::new(crate::shadows::VIEW_BYTES),
                            }),
                        }],
                    }),
                    ResourceKind::BindGroup,
                ));
            }
            uniforms = Some(buffer);
        }
        let bytes = u64::from(layers) * 4 * u64::from(size).pow(2)
            + uniforms.as_ref().map_or(0, |u| u.size());
        let clear = (slots > 0).then(|| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("LC3 tile clear"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("../shaders/local_light_clear.wgsl").into(),
                ),
            });
            ledger.own(
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("LC3 tile clear"),
                    layout: None,
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("vertex"),
                        buffers: &[],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some("fragment"),
                        targets: &[],
                        compilation_options: Default::default(),
                    }),
                    primitive: Default::default(),
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth32Float,
                        depth_write_enabled: Some(true),
                        depth_compare: Some(wgpu::CompareFunction::Always),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                }),
                ResourceKind::RenderPipeline,
            )
        });
        Self {
            _texture: texture,
            view,
            layers: views,
            uniforms,
            binds,
            stride,
            resolution,
            slots,
            clear,
            _allocation: ledger.allocate(AllocationKind::Shadow, bytes),
        }
    }
}

pub(super) struct LightClusters {
    #[cfg(test)]
    pub legacy_limit: bool,
    pub enabled: bool,
    pub settings: LocalShadows,
    /// The `LcFrame` uniform: the grid's camera and the pooled map matrices (group 3
    /// binding 23, and the kernel's binding 2).
    pub frame: Owned<wgpu::Buffer>,
    /// The cluster lists, R32Uint `LIST_SIZE` (group 3 binding 24).
    _lists: Owned<wgpu::Texture>,
    pub lists: Owned<wgpu::TextureView>,
    layout: Owned<wgpu::BindGroupLayout>,
    pipeline: Owned<wgpu::ComputePipeline>,
    /// The kernel's group, keyed on the light buffer generation it binds.
    bind: Option<(u64, Owned<wgpu::BindGroup>)>,
    /// Bound at group 3 binding 25; `generation` changes with it.
    pub atlas: Atlas,
    retired: Vec<(u64, Atlas)>,
    pub generation: u64,
    /// Each slot's map content (light placement, settings and casters), None to redraw.
    keys: Vec<Option<[u8; 32]>>,
    slots: Vec<Option<u32>>,
    strengths: Vec<f32>,
    tiles: Vec<Option<lc::Tile>>,
    pub stats: LightClusterStats,
    /// The frame uniform as last written: unchanged frames upload nothing.
    published: Vec<u8>,
    _allocation: Allocation,
}
/// One admitted frame's culling dispatch and map redraws.
pub(super) struct LocalFrame {
    dispatch: bool,
    /// Layers to redraw, each with its caster (draw, instance slot) pairs.
    redraw: Vec<(u32, Vec<(usize, u32)>)>,
}
impl LightClusters {
    pub fn new(device: &wgpu::Device, ledger: &Ledger) -> Self {
        let frame = ledger.own(
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("LC1 cluster frame"),
                size: lc::FRAME_BYTES as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        let lists = ledger.own(
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("LC1 light cluster lists"),
                size: wgpu::Extent3d {
                    width: lc::LIST_SIZE[0],
                    height: lc::LIST_SIZE[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R32Uint,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            }),
            ResourceKind::Texture,
        );
        let list_view = ledger.own(
            lists.create_view(&Default::default()),
            ResourceKind::TextureView,
        );
        let layout = ledger.own(
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("LC1 cluster kernel"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: wgpu::TextureFormat::R32Uint,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(lc::FRAME_BYTES as u64),
                        },
                        count: None,
                    },
                ],
            }),
            ResourceKind::BindGroupLayout,
        );
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("LC1 cluster kernel ABI"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("LC1 light clusters"),
            source: wgpu::ShaderSource::Wgsl(format!("{}{}", lc::constants(), lc::KERNEL).into()),
        });
        let pipeline = ledger.own(
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("LC1 light clusters"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some("lc_build"),
                compilation_options: Default::default(),
                cache: None,
            }),
            ResourceKind::ComputePipeline,
        );
        Self {
            #[cfg(test)]
            legacy_limit: false,
            enabled: true,
            settings: LocalShadows::default(),
            _allocation: ledger.allocate(
                AllocationKind::Light,
                u64::from(lc::LIST_SIZE[0] * lc::LIST_SIZE[1]) * 4 + lc::FRAME_BYTES as u64,
            ),
            frame,
            _lists: lists,
            lists: list_view,
            layout,
            pipeline,
            bind: None,
            atlas: Atlas::new(device, ledger, None, 1, 0),
            retired: vec![],
            generation: 0,
            keys: vec![],
            slots: vec![],
            strengths: vec![],
            tiles: vec![],
            stats: LightClusterStats::default(),
            published: vec![],
        }
    }
    pub fn collect(&mut self, completed: u64) {
        self.retired.retain(|(last, _)| *last > completed);
    }
    #[cfg(test)]
    pub fn test_lists(&self) -> &wgpu::Texture {
        &self._lists
    }
    #[cfg(test)]
    pub fn test_reset_selection(&mut self) {
        self.slots.clear();
        self.strengths.clear();
        self.tiles.clear();
    }
}

impl Renderer {
    /// LC1: how many point and spot lights are shadowed each frame, and their map settings.
    /// Applies from the next lit frame.
    pub fn set_local_shadows(&mut self, settings: LocalShadows) -> Result<(), Error> {
        self.check_fault()?;
        settings.validate(self.device.limits().max_texture_dimension_2d)?;
        self.lighting.clusters.settings = settings;
        Ok(())
    }
    pub fn local_shadows(&self) -> LocalShadows {
        self.lighting.clusters.settings
    }
    /// LC1 clustered culling, on by default. Off, every receiver walks every light (the
    /// pre-LC1 loop): for comparisons and debugging.
    pub fn set_light_clusters(&mut self, enabled: bool) {
        self.lighting.clusters.enabled = enabled;
    }
    pub fn light_cluster_stats(&self) -> LightClusterStats {
        self.lighting.clusters.stats.clone()
    }
    /// Per admitted lit frame, before `upload_lighting`: choose the shadowed lights and
    /// mark their records (kind.w), write the shading header and the kernel uniform, and
    /// plan the map faces whose light or casters changed. `maps` is false when the frame's
    /// shading modules trace local shadows (light_cluster::ray_scene).
    pub(super) fn prepare_light_clusters(
        &mut self,
        camera: &Camera,
        viewport: [u32; 2],
        draws: &[PreparedDraw<'_>],
        maps: bool,
        uploads: &mut UploadBytes,
    ) -> LocalFrame {
        let lights = &self.lighting.source_lights;
        let state = &mut self.lighting.clusters;
        let count = state.settings.count;
        for slot in &mut state.slots {
            if slot.is_some_and(|i| {
                lights
                    .get(i as usize)
                    .is_none_or(|l| lc::sphere(l).is_none())
            }) {
                *slot = None;
            }
        }
        let eye = camera.view().inverse().w_axis.truncate();
        let previous = state.slots.clone();
        let all_rays = !maps && count > 0;
        #[cfg(test)]
        let all_rays = all_rays && !state.legacy_limit;
        let slots = if count > 0 && !all_rays {
            let desired = lc::select(
                lights,
                camera.matrix(viewport[0], viewport[1]),
                eye,
                &state.slots,
                count,
            );
            #[cfg(test)]
            let desired = if state.legacy_limit {
                lc::select_legacy(
                    lights,
                    camera.matrix(viewport[0], viewport[1]),
                    eye,
                    &state.slots,
                    count,
                )
            } else {
                desired
            };
            let (slots, strengths) =
                lc::transition(&state.slots, &state.strengths, &desired, count);
            state.strengths = strengths;
            slots
        } else {
            vec![]
        };
        for (index, record) in self
            .lighting
            .pending
            .records
            .chunks_exact_mut(crate::lighting::LIGHT_BYTES)
            .enumerate()
        {
            let w = if all_rays && lc::sphere(&lights[index]).is_some() {
                // Rays address the light directly and need neither a map slot nor selection.
                (index as u32 & 0xffff) + 1
            } else {
                slots
                    .iter()
                    .position(|s| *s == Some(index as u32))
                    .map_or(0, |slot| {
                        (slot as u32 + 1) | if lc::cube(&lights[index]) { 0x10000 } else { 0 }
                    })
            };
            record[60..64].copy_from_slice(&w.to_ne_bytes());
        }
        let mut layers = vec![glam::Mat4::IDENTITY; count as usize * 6];
        let mut faces = vec![0usize; count as usize];
        state.tiles.resize(count as usize, None);
        if state.atlas.resolution != state.settings.resolution {
            state.tiles.fill(None);
        }
        for slot in 0..count as usize {
            if slots.get(slot).is_none_or(Option::is_none) || previous.get(slot) != slots.get(slot) {
                state.tiles[slot] = None;
            }
        }
        let vp = camera.matrix(viewport[0], viewport[1]);
        for (slot, light) in slots.iter().enumerate() {
            if let Some(light) = light {
                if state.tiles[slot].is_none() {
                    let pixels = lc::coverage(&lights[*light as usize], vp).sqrt()
                        * viewport[0].max(viewport[1]) as f32;
                    let maximum = state.settings.resolution;
                    let desired = if pixels > maximum as f32 * 0.5 {
                        maximum
                    } else if pixels > maximum as f32 * 0.25 {
                        (maximum / 2).max(1)
                    } else {
                        (maximum / 4).max(1)
                    };
                    state.tiles[slot] = lc::allocate_tile(&state.tiles, maximum, desired);
                }
                let m = lc::matrices(&lights[*light as usize], state.settings.near);
                faces[slot] = m.len();
                layers[slot * 6..slot * 6 + m.len()].copy_from_slice(&m);
            }
        }
        let clustered = state.enabled && lights.len() <= lc::MAX_CLUSTERED;
        let mut header = lc::header(camera, viewport, lights.len() as u32, clustered, &layers);
        for (slot, tile) in state.tiles.iter().enumerate() {
            if let Some(tile) = tile {
                for (k, value) in [
                    tile.x as f32,
                    tile.y as f32,
                    tile.size as f32,
                    state.strengths.get(slot).copied().unwrap_or(0.0),
                ]
                .into_iter()
                .enumerate()
                {
                    let offset = lc::TILE_OFFSET + slot * 16 + k * 4;
                    header[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
                }
            }
        }
        // LC2 source sampling rotates only with TR1; comparisons with history off remain
        // deterministic. Raster map filtering is deterministic on every frame.
        let sample = self
            .reconstruction_frame
            .map_or(0u32, |k| k.wrapping_add(1) as u32);
        header[88..92].copy_from_slice(&sample.to_ne_bytes());
        header[100..104].copy_from_slice(&state.settings.near.to_ne_bytes());
        if let Some(span) = crate::lighting::dirty_span(&state.published, &header) {
            self.queue
                .write_buffer(&state.frame, span.start as u64, &header[span.clone()]);
            uploads.lighting += (span.end - span.start) as u64;
            uploads.lighting_writes += 1;
            uploads.write_calls += 1;
            state.published = header;
        }
        let mut redraw = vec![];
        let maps = maps && slots.iter().any(Option::is_some);
        if maps {
            if state.atlas.slots != count || state.atlas.resolution != state.settings.resolution {
                let next = Atlas::new(
                    &self.device,
                    &self.ledger,
                    Some(&self.shadows.bindings),
                    state.settings.resolution,
                    count,
                );
                let old = std::mem::replace(&mut state.atlas, next);
                state.retired.push((self.submissions, old));
                state.generation += 1;
                state.keys = vec![None; count as usize];
            }
            // Hash each caster once, then include only intersecting casters in a map's
            // key. A distant moving object no longer invalidates the entire atlas.
            let mut candidates = vec![];
            for (d, draw) in draws.iter().enumerate() {
                for (i, model) in draw.instances.state.models.iter().enumerate() {
                    let Ok(bounds) = shadow_gpu::world_bounds(draw.mesh.bounds, *model) else {
                        continue;
                    };
                    let key: [u8; 32] = Sha256::new()
                        .chain_update((std::ptr::from_ref(draw.mesh) as usize).to_ne_bytes())
                        .chain_update(draw.mesh.index_count.to_ne_bytes())
                        .chain_update(
                            draw.mesh
                                .skin
                                .as_ref()
                                .map_or(0, |s| s.revision)
                                .to_ne_bytes(),
                        )
                        .chain_update(
                            draw.material
                                .map_or(0, |m| std::ptr::from_ref(m) as usize)
                                .to_ne_bytes(),
                        )
                        .chain_update(matrix_bytes(*model))
                        .finalize()
                        .into();
                    candidates.push((d, i as u32, bounds, key));
                }
            }
            for (slot, light) in slots.iter().enumerate() {
                let Some(light) = *light else {
                    state.keys[slot] = None;
                    continue;
                };
                let Some(tile) = state.tiles[slot] else {
                    continue;
                };
                let (centre, radius) = lc::sphere(&lights[light as usize]).unwrap();
                let mut casters = vec![];
                let mut scene = Sha256::new();
                for (d, i, b, key) in &candidates {
                    if centre.distance_squared(centre.clamp(b.min, b.max)) <= radius * radius {
                        casters.push((*d, *i));
                        scene.update(key);
                    }
                }
                let scene: [u8; 32] = scene.finalize().into();
                let record = &self.lighting.pending.records
                    [light as usize * crate::lighting::LIGHT_BYTES..]
                    [..crate::lighting::LIGHT_BYTES];
                let key: [u8; 32] = Sha256::new()
                    .chain_update(&record[..32])
                    .chain_update((light as u32).to_ne_bytes())
                    .chain_update(tile.x.to_ne_bytes())
                    .chain_update(tile.y.to_ne_bytes())
                    .chain_update(tile.size.to_ne_bytes())
                    .chain_update(state.settings.near.to_ne_bytes())
                    .chain_update(state.settings.resolution.to_ne_bytes())
                    .chain_update(scene)
                    .finalize()
                    .into();
                if state.keys[slot] == Some(key) {
                    continue;
                }
                state.keys[slot] = Some(key);
                for face in 0..faces[slot] {
                    let layer = (slot * 6 + face) as u32;
                    self.queue.write_buffer(
                        state.atlas.uniforms.as_ref().unwrap(),
                        u64::from(layer) * state.atlas.stride,
                        &matrix_bytes(layers[layer as usize]),
                    );
                    uploads.shadows += 64;
                    uploads.write_calls += 1;
                    redraw.push((layer, casters.clone()));
                }
            }
        } else {
            state.keys.iter_mut().for_each(|k| *k = None);
        }
        let stats = &mut state.stats;
        stats.clustered = clustered;
        stats.shadowed = if all_rays {
            lights
                .iter()
                .enumerate()
                .filter(|(_, l)| lc::sphere(l).is_some())
                .map(|(i, _)| Some(i as u32))
                .collect()
        } else {
            slots.clone()
        };
        stats.resolutions = if maps {
            state
                .tiles
                .iter()
                .map(|t| t.map_or(0, |t| t.size))
                .collect()
        } else {
            vec![]
        };
        stats.strengths = if maps {
            state.strengths.clone()
        } else {
            vec![1.0; stats.shadowed.len()]
        };
        stats.atlas_bytes = if maps {
            6 * 4 * u64::from(state.atlas.resolution * 4).pow(2)
        } else {
            0
        };
        state.slots = slots;
        stats.maps = maps;
        stats.faces_drawn = redraw.len() as u32;
        stats.total_faces_drawn += redraw.len() as u64;
        stats.caster_draws = redraw.iter().map(|(_, c)| c.len() as u32).sum();
        LocalFrame {
            dispatch: clustered,
            redraw,
        }
    }
    /// The culling dispatch and map redraws, before any shading pass of the frame.
    pub(super) fn encode_light_clusters(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &LocalFrame,
        draws: &[PreparedDraw<'_>],
    ) {
        let generation = self.lighting.generation;
        let lights = self.lighting.light_buffer().clone();
        let state = &mut self.lighting.clusters;
        if frame.dispatch {
            if state.bind.as_ref().is_none_or(|(g, _)| *g != generation) {
                let bind = self.ledger.own(
                    self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("LC1 cluster kernel"),
                        layout: &state.layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: lights.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&state.lists),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: state.frame.as_entire_binding(),
                            },
                        ],
                    }),
                    ResourceKind::BindGroup,
                );
                state.bind = Some((generation, bind));
            }
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("LC1 light clusters"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&state.pipeline);
            pass.set_bind_group(0, &*state.bind.as_ref().unwrap().1, &[]);
            pass.dispatch_workgroups(lc::CLUSTERS.div_ceil(64), 1, 1);
        }
        let state = &self.lighting.clusters;
        for (layer, casters) in &frame.redraw {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("LC1 local shadow face"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &state.atlas.layers[*layer as usize % 6],
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            let tile = state.tiles[*layer as usize / 6].unwrap();
            pass.set_viewport(
                tile.x as f32,
                tile.y as f32,
                tile.size as f32,
                tile.size as f32,
                0.0,
                1.0,
            );
            pass.set_scissor_rect(tile.x, tile.y, tile.size, tile.size);
            pass.set_pipeline(state.atlas.clear.as_ref().unwrap());
            pass.draw(0..3, 0..1);
            pass.set_pipeline(&self.pipeline.shadow);
            pass.set_bind_group(0, &*state.atlas.binds[*layer as usize], &[]);
            let (mut instances, mut material, mut mesh) = (None, None, None);
            for &(d, slot) in casters {
                let draw = &draws[d];
                let bind: &wgpu::BindGroup = &draw.instances.bind;
                if changed(&mut instances, bind) {
                    pass.set_bind_group(1, bind, &[]);
                }
                let bind: &wgpu::BindGroup = &draw
                    .material
                    .unwrap_or(self.default_material.as_ref().unwrap())
                    .bind;
                if changed(&mut material, bind) {
                    pass.set_bind_group(2, bind, &[]);
                }
                if changed(&mut mesh, draw.mesh) {
                    pass.set_vertex_buffer(0, draw.mesh.vertex.slice(..));
                    pass.set_index_buffer(draw.mesh.index.slice(..), wgpu::IndexFormat::Uint32);
                }
                pass.draw_indexed(0..draw.mesh.index_count, 0, slot..slot + 1);
            }
        }
    }
}
