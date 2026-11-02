//! Separately versioned virtual cache. Publication reuses M3's complete-file rename primitive.
use crate::{
    Error, Result,
    cache::{atomic_publish, hash, hex, immutable_entry, read_bounded},
    ensure,
    virtual_cook::{self, Settings, Statistics},
};
use renderlab_asset::virtual_mesh::{self, PAGE_BYTES, Package, PackageCounts, VirtualLimits};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Clone)]
struct Manifest {
    key: [u8; 32],
    package: [u8; 32],
    bytes: u64,
}
impl Manifest {
    fn encode(&self) -> Vec<u8> {
        let mut b = b"RVM\0".to_vec();
        b.extend(1u32.to_le_bytes());
        b.extend(self.key);
        b.extend(self.package);
        b.extend(self.bytes.to_le_bytes());
        b
    }
    fn decode(b: &[u8], limits: VirtualLimits) -> Result<Self> {
        ensure(
            b.len() == 80 && &b[..4] == b"RVM\0" && b[4..8] == 1u32.to_le_bytes(),
            "invalid virtual manifest",
        )?;
        let bytes = u64::from_le_bytes(b[72..80].try_into().unwrap());
        ensure(
            bytes <= limits.prepared.max_file_bytes as u64,
            "virtual manifest byte limit",
        )?;
        Ok(Self {
            key: b[8..40].try_into().unwrap(),
            package: b[40..72].try_into().unwrap(),
            bytes,
        })
    }
}
fn path(cache: &Path, kind: &str, digest: &[u8; 32]) -> PathBuf {
    cache.join("virtual-v1").join(kind).join(hex(digest))
}
fn key(source: &[u8], settings: Settings, limits: VirtualLimits) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(virtual_cook::ALGORITHM);
    h.update(settings.bytes());
    h.update(virtual_mesh::VERSION.to_le_bytes());
    let p = limits.prepared;
    for n in [
        limits.max_groups,
        limits.max_clusters,
        limits.max_pages,
        limits.max_metadata_bytes,
        limits.max_geometry_bytes,
        p.max_file_bytes,
        p.max_decoded_bytes,
        p.max_meshes,
        p.max_primitives,
        p.max_placements,
        p.max_materials,
        p.max_textures,
        p.max_vertices,
        p.max_indices,
        p.max_dimension as usize,
    ] {
        h.update((n as u64).to_le_bytes());
    }
    h.update(source);
    h.finalize().into()
}
pub struct Loaded {
    pub package: Package,
    pub key: String,
    pub package_sha256: String,
    pub metadata_file_bytes: usize,
    pub counts: PackageCounts,
}
#[derive(Debug)]
pub struct Report {
    pub cache_hit: bool,
    pub miss_reason: Option<String>,
    pub source_bytes: usize,
    pub prepare_us: u128,
    pub verify_us: u128,
    pub total_us: u128,
    pub statistics: Option<Statistics>,
}
pub struct CookResult {
    pub loaded: Loaded,
    pub report: Report,
}

/// Verify metadata plus each complete immutable page one at a time, retaining no source geometry.
fn load(cache: &Path, m: &Manifest, limits: VirtualLimits) -> Result<Loaded> {
    let bytes = read_bounded(&path(cache, "packages", &m.package), m.bytes as usize)?;
    ensure(
        bytes.len() as u64 == m.bytes && hash(&bytes) == m.package,
        "virtual metadata digest/length mismatch",
    )?;
    let package = virtual_mesh::decode(&bytes, limits)?;
    for p in &package.pages {
        let bytes = read_bounded(&path(cache, "pages", &p.sha256), PAGE_BYTES)?;
        virtual_mesh::decode_page(p, &bytes, hash(&bytes))?;
    }
    let counts = package.validate(limits)?;
    Ok(Loaded {
        package,
        key: hex(&m.key),
        package_sha256: hex(&m.package),
        metadata_file_bytes: bytes.len(),
        counts,
    })
}
pub fn load_manifest(cache: &Path, current: &Path, limits: VirtualLimits) -> Result<Loaded> {
    load(
        cache,
        &Manifest::decode(&read_bounded(current, 80)?, limits)?,
        limits,
    )
}
pub fn load_page(cache: &Path, p: &virtual_mesh::Page) -> Result<virtual_mesh::PageGeometry> {
    let b = read_bounded(&path(cache, "pages", &p.sha256), PAGE_BYTES)?;
    Ok(virtual_mesh::decode_page(p, &b, hash(&b))?)
}
pub fn page_path(cache: &Path, p: &virtual_mesh::Page) -> PathBuf {
    path(cache, "pages", &p.sha256)
}

/// Input is a separately preserved M3 full-detail reference; runtime consumes only package/pages.
pub fn cook_file(
    source: &Path,
    cache: &Path,
    current: &Path,
    settings: Settings,
    limits: VirtualLimits,
) -> Result<CookResult> {
    let start = Instant::now();
    settings.validate()?;
    let source = read_bounded(source, limits.prepared.max_file_bytes)?;
    let key = key(&source, settings, limits);
    let key_path = path(cache, "keys", &key);
    let cached = (|| {
        let m = Manifest::decode(&read_bounded(&key_path, 80)?, limits)?;
        ensure(m.key == key, "virtual key identity mismatch")?;
        let loaded = load(cache, &m, limits)?;
        Ok((m, loaded))
    })();
    let miss_reason = match cached {
        Ok((m, loaded)) => {
            atomic_publish(current, |f| {
                f.write_all(&m.encode())?;
                Ok(())
            })?;
            return Ok(CookResult {
                loaded,
                report: Report {
                    cache_hit: true,
                    miss_reason: None,
                    source_bytes: source.len(),
                    prepare_us: 0,
                    verify_us: start.elapsed().as_micros(),
                    total_us: start.elapsed().as_micros(),
                    statistics: None,
                },
            });
        }
        Err(Error(reason)) => reason,
    };
    let before = Instant::now();
    let reference = renderlab_asset::decode(&source, limits.prepared)?;
    let cooked = virtual_cook::prepare(&reference, settings, limits)?;
    let prepare_us = before.elapsed().as_micros();
    let bytes = virtual_mesh::encode(&cooked.package, limits)?;
    let m = Manifest {
        key,
        package: hash(&bytes),
        bytes: bytes.len() as u64,
    };
    for (p, b) in cooked.package.pages.iter().zip(&cooked.pages) {
        immutable_entry(&path(cache, "pages", &p.sha256), b)?;
    }
    immutable_entry(&path(cache, "packages", &m.package), &bytes)?;
    let before = Instant::now();
    let loaded = load(cache, &m, limits)?;
    let verify_us = before.elapsed().as_micros();
    immutable_entry(&key_path, &m.encode())?;
    atomic_publish(current, |f| {
        f.write_all(&m.encode())?;
        Ok(())
    })?;
    Ok(CookResult {
        loaded,
        report: Report {
            cache_hit: false,
            miss_reason: Some(miss_reason),
            source_bytes: source.len(),
            prepare_us,
            verify_us,
            total_us: start.elapsed().as_micros(),
            statistics: Some(cooked.statistics),
        },
    })
}
