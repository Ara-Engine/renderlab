//! Tests: the storage buffers each pipeline layout makes visible to the compute stage, from the
//! entries of the bind group layouts it combines, so a test can compare a path's real need with
//! the limits it requests (`visibility_limits`). wgpu counts every plain storage-buffer entry per
//! stage it is visible to; binding arrays count against their own limit.
//!
//! Layouts are recorded where they are made (`record_group`), pipeline layouts where they are
//! combined (`record_pipeline`); a group nobody recorded counts nothing.
use std::collections::HashMap;
use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
use std::sync::Mutex;

struct Needs {
    /// Per bind group layout (its identity's hash), its compute-visible storage buffers.
    groups: HashMap<u64, u32>,
    /// Per pipeline layout label, the sum over its groups.
    pipelines: HashMap<String, u32>,
}
static NEEDS: Mutex<Option<Needs>> = Mutex::new(None);

fn key(layout: &wgpu::BindGroupLayout) -> u64 {
    BuildHasherDefault::<DefaultHasher>::default().hash_one(layout)
}

fn with<R>(f: impl FnOnce(&mut Needs) -> R) -> R {
    let mut needs = NEEDS.lock().unwrap();
    f(needs.get_or_insert_with(|| Needs {
        groups: HashMap::new(),
        pipelines: HashMap::new(),
    }))
}

/// Records `layout`, made from `entries`.
pub(crate) fn record_group(layout: &wgpu::BindGroupLayout, entries: &[wgpu::BindGroupLayoutEntry]) {
    let count = entries
        .iter()
        .filter(|e| {
            e.count.is_none()
                && e.visibility.contains(wgpu::ShaderStages::COMPUTE)
                && matches!(
                    e.ty,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { .. },
                        ..
                    }
                )
        })
        .count() as u32;
    with(|n| n.groups.insert(key(layout), count));
}

/// Records the pipeline layout `label` combining `groups`.
pub(crate) fn record_pipeline(label: &str, groups: &[Option<&wgpu::BindGroupLayout>]) {
    with(|n| {
        let sum = groups
            .iter()
            .flatten()
            .map(|g| n.groups.get(&key(g)).copied().unwrap_or(0))
            .sum();
        n.pipelines.insert(label.to_owned(), sum);
    });
}

/// The pipeline layout recorded so far with the most compute-visible storage buffers.
pub(crate) fn largest() -> Option<(String, u32)> {
    with(|n| {
        n.pipelines
            .iter()
            .max_by_key(|(label, count)| (**count, std::cmp::Reverse((*label).clone())))
            .map(|(label, count)| (label.clone(), *count))
    })
}
