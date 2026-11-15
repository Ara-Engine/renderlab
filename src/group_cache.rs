//! Bind groups that encoding used to create every frame, kept while what they bind is unchanged.
//!
//! `GroupCache::get` takes the descriptor a call site would pass to `create_bind_group` and
//! returns the group made from an identical descriptor (the same layout object and, binding by
//! binding, the same wgpu objects at the same offsets and sizes), or makes it. wgpu objects compare
//! by identity, so a resource replaced since (a grown buffer, a new view) is a different key and
//! gets a fresh group. Groups unused for `KEEP_FRAMES` frames are dropped (`end_frame`), so a
//! cached group holds its resources at most that long after their last use. The same bindings
//! give the same group contents, so nothing a shader reads changes.
//! `RENDERLAB_GROUP_CACHE=0` creates every group again each frame.
use std::collections::HashMap;
use std::num::NonZeroU64;
use std::sync::Mutex;

const KEEP_FRAMES: u64 = 8;

#[derive(Clone, PartialEq, Eq, Hash)]
enum Part {
    Layout(wgpu::BindGroupLayout),
    Binding(u32),
    Buffer(wgpu::Buffer, u64, Option<NonZeroU64>),
    View(wgpu::TextureView),
    Sampler(wgpu::Sampler),
    Tlas(wgpu::Tlas),
}

#[derive(Default)]
pub(crate) struct GroupCache {
    groups: Mutex<HashMap<Vec<Part>, (wgpu::BindGroup, u64)>>,
    frame: std::sync::atomic::AtomicU64,
}

fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_GROUP_CACHE").map_or(true, |v| v != "0"))
}

/// The descriptor's identity, or None for a binding kind it does not key (made every time).
fn key(desc: &wgpu::BindGroupDescriptor<'_>) -> Option<Vec<Part>> {
    use wgpu::BindingResource as R;
    let buffer = |b: &wgpu::BufferBinding<'_>| Part::Buffer(b.buffer.clone(), b.offset, b.size);
    let mut parts = vec![Part::Layout(desc.layout.clone())];
    for entry in desc.entries {
        parts.push(Part::Binding(entry.binding));
        match &entry.resource {
            R::Buffer(b) => parts.push(buffer(b)),
            R::BufferArray(bs) => parts.extend(bs.iter().map(buffer)),
            R::Sampler(s) => parts.push(Part::Sampler((*s).clone())),
            R::SamplerArray(ss) => parts.extend(ss.iter().map(|s| Part::Sampler((*s).clone()))),
            R::TextureView(v) => parts.push(Part::View((*v).clone())),
            R::TextureViewArray(vs) => parts.extend(vs.iter().map(|v| Part::View((*v).clone()))),
            R::AccelerationStructure(t) => parts.push(Part::Tlas((*t).clone())),
            _ => return None,
        }
    }
    Some(parts)
}

impl GroupCache {
    /// The group `desc` describes: the cached one when an identical descriptor made it.
    pub(crate) fn get(&self, device: &wgpu::Device, desc: &wgpu::BindGroupDescriptor<'_>) -> wgpu::BindGroup {
        let Some(key) = key(desc).filter(|_| on()) else {
            return device.create_bind_group(desc);
        };
        let frame = self.frame.load(std::sync::atomic::Ordering::Relaxed);
        let mut groups = self.groups.lock().unwrap();
        if let Some((group, used)) = groups.get_mut(&key) {
            *used = frame;
            return group.clone();
        }
        let group = device.create_bind_group(desc);
        groups.insert(key, (group.clone(), frame));
        group
    }
    /// After a frame's encoding: drops the groups no frame used for KEEP_FRAMES frames.
    pub(crate) fn end_frame(&self) {
        let frame = self.frame.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        self.groups
            .lock()
            .unwrap()
            .retain(|_, (_, used)| *used + KEEP_FRAMES > frame);
    }
}
