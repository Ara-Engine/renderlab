//! T1 temporal film accumulation (docs/m6-temporal-primaries.md), opt-in through
//! `SoftShadows::Temporal` on traced soft frames. Each frame traces one jittered primary
//! per pixel with Interactive's eight disk directions, rotated by the pixel's sequence
//! index, shades it with the frame's program, and accumulates the program's output into
//! an engine-owned per-pixel history: the film reconstruction. A reset frame reads no
//! history and is traced and shaded by the production Interactive pipelines, so it is the
//! Interactive image byte for byte; `film_init` then starts every history from it.
//!
//! The temporal modules are the Interactive kernel and Soft shading compositions with the
//! `FILM_*_SEAM` edits applied, so Exact, Interactive and hard modules are unchanged.
//! T1a accumulates a static view: with `FilmMotion::Restart` any change to the camera, scene,
//! lighting, shadows, programs or parameters resets every pixel. With sample records
//! (`film_records.rs`, T2) camera changes move the records instead, and static frames are T1a's.
use super::*;
use crate::program::{self, Composed, Stage};
use super::film_records::FilmRecords;

/// Sample sequence, `film_index`/`film_offset`/`film_begin`, appended to both modules.
pub(super) const FILM_SEQUENCE: &str = include_str!("../shaders/film_accumulation.wgsl");
/// `film_store`, between the program and the temporal shading entry.
const FILM_STORE: &str = include_str!("../shaders/film_store.wgsl");
/// `film_init`, the reset frames' history start, in the temporal kernel module.
const FILM_INIT: &str = include_str!("../shaders/film_init.wgsl");
/// Rank-1 lattice generator of the sample sequence; WGSL `FILM_GENERATOR` must match.
#[cfg(test)]
const GENERATOR: u32 = 71;

/// The camera ray through `film_sample` instead of the pixel centre (primary_ray).
pub(super) const FILM_RAY_SEAM: (&str, &str) = (
    "(vec2<f32>(pixel)+vec2<f32>(0.5))",
    "(vec2<f32>(pixel)+film_sample)",
);
/// Disk directions 8 (k mod 32) onward of the pixel's Exact sequence (fresh_estimate).
pub(super) const FILM_ROTATION_SEAM: (&str, &str) = (
    "let direction=fresh_direction(j,shift,l,u,v,view.texel.z);",
    "let direction=fresh_direction(j+film_rotation,shift,l,u,v,view.texel.z);",
);
/// The fused Interactive trace selects the pixel's sample before tracing it.
const FILM_TRACE_SEAM: (&str, &str) = (
    "    let inside=all(id.xy<size);\n",
    "    let inside=all(id.xy<size);\n    if inside {film_begin(film_index(vec2<i32>(id.xy)),id.xy,size.x);}\n",
);
/// After the trace publishes a tile's coverage, it records whether the tile keeps history.
const FILM_TILE_SEAM: (&str, &str) = (
    "    if all(id.xy==group.xy*PRIMARY_TILE) {textureStore(primary_tile_output,group.xy,vec4<u32>(covered));}\n",
    "    if all(id.xy==group.xy*PRIMARY_TILE) {textureStore(primary_tile_output,group.xy,vec4<u32>(covered));}\n    if all(id.xy==group.xy*PRIMARY_TILE) {film_keep_tile(group.xy,covered);}\n",
);
/// The shading entry's stores, each routed through `film_store`, and the sample
/// selection before its gradients read the camera ray.
const FILM_ENTRY_SEAMS: [(&str, &str); 6] = [
    (
        "{textureStore(primary_hdr,vec2<i32>(p),primary_frame.clear);}",
        "{film_store(vec2<i32>(p),film_background(),0u,0.0);}",
    ),
    (
        "textureStore(primary_hdr,vec2<i32>(id.xy),primary_frame.clear);",
        "film_store(vec2<i32>(id.xy),film_background(),0u,0.0);",
    ),
    (
        "let pixel=vec2<i32>(id);let record=textureLoad(primary_hit,pixel,0);",
        "let pixel=vec2<i32>(id);let record=textureLoad(primary_hit,pixel,0);\n    film_begin(film_index(pixel),id,size.x);",
    ),
    (
        "if record.x==0u {textureStore(primary_hdr,pixel,primary_frame.clear);return;}",
        "if record.x==0u {film_store(pixel,film_background(),0u,0.0);return;}",
    ),
    (
        "{textureStore(primary_hdr,pixel,vec4<f32>(0.0));return;}",
        "{film_store(pixel,vec3<f32>(0.0),record.x,film_view_distance(ta.position,tb.position,tc.position,w,hit.placement));return;}",
    ),
    (
        "textureStore(primary_hdr,pixel,vec4<f32>(rl_finite(shade(surface_output,surface_input)),1.0));",
        "film_store(pixel,rl_finite(shade(surface_output,surface_input)),record.x,film_view_distance(ta.position,tb.position,tc.position,w,hit.placement));",
    ),
];

