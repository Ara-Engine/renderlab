//! S4a's wgpu inputs (`RENDERLAB_RL_HIT_REUSE`, shaders/reflection_hit_reuse.wgsl,
//! docs/design-hit-reuse.md): TR1's frame record (66), both history sides (67/68 side 0, 73/74
//! side 1) and the side its resolve reads this frame (75). Every group that names them is
//! parity-free: the cached shading and trace groups bind both sides, and the side is a uniform
//! written each frame, so no group goes stale when TR1 alternates. Before TR1 has histories,
//! stand-ins bind (a zero frame record: no hit reuses). A child of primary_ray_gpu.
use super::*;

const FRAME: u32 = 66;
const COLOUR: [u32; 2] = [67, 73];
const ANCHOR: [u32; 2] = [68, 74];
const SIDE: u32 = 75;

/// The prefix of TR1's frame record the reuse test reads (RlReuseFrame: three matrices and
/// seven vec4 words).
const FRAME_PREFIX_BYTES: u64 = 3 * 64 + 7 * 16;

/// The layout entries of every traced group that composes the reuse test; none unset.
pub(super) fn layout_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    if crate::program::hit_reuse().is_none() {
        return vec![];
    }
    use wgpu::TextureSampleType as S;
    vec![
        read(FRAME, FRAME_PREFIX_BYTES),
        texture(COLOUR[0], S::Float { filterable: false }),
        texture(ANCHOR[0], S::Uint),
        texture(COLOUR[1], S::Float { filterable: false }),
        texture(ANCHOR[1], S::Uint),
        uniform(SIDE, 16),
    ]
}

/// The side uniform and the stand-ins bound while TR1 has no histories.
pub(super) struct ReuseInputs {
    side: Buffer,
    frame: Buffer,
    colour: Owned<wgpu::TextureView>,
    anchor: Owned<wgpu::TextureView>,
}

impl Renderer {
    /// Creates the reuse inputs on first use and writes the side TR1's resolve reads this
    /// frame. Runs in `prepare_primary`, before `prepare_reconstruction` flips the parity: the
    /// side TR1 wrote last frame is the side it reads now.
    pub(super) fn prepare_hit_reuse(&mut self) {
        if crate::program::hit_reuse().is_none() {
            return;
        }
        if self
            .view
            .primary
            .as_ref()
            .is_some_and(|s| s.reuse.is_none())
        {
            let stand_in = |format: wgpu::TextureFormat, label: &str| {
                let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                self.ledger.own(
                    texture.create_view(&wgpu::TextureViewDescriptor::default()),
                    ResourceKind::TextureView,
                )
            };
            let inputs = ReuseInputs {
                side: Buffer::new(self, "hit reuse side", 16, wgpu::BufferUsages::UNIFORM),
                frame: Buffer::new(
                    self,
                    "hit reuse stand-in frame",
                    FRAME_PREFIX_BYTES,
                    wgpu::BufferUsages::STORAGE,
                ),
                colour: stand_in(
                    wgpu::TextureFormat::Rgba32Float,
                    "hit reuse stand-in colour",
                ),
                anchor: stand_in(wgpu::TextureFormat::Rg32Uint, "hit reuse stand-in anchors"),
            };
            self.view.primary.as_mut().unwrap().reuse = Some(inputs);
        }
        let side = self
            .view
            .reconstruction_state
            .as_ref()
            .map_or(0, |r| r.reuse_sources().4 as u32);
        if let Some(inputs) = self.view.primary.as_ref().and_then(|s| s.reuse.as_ref()) {
            self.queue.write_buffer(
                &inputs.side.value,
                0,
                &[side, 0, 0, 0].map(u32::to_ne_bytes).concat(),
            );
        }
    }

    /// The reuse entries for a group (empty unset), and the identity of TR1's histories for
    /// the bindings key: a new or resized history rebuilds the groups once, never per parity.
    pub(super) fn hit_reuse_entries(
        &self,
    ) -> (Vec<(u32, wgpu::BindingResource<'_>)>, Option<wgpu::Texture>) {
        use wgpu::BindingResource as R;
        let Some(inputs) = self.view.primary.as_ref().and_then(|s| s.reuse.as_ref()) else {
            return (vec![], None);
        };
        let side = (SIDE, inputs.side.value.as_entire_binding());
        match self.view.reconstruction_state.as_ref() {
            Some(r) => {
                let (frame, colour, anchor, identity, _) = r.reuse_sources();
                (
                    vec![
                        (FRAME, frame.as_entire_binding()),
                        (COLOUR[0], R::TextureView(colour[0])),
                        (ANCHOR[0], R::TextureView(anchor[0])),
                        (COLOUR[1], R::TextureView(colour[1])),
                        (ANCHOR[1], R::TextureView(anchor[1])),
                        side,
                    ],
                    Some(identity.clone()),
                )
            }
            None => (
                vec![
                    (FRAME, inputs.frame.value.as_entire_binding()),
                    (COLOUR[0], R::TextureView(&inputs.colour)),
                    (ANCHOR[0], R::TextureView(&inputs.anchor)),
                    (COLOUR[1], R::TextureView(&inputs.colour)),
                    (ANCHOR[1], R::TextureView(&inputs.anchor)),
                    side,
                ],
                None,
            ),
        }
    }
}
