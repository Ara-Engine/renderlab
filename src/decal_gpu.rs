//! DC1's single extra lighting binding. Publication happens only on admitted frames.
use super::*;
use crate::{Decal, DecalSettings, DecalStats};

pub(super) struct DecalBuffer {
    pub buffer: Owned<wgpu::Buffer>,
    _allocation: Allocation,
}
pub(super) struct DecalState {
    pub active: DecalBuffer,
    retired: Vec<(u64, DecalBuffer)>,
    pending: Vec<u32>,
    published: Vec<u32>,
    pub generation: u64,
    pub hash: [u8; 32],
    stats: DecalStats,
}
impl DecalState {
    pub fn new(device: &wgpu::Device, ledger: &Ledger) -> Self {
        let words = vec![0; crate::decal::HEADER];
        Self {
            active: buffer(device, ledger, &words),
            retired: vec![],
            pending: words.clone(),
            published: words,
            generation: 0,
            hash: [0; 32],
            stats: DecalStats {
                bytes: 64,
                ..Default::default()
            },
        }
    }
    pub fn publish(
        &mut self,
        device: &wgpu::Device,
        ledger: &Ledger,
        submission: u64,
        uploads: &mut UploadBytes,
    ) {
        if self.pending == self.published {
            return;
        }
        let next = buffer(device, ledger, &self.pending);
        let old = std::mem::replace(&mut self.active, next);
        self.retired.push((submission, old));
        self.published.clone_from(&self.pending);
        self.generation += 1;
        uploads.materials += self.stats.bytes;
    }
    pub fn collect(&mut self, completed: u64) {
        self.retired.retain(|(last, _)| *last > completed);
    }
}
fn buffer(device: &wgpu::Device, ledger: &Ledger, words: &[u32]) -> DecalBuffer {
    let size = words.len() as u64 * 4;
    let buffer = ledger.own(
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("DC1 decal grid, records and linear mip texels"),
            size,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: true,
        }),
        ResourceKind::Buffer,
    );
    {
        let mut bytes = buffer.slice(..).get_mapped_range_mut().unwrap();
        for (i, word) in words.iter().enumerate() {
            bytes
                .slice(i * 4..i * 4 + 4)
                .copy_from_slice(&word.to_ne_bytes());
        }
    }
    buffer.unmap();
    DecalBuffer {
        buffer,
        _allocation: ledger.allocate(AllocationKind::Material, size),
    }
}

pub(super) fn layout_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 21,
        visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(64),
        },
        count: None,
    }
}

impl Renderer {
    /// Replace the scene's decals atomically. Empty disables decals. They are otherwise
    /// active on every lit path, with no opt-in or RT requirement. Textures are retained
    /// as packed mip texels, so the caller may drop its handles after this returns.
    pub fn set_decals(&mut self, decals: &[Decal]) -> Result<(), Error> {
        self.set_decals_with_settings(decals, DecalSettings::default())
    }
    pub fn set_decals_with_settings(
        &mut self,
        decals: &[Decal],
        settings: DecalSettings,
    ) -> Result<(), Error> {
        self.check_fault()?;
        let limits = self.device.limits();
        let max = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        let (words, stats) = crate::decal::pack(decals, settings, max)?;
        self.lighting.decals.pending = words;
        let mut hash = Sha256::new();
        for word in &self.lighting.decals.pending {
            hash.update(word.to_le_bytes());
        }
        self.lighting.decals.hash = hash.finalize().into();
        self.lighting.decals.stats = stats;
        Ok(())
    }
    pub fn decal_stats(&self) -> DecalStats {
        self.lighting.decals.stats
    }
}