pub(super) fn seam(source: &str, (find, replace): (&str, &str)) -> Result<String, Error> {
    if source.matches(find).count() != 1 {
        return Err(Error::InvalidInput(
            "temporal film accumulation requires the unchanged engine seams it edits",
        ));
    }
    Ok(source.replacen(find, replace, 1))
}

/// The temporal trace module from the soft kernel module (`shader_sources()[1]`): the
/// fused Interactive trace with jittered camera rays and rotated disk directions.
pub(crate) fn kernel_source(soft_kernels: &str) -> Result<String, Error> {
    let mut source = soft_kernels.to_owned();
    for edit in [
        FILM_RAY_SEAM,
        FILM_ROTATION_SEAM,
        FILM_TRACE_SEAM,
        FILM_TILE_SEAM,
    ] {
        source = seam(&source, edit)?;
    }
    Ok(format!("{source}\n{FILM_SEQUENCE}\n{FILM_INIT}"))
}

/// Every program's temporal shading head from the Soft shading head
/// (`shading_heads()[1]`): its gradients follow the jittered camera ray.
pub(crate) fn shading_head(soft_head: &str) -> Result<String, Error> {
    Ok(format!(
        "{}\n{FILM_SEQUENCE}",
        seam(soft_head, FILM_RAY_SEAM)?
    ))
}

/// One program's temporal shading module on `head` (`shading_head`): the traced
/// composition, with every store of its entry routed through `film_store`. The program's
/// byte range, which diagnostics map lines through, is unchanged.
pub(crate) fn compose_shading(head: &str, source: &str, parameters: bool) -> Composed {
    let mut composed = program::compose(head, source, Stage::Traced, parameters);
    let mut entry = composed.source.split_off(composed.program.end);
    for edit in FILM_ENTRY_SEAMS {
        // The entry is engine text after the program, so these always apply.
        entry = seam(&entry, edit).expect("temporal shading entry seam");
    }
    composed.source.push('\n');
    composed.source.push_str(FILM_STORE);
    composed.source.push_str(&entry);
    composed
}

/// The lattice node of index k before the move off the pixel edges, in 1/256 pixel units,
/// for symmetry `s` (0..4).
#[cfg(test)]
fn lattice_node(k: u32, s: u32) -> [u32; 2] {
    let order = if k < 4 { (0x0231 >> (4 * k)) & 0xf } else { k };
    let j = order.reverse_bits() >> 24;
    let mut p = [j, (GENERATOR * j) & 0xff];
    if s & 1 != 0 {
        p = p.map(|v| (256 - v) & 0xff);
    }
    if s & 2 != 0 {
        p.swap(0, 1);
    }
    p
}

/// CPU mirror of WGSL `film_offset` in 1/512 pixel units: the exact centre for k = 0, else
/// the lattice node moved (8.5, 8.5) / 256 px.
#[cfg(test)]
fn sample_point(k: u32, s: u32) -> [u32; 2] {
    if k == 0 {
        return [256, 256];
    }
    lattice_node(k, s).map(|v| 2 * ((v + 8) & 0xff) + 1)
}

/// Bind group and pipeline layouts of the temporal trace and shading: the fused
/// Interactive and Soft shading ABIs plus the film history.
pub(super) struct FilmLayouts {
    trace_bindings: Owned<wgpu::BindGroupLayout>,
    shade_bindings: Owned<wgpu::BindGroupLayout>,
    init_bindings: Owned<wgpu::BindGroupLayout>,
    pub trace: Owned<wgpu::PipelineLayout>,
    pub shade: Owned<wgpu::PipelineLayout>,
    pub init: Owned<wgpu::PipelineLayout>,
}
impl FilmLayouts {
    pub fn new(
        device: &wgpu::Device,
        ledger: &Ledger,
        arrays: &wgpu::BindGroupLayout,
        materials: &wgpu::BindGroupLayout,
        lighting: &wgpu::BindGroupLayout,
    ) -> Self {
        use wgpu::TextureFormat as F;
        let uint = wgpu::TextureSampleType::Uint;
        let group = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            ledger.own(
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some(label),
                    entries,
                }),
                ResourceKind::BindGroupLayout,
            )
        };
        let frame = uniform(10, FRAME_BYTES as u64);
        let film = [read(24, 16), texture(25, uint)];
        let mut trace = vec![
            scene(TRACE_SCENE),
            scene(2),
            frame,
            image(13, F::Rgba32Uint),
            image(5, F::R32Uint),
            image(20, F::R32Uint),
            texture(29, uint),
            image(30, F::R32Uint),
        ];
        trace.extend(film);
        let mut shade = vec![
            uniform(0, 80),
            texture(7, uint),
            frame,
            image(18, F::Rgba16Float),
            texture(21, uint),
            read(22, 16),
            texture(6, uint),
            texture(26, wgpu::TextureSampleType::Float { filterable: false }),
            image(27, F::Rgba32Float),
            image(28, F::Rg32Uint),
            texture(29, uint),
            texture(30, uint),
        ];
        shade.extend(film);
        let trace_bindings = group("temporal film trace", &trace);
        let shade_bindings = group("temporal film shading", &shade);
        let init_bindings = group(
            "temporal film reset",
            &[
                texture(7, uint),
                texture(21, uint),
                texture(18, wgpu::TextureSampleType::Float { filterable: false }),
                image(27, F::Rgba32Float),
                image(28, F::Rg32Uint),
                read(24, 16),
                image(30, F::R32Uint),
            ],
        );
        let layout = |label, groups: &[Option<&wgpu::BindGroupLayout>]| {
            ledger.own(
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(label),
                    bind_group_layouts: groups,
                    immediate_size: 0,
                }),
                ResourceKind::PipelineLayout,
            )
        };
        Self {
            trace: layout(
                "temporal film trace ABI",
                &[Some(&trace_bindings), Some(arrays), None, Some(lighting)],
            ),
            shade: layout(
                "temporal film shading ABI",
                &[
                    Some(&shade_bindings),
                    Some(arrays),
                    Some(materials),
                    Some(lighting),
                ],
            ),
            init: layout(
                "temporal film reset ABI",
                &[Some(&init_bindings), Some(arrays), None, Some(lighting)],
            ),
            trace_bindings,
            shade_bindings,
            init_bindings,
        }
    }
}

/// The camera a temporal frame was rendered with, as the record passes reproject it.
#[derive(Clone, Copy)]
struct FilmView {
    /// View-projection at the HDR size (primary_frame's matrix before inversion).
    matrix: Mat4,
    /// The same with the far plane at infinity, which records are expressed in.
    infinite: Mat4,
    /// The far plane's depth in `infinite`: near / far.
    far_depth: f32,
    /// The eye the lighting uniform holds.
    eye: Vec3,
    /// Radians per pixel at the image centre: 2 tan(fov / 2) / height.
    angle: f32,
}

/// Per-pixel film history at the HDR size, ping-ponged by frame: the running mean of the
/// program's exposed output with its sample weight (Rgba32Float) and the anchor
/// (Rg32Uint, film_accumulation.wgsl), 48 bytes per pixel, used only in trace tiles a camera
/// ray has hit since the last reset (`tiles`, one R32Uint flag per tile, also ping-ponged).
/// With `FilmMotion` records it also holds the sample records (`records`).
pub(in crate::renderer) struct FilmState {
    colour: [Depth; 2],
    anchor: [Depth; 2],
    tiles: [Depth; 2],
    frame: Buffer,
    /// The FilmFrame words the frame buffer holds.
    written: Option<[u32; 4]>,
    /// Frames at the current camera before this one since the last reset or camera motion
    /// (T1a's `frames`: with `FilmMotion::Restart` the camera never moves without a reset).
    frames: u32,
    /// The last frame's camera.
    view: Option<FilmView>,
    /// `cut_film` was called: the next frame resets.
    cut: bool,
    /// This frame moved the camera without a reset (records only).
    pub moving: bool,
    /// Sample records, with `FilmMotion::Records4` or `Records8`.
    pub records: Option<FilmRecords>,
    /// Keep history in every tile (tests compare it with the production tile rule).
    #[cfg(test)]
    pub materialize: bool,
    /// The side this frame writes; the other side holds the previous frame.
    pub parity: usize,
    /// Identity of everything the last accumulated frame's image depended on.
    key: Option<[u8; 32]>,
    /// The last frame reset every pixel.
    pub reset: bool,
    /// Trace, shading and reset bind groups per written side, for one primary bindings
    /// generation.
    groups: Option<(u64, [[Owned<wgpu::BindGroup>; 3]; 2])>,
}
impl FilmState {
    /// `records` is K, the records per pixel, or 0 without records.
    fn new(r: &Renderer, size: wgpu::Extent3d, records: u32) -> Self {
        let target = |label, size: wgpu::Extent3d, format: wgpu::TextureFormat| {
            let texture = r.ledger.own(
                r.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::STORAGE_BINDING
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | if cfg!(test) {
                            wgpu::TextureUsages::COPY_SRC
                        } else {
                            wgpu::TextureUsages::empty()
                        },
                    view_formats: &[],
                }),
                ResourceKind::Texture,
            );
            let view = r.ledger.own(
                texture.create_view(&Default::default()),
                ResourceKind::TextureView,
            );
            let bytes = u64::from(size.width)
                * u64::from(size.height)
                * u64::from(format.block_copy_size(None).unwrap());
            Depth {
                texture,
                view,
                _allocation: r.ledger.allocate(AllocationKind::Target, bytes),
            }
        };
        let tile_size = wgpu::Extent3d {
            width: size.width.div_ceil(TRACE_TILE[0]),
            height: size.height.div_ceil(TRACE_TILE[1]),
            depth_or_array_layers: 1,
        };
        let colour = || {
            target(
                "film history colour",
                size,
                wgpu::TextureFormat::Rgba32Float,
            )
        };
        let anchor = || target("film history anchors", size, wgpu::TextureFormat::Rg32Uint);
        let tiles = || {
            target(
                "film history tiles",
                tile_size,
                wgpu::TextureFormat::R32Uint,
            )
        };
        Self {
            colour: [colour(), colour()],
            anchor: [anchor(), anchor()],
            tiles: [tiles(), tiles()],
            frame: Buffer::new(r, "film frame state", 16, wgpu::BufferUsages::STORAGE),
            written: None,
            frames: 0,
            view: None,
            cut: false,
            moving: false,
            records: (records > 0).then(|| FilmRecords::new(r, size, records)),
            #[cfg(test)]
            materialize: r.primary.as_ref().unwrap().test_film_materialize,
            parity: 0,
            key: None,
            reset: true,
            groups: None,
        }
    }
    fn size(&self) -> wgpu::Extent3d {
        self.colour[0].texture.size()
    }
    /// The history colour, anchor and tile-flag views of `side`.
    pub(super) fn views(&self, side: usize) -> [&wgpu::TextureView; 3] {
        [
            &self.colour[side].view,
            &self.anchor[side].view,
            &self.tiles[side].view,
        ]
    }
    /// The history and tile-flag textures of `side` (tests).
    #[cfg(test)]
    pub fn history(&self, side: usize) -> [&wgpu::Texture; 3] {
        [
            &self.colour[side].texture,
            &self.anchor[side].texture,
            &self.tiles[side].texture,
        ]
    }
    /// The last frame's shading group (tests re-shade its samples with it).
    #[cfg(test)]
    pub fn shade_group(&self) -> &wgpu::BindGroup {
        &self.groups.as_ref().unwrap().1[self.parity][1]
    }
}

impl Renderer {
    /// Identity of every input a temporal frame's image depends on besides the frame
    /// counter: the camera, prelude/program revision and program, every GPU write that
    /// changes geometry, transforms, materials, parameters, textures, lights, lighting,
    /// exposure or the environment, the shadow settings and views, the TLAS and draw
    /// tables. Every pixel resets when it changes. With sample records the camera moves the
    /// records instead, so the key leaves out what follows the camera: the camera itself, the
    /// lighting uniform's eye and the camera-fitted shadow views, whose light-dependent
    /// content follows the light records and settings.
    pub(in crate::renderer) fn film_key(
        &self,
        camera_bytes: &[u8; 64],
        shadow: Option<&[u8]>,
    ) -> [u8; 32] {
        let u = &self.uploads;
        let state = self.primary.as_ref().unwrap();
        let moves = self.film_motion.records() > 0;
        let mut hash = Sha256::new()
            .chain_update(self.revision.0.to_ne_bytes())
            .chain_update(state.program.to_ne_bytes())
            .chain_update(state.geometry_generation.to_ne_bytes())
            .chain_update(state.bindings_generation.to_ne_bytes())
            .chain_update(format!("{:?}", self.shadows.settings).as_bytes());
        if moves {
            hash.update(self.lighting.pending.uniform(Vec3::ZERO));
            hash.update(self.environment.uniform());
        } else {
            hash.update(camera_bytes);
            hash.update(u.lighting.to_ne_bytes());
            if let Some(records) = shadow {
                hash.update(records);
            }
        }
        for v in [
            u.environment,
            u.geometry,
            u.lights,
            u.lut,
            u.textures,
            u.materials,
            u.normals,
            u.models,
            u.instances,
        ] {
            hash.update(v.to_ne_bytes());
        }
        hash.finalize().into()
    }

    /// Camera cut: the next temporal frame reads no history and restarts every pixel from
    /// the Interactive image. With sample records, camera changes are otherwise motion.
    pub fn cut_film(&mut self) {
        if let Some(film) = self.primary.as_mut().and_then(|p| p.film.as_mut()) {
            film.cut = true;
        }
    }

    /// Publishes a temporal frame's history after `prepare_primary`: allocates it at the
    /// HDR size, resets every pixel when `key` changed or on a cut, otherwise marks camera
    /// motion for the records, flips the written side and binds the frame's targets.
    pub(in crate::renderer) fn prepare_film(
        &mut self,
        key: [u8; 32],
        camera: &Camera,
    ) -> Result<(), Error> {
        let size = self.lighting.hdr.as_ref().unwrap().resolved.texture.size();
        let records = self.film_motion.records();
        let stale = self
            .primary
            .as_ref()
            .unwrap()
            .film
            .as_ref()
            .is_none_or(|f| {
                f.size() != size || f.records.as_ref().map_or(0, |r| r.count) != records
            });
        if stale {
            let next = FilmState::new(self, size, records);
            let submissions = self.submissions;
            let state = self.primary.as_mut().unwrap();
            if let Some(old) = state.film.replace(next) {
                state
                    .retired
                    .push((submissions, Retired::Film(Box::new(old))));
            }
        }
        let state = self.primary.as_mut().unwrap();
        let generation = state.bindings_generation;
        let placements = state.placement_count;
        let film = state.film.as_mut().unwrap();
        // The same matrix and eye as primary_frame and the lighting uniform.
        let view = FilmView {
            matrix: camera.matrix(size.width, size.height),
            infinite: camera.infinite_matrix(size.width, size.height),
            far_depth: camera.near() / camera.far(),
            eye: camera.view().inverse().w_axis.truncate(),
            angle: 2.0 * (0.5 * camera.fov_y()).tan() / size.height as f32,
        };
        let previous = film.view.replace(view);
        film.reset = film.key != Some(key) || film.cut || previous.is_none();
        film.key = Some(key);
        film.cut = false;
        let previous = previous.filter(|_| !film.reset).unwrap_or(view);
        let was_moving = film.moving;
        // Without records the key holds the camera, so a camera change always resets.
        film.moving = !film.reset && matrix_bytes(previous.matrix) != matrix_bytes(view.matrix);
        if let Some(records) = film.records.as_mut() {
            records.moving = film.moving;
            records.seed = !film.reset && !film.moving && was_moving;
        }
        film.parity = 1 - film.parity;
        film.frames = if film.reset || film.moving {
            0
        } else {
            film.frames.saturating_add(1)
        };
        #[cfg(test)]
        let materialize = u32::from(film.materialize);
        #[cfg(not(test))]
        let materialize = 0;
        let words = [u32::from(film.reset), film.frames, materialize, 0];
        if film.written != Some(words) {
            let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_ne_bytes()).collect();
            self.queue.write_buffer(&film.frame.value, 0, &bytes);
            film.written = Some(words);
            self.uploads.view_settings += 16;
            self.uploads.write_calls += 1;
        }
        if film.groups.as_ref().is_none_or(|g| g.0 != generation) {
            let groups = self.film_groups();
            let state = self.primary.as_mut().unwrap();
            let film = state.film.as_mut().unwrap();
            if let Some((_, old)) = film.groups.replace((generation, groups)) {
                // Frames in flight may still bind them.
                state
                    .retired
                    .push((self.submissions, Retired::FilmGroups(Box::new(old))));
            }
        }
        if records > 0 {
            self.prepare_records(
                previous.infinite,
                view.infinite,
                view.eye,
                view.angle,
                view.far_depth,
                placements,
            )?;
        }
        Ok(())
    }

    /// Trace, shading and reset groups for each written side over the current targets.
    fn film_groups(&self) -> [[Owned<wgpu::BindGroup>; 3]; 2] {
        use wgpu::BindingResource as R;
        let state = self.primary.as_ref().unwrap();
        let film = state.film.as_ref().unwrap();
        let t = state.targets.as_ref().unwrap();
        let f = self.fresh.as_ref().unwrap().targets.as_ref().unwrap();
        let hdr = &self.lighting.hdr.as_ref().unwrap().resolved;
        let hardware = self.hardware_shadows.as_ref().unwrap();
        let frame = &state.frame.as_ref().unwrap().0.value;
        let region = &state.region_buffer.as_ref().unwrap().value;
        let group = |layout: &wgpu::BindGroupLayout, entries: &[(u32, R<'_>)]| {
            let entries: Vec<_> = entries
                .iter()
                .map(|(binding, resource)| wgpu::BindGroupEntry {
                    binding: *binding,
                    resource: resource.clone(),
                })
                .collect();
            self.ledger.own(
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("temporal film bindings"),
                    layout,
                    entries: &entries,
                }),
                ResourceKind::BindGroup,
            )
        };
        [0, 1].map(|written| {
            let read = 1 - written;
            let film_entries = [
                (24, film.frame.value.as_entire_binding()),
                (25, R::TextureView(&film.anchor[read].view)),
            ];
            let mut trace = vec![
                (TRACE_SCENE, hardware.scene().as_binding()),
                (2, hardware.scene().as_binding()),
                (10, frame.as_entire_binding()),
                (13, R::TextureView(&t.hit.view)),
                (5, R::TextureView(&f.visibility.view)),
                (20, R::TextureView(&t.tiles.view)),
                (29, R::TextureView(&film.tiles[read].view)),
                (30, R::TextureView(&film.tiles[written].view)),
            ];
            trace.extend(film_entries.clone());
            let mut shade = vec![
                (0, self.uniform.as_entire_binding()),
                (7, R::TextureView(&t.hit.view)),
                (10, frame.as_entire_binding()),
                (18, R::TextureView(&hdr.view)),
                (21, R::TextureView(&t.tiles.view)),
                (22, region.as_entire_binding()),
                (6, R::TextureView(&f.visibility.view)),
                (26, R::TextureView(&film.colour[read].view)),
                (27, R::TextureView(&film.colour[written].view)),
                (28, R::TextureView(&film.anchor[written].view)),
                (29, R::TextureView(&film.tiles[read].view)),
                (30, R::TextureView(&film.tiles[written].view)),
            ];
            shade.extend(film_entries);
            let init = [
                (7, R::TextureView(&t.hit.view)),
                (21, R::TextureView(&t.tiles.view)),
                (18, R::TextureView(&hdr.view)),
                (27, R::TextureView(&film.colour[written].view)),
                (28, R::TextureView(&film.anchor[written].view)),
                (24, film.frame.value.as_entire_binding()),
                (30, R::TextureView(&film.tiles[written].view)),
            ];
            [
                group(&state.film_layouts.trace_bindings, &trace),
                group(&state.film_layouts.shade_bindings, &shade),
                group(&state.film_layouts.init_bindings, &init),
            ]
        })
    }

    /// The temporal kernel, compiled from the installed prelude on the first temporal
    /// frame (and again after a prelude reload), so other modes never pay for it.
    pub(in crate::renderer) fn ensure_film_kernel(&mut self) -> Result<(), Error> {
        if self.pipeline.primary.as_ref().unwrap().film.is_some() {
            return Ok(());
        }
        let source = kernel_source(&self.primary_sources.as_ref().unwrap().0[1])?;
        let scopes = OperationScopes::new(&self.device);
        let pipeline =
            self.primary
                .as_ref()
                .unwrap()
                .film_kernel(&self.device, &self.ledger, &source);
        if let Some(error) = scopes.finish() {
            return Err(match error {
                wgpu::Error::Validation { .. } => Error::Shader(error.to_string()),
                _ => self.fail(error.to_string()),
            });
        }
        self.pipelines.created += 2;
        self.pipeline.primary.as_mut().unwrap().film = Some(pipeline);
        Ok(())
    }

    /// One temporal frame: the jittered trace with its rotated eight-direction counts, then
    /// the program's temporal shading over the scene region and clear blocks. A reset frame
    /// is the production Interactive frame followed by `film_init` (and the records' start).
    /// With records, a moving frame is the record passes, and the first static frame after
    /// motion seeds T1a's history from the records first.
    pub(in crate::renderer) fn encode_film(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        trace_stamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
        compute_stamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
        shade_stamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let state = self.primary.as_ref().unwrap();
        let film = state.film.as_ref().unwrap();
        let groups = &film.groups.as_ref().unwrap().1[film.parity];
        let size = state.targets.as_ref().unwrap().hit.texture.size();
        let pipelines = self.pipeline.primary.as_ref().unwrap();
        if film.reset {
            self.encode_primary_interactive(
                encoder,
                &pipelines.interactive,
                TRACE_TILE,
                trace_stamps,
            );
            self.encode_primary_shading(encoder, Shading::Soft, shade_stamps);
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("temporal film reset"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipelines.film.as_ref().unwrap()[1]);
            pass.set_bind_group(0, &*groups[2], &[]);
            pass.set_bind_group(1, &*state.geometry.as_ref().unwrap().arrays, &[]);
            pass.set_bind_group(3, self.lighting.bind(), &[]);
            pass.dispatch_workgroups(
                size.width.div_ceil(TRACE_TILE[0]),
                size.height.div_ceil(TRACE_TILE[1]),
                1,
            );
            drop(pass);
            if film.records.is_some() {
                self.encode_records_init(encoder);
            }
            return;
        }
        if film.moving {
            self.encode_records_motion(encoder, trace_stamps, compute_stamps, shade_stamps);
            return;
        }
        if film.records.as_ref().is_some_and(|r| r.seed) {
            self.encode_records_seed(encoder, 1 - film.parity);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("temporal film trace"),
                timestamp_writes: trace_stamps,
            });
            pass.set_pipeline(&pipelines.film.as_ref().unwrap()[0]);
            pass.set_bind_group(0, &*groups[0], &[]);
            pass.set_bind_group(1, &*state.geometry.as_ref().unwrap().arrays, &[]);
            pass.set_bind_group(3, self.lighting.bind(), &[]);
            pass.dispatch_workgroups(
                size.width.div_ceil(TRACE_TILE[0]),
                size.height.div_ceil(TRACE_TILE[1]),
                1,
            );
        }
        let program = self.program_slot(state.program);
        let pipeline = program.pipelines.temporal.as_ref().unwrap();
        self.encode_shade_group(encoder, pipeline, &groups[1], SHADE_TILE, shade_stamps);
    }
}

/// Test-only local verification of film-solid reference pixels.
#[cfg(test)]
impl Renderer {
    /// Runs support/film_t1_footprint.wgsl (`source`, composed on the temporal kernel module)
    /// after a traced frame: one dispatch of `grid` x `grid` sub-pixel samples per 80-byte
    /// FootprintTarget in `targets`, each drained before the next. Returns the 32-byte
    /// FootprintSums per target.
    pub(in crate::renderer) fn test_film_footprint(
        &mut self,
        source: &str,
        targets: &[u8],
        grid: u32,
    ) -> Vec<u8> {
        let count = targets.len() / 80;
        let state = self.primary.as_ref().unwrap();
        let device = self.device.clone();
        let group = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };
        let zero = group(
            "footprint trace",
            &[
                scene(TRACE_SCENE),
                scene(2),
                uniform(10, FRAME_BYTES as u64),
            ],
        );
        let own = group(
            "footprint targets",
            &[
                read(0, 80),
                buffer(1, wgpu::BufferBindingType::Storage { read_only: false }, 32),
                uniform(2, 16),
            ],
        );
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("footprint ABI"),
            bind_group_layouts: &[
                Some(&zero),
                Some(&state.array_bindings),
                Some(&own),
                Some(&self.lighting.bindings),
            ],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("footprint verification"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("footprint verification"),
            layout: Some(&layout),
            module: &module,
            entry_point: Some("film_footprint"),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &[("FRESH_DIRECTIONS", 32.0)],
                ..Default::default()
            },
            cache: None,
        });
        let make = |label, size, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let copy = wgpu::BufferUsages::COPY_DST;
        let target_buffer = make(
            "footprint targets",
            targets.len() as u64,
            wgpu::BufferUsages::STORAGE | copy,
        );
        self.queue.write_buffer(&target_buffer, 0, targets);
        let sums = make(
            "footprint sums",
            (count * 32) as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | copy,
        );
        // Every word starts at zero except `nearest`, a minimum.
        let initial: Vec<u8> = (0..count * 8)
            .flat_map(|w| if w % 8 == 6 { u32::MAX } else { 0 }.to_ne_bytes())
            .collect();
        self.queue.write_buffer(&sums, 0, &initial);
        let params = make(
            "footprint parameters",
            16,
            wgpu::BufferUsages::UNIFORM | copy,
        );
        let hardware = self.hardware_shadows.as_ref().unwrap();
        let frame = &state.frame.as_ref().unwrap().0.value;
        let zero_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("footprint trace"),
            layout: &zero,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: TRACE_SCENE,
                    resource: hardware.scene().as_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: hardware.scene().as_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: frame.as_entire_binding(),
                },
            ],
        });
        let own_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("footprint targets"),
            layout: &own,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: target_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: sums.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params.as_entire_binding(),
                },
            ],
        });
        for index in 0..count as u32 {
            let words: Vec<u8> = [grid, index, 0, 0]
                .iter()
                .flat_map(|w| w.to_ne_bytes())
                .collect();
            self.queue.write_buffer(&params, 0, &words);
            let mut encoder = self.device.create_command_encoder(&Default::default());
            {
                let state = self.primary.as_ref().unwrap();
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &zero_group, &[]);
                pass.set_bind_group(1, &*state.geometry.as_ref().unwrap().arrays, &[]);
                pass.set_bind_group(2, &own_group, &[]);
                pass.set_bind_group(3, self.lighting.bind(), &[]);
                pass.dispatch_workgroups(grid.div_ceil(8), grid.div_ceil(4), 1);
            }
            self.queue.submit([encoder.finish()]);
            self.test_drain("footprint verification");
        }
        let staging = make(
            "footprint readback",
            (count * 32) as u64,
            wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        );
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&sums, 0, &staging, 0, (count * 32) as u64);
        let done = Arc::new(OnceLock::new());
        let callback = done.clone();
        encoder.map_buffer_on_submit(&staging, wgpu::MapMode::Read, .., move |v| {
            callback.set(v).unwrap();
        });
        self.queue.submit([encoder.finish()]);
        self.test_drain("footprint readback");
        done.get().unwrap().as_ref().unwrap();
        staging.get_mapped_range(..).unwrap().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(points: impl Iterator<Item = [u32; 2]>) -> Vec<[u32; 2]> {
        let mut v: Vec<_> = points.collect();
        v.sort_unstable();
        v
    }

    #[test]
    fn sequence_starts_at_the_centre_then_the_quarter_pair_and_corner() {
        for s in 0..4 {
            assert_eq!(sample_point(0, s), [256, 256], "symmetry {s}");
        }
        // The design's order on its 16-point lattice (row i at 16 i, column 7 i mod 16):
        // rows 8, 12, 4, 0, then the bit-reversed rows.
        let rows = [8, 12, 4, 0, 2, 10, 6, 14, 1, 9, 5, 13, 3, 11, 7, 15];
        for (k, row) in rows.into_iter().enumerate() {
            assert_eq!(
                lattice_node(k as u32, 0),
                [16 * row, 16 * (7 * row % 16)],
                "index {k}"
            );
        }
    }

    #[test]
    fn no_sample_lies_on_a_pixel_edge() {
        for s in 0..4 {
            for k in 0..256 {
                assert!(
                    sample_point(k, s).iter().all(|&v| v > 0 && v < 512),
                    "k={k}"
                );
            }
        }
    }

    #[test]
    fn every_power_of_two_prefix_is_the_moved_lattice_with_the_exact_centre() {
        let moved = |p: [u32; 2]| p.map(|v| 2 * ((v + 8) & 0xff) + 1);
        for m in 2..=8u32 {
            let n = 1u32 << m;
            let step = 256 / n;
            let lattice = set((0..n)
                .map(|i| [i * step, (GENERATOR * i % n) * step])
                .filter(|&p| p != [128, 128])
                .map(moved)
                .chain([[256, 256]]));
            for s in [0, 1] {
                assert_eq!(
                    set((0..n).map(|k| sample_point(k, s))),
                    lattice,
                    "N={n} s={s}"
                );
            }
            // Transposition preserves the lattice only while 71 * 71 = 1 (mod N).
            let transposed = set((0..n).map(|k| sample_point(k, 2))) == lattice;
            assert_eq!(transposed, GENERATOR * GENERATOR % n == 1, "N={n}");
        }
        assert_eq!(set((0..256).map(|k| sample_point(k, 3))).len(), 256);
    }

    #[test]
    fn temporal_compositions_apply_every_seam_once() {
        let prelude = include_str!("../shaders/scene.wgsl");
        let kernels = shader_sources(prelude).unwrap();
        let heads = shading_heads(prelude).unwrap();
        let kernel = kernel_source(&kernels[1]).unwrap();
        assert!(kernel.contains("fresh_direction(j+film_rotation"));
        let head = shading_head(&heads[1]).unwrap();
        let standard = compose_shading(&head, crate::STANDARD_PROGRAM, false);
        // Every store goes through film_store, which holds the module's only HDR writes:
        // tiles without history, and accumulated pixels.
        assert_eq!(
            standard.source.matches("textureStore(primary_hdr").count(),
            2
        );
        assert_eq!(
            &standard.source[standard.program.clone()],
            crate::STANDARD_PROGRAM
        );
        for source in [kernel, standard.source] {
            let module = naga::front::wgsl::parse_str(&source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
        // The default modules never see a seam.
        assert!(!kernels[1].contains("film_"));
        assert!(!heads[1].contains("film_"));
    }
}
