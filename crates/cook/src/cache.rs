use crate::{CookLimits, Error, Result, Settings, ensure, prepare};
use renderlab_asset::{Asset, Counts, SCHEMA_VERSION};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

const ALGORITHM: &[u8] =
    b"renderlab-cook/static-glb/area-mips/1/gltf-1.4.1/image-0.25.10/sha2-0.10.9";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
pub fn hash_hex(bytes: &[u8]) -> String {
    hex(&hash(bytes))
}
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn key(source: &[u8], settings: Settings, limits: CookLimits) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(ALGORITHM);
    if settings.normal_maps {
        h.update(b"normal-maps-v2/mikktspace-0.3.0/gltf-negated-mikk-w/ignore-tangent-without-normal/frame2e-4/area1e-6/linear+Y/normalized-area-mips-v1/scale-xy");
    }
    h.update(SCHEMA_VERSION.to_le_bytes());
    h.update(settings.bytes());
    let p = limits.prepared;
    for n in [
        limits.source_bytes,
        limits.json_bytes,
        limits.nodes,
        limits.hierarchy_depth,
        limits.images,
        limits.image_decoded_bytes,
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
    let extra = |l: CookLimits| {
        [
            l.accessors,
            l.buffer_views,
            l.scenes,
            l.samplers,
            l.json_values,
            l.manifest_bytes,
        ]
    };
    if extra(limits) != extra(CookLimits::default()) {
        h.update(b"source-policy-2");
        for n in extra(limits) {
            h.update((n as u64).to_le_bytes());
        }
    }
    h.update(source);
    h.finalize().into()
}

/// Bounded actual reads, even if a file grows after its metadata was queried.
pub(crate) fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let file = File::open(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
    let size = file.metadata()?.len();
    ensure(
        size <= limit as u64,
        format!("{} exceeds byte limit {limit}", path.display()),
    )?;
    let size = usize::try_from(size)
        .map_err(|_| Error("file length exceeds host address space".into()))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|e| Error(format!("file allocation: {e}")))?;
    file.take((limit as u64).saturating_add(1))
        .read_to_end(&mut bytes)?;
    ensure(
        bytes.len() <= limit,
        format!("{} grew beyond byte limit", path.display()),
    )?;
    Ok(bytes)
}

pub(crate) fn atomic_publish(
    path: &Path,
    write: impl FnOnce(&mut File) -> Result<()>,
) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        ".renderlab-{}-{}.tmp",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let result = (|| {
        write(&mut file)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub(crate) fn immutable_entry(path: &Path, bytes: &[u8]) -> Result<bool> {
    if read_bounded(path, bytes.len()).is_ok_and(|existing| existing == bytes) {
        return Ok(true);
    }
    // All writers for this hash publish identical complete bytes. Rename also repairs corruption.
    atomic_publish(path, |file| {
        file.write_all(bytes)?;
        Ok(())
    })?;
    Ok(false)
}

#[derive(Clone)]
struct Entry {
    hash: [u8; 32],
    bytes: u64,
}
#[derive(Clone)]
struct Manifest {
    version: u32,
    key: [u8; 32],
    payload: Entry,
    geometry: Vec<Entry>,
}
impl Manifest {
    fn encode(&self) -> Vec<u8> {
        let mut bytes = b"RLM\0".to_vec();
        bytes.extend(self.version.to_le_bytes());
        bytes.extend(self.key);
        bytes.extend(self.payload.hash);
        bytes.extend(self.payload.bytes.to_le_bytes());
        bytes.extend((self.geometry.len() as u32).to_le_bytes());
        for entry in &self.geometry {
            bytes.extend(entry.hash);
            bytes.extend(entry.bytes.to_le_bytes());
        }
        bytes
    }
    fn decode(bytes: &[u8], limits: CookLimits) -> Result<Self> {
        ensure(
            bytes.len() >= 84 && bytes.len() <= limits.manifest_bytes,
            "truncated/oversized cache manifest",
        )?;
        ensure(
            &bytes[..4] == b"RLM\0"
                && matches!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 1 | 2),
            "unknown cache manifest version",
        )?;
        let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
        ensure(
            count <= limits.prepared.max_meshes
                && count.checked_mul(40).and_then(|n| n.checked_add(84)) == Some(bytes.len()),
            "invalid cache geometry count or manifest length",
        )?;
        let parse = |data: &[u8]| -> Result<Entry> {
            let bytes = u64::from_le_bytes(data[32..40].try_into().unwrap());
            ensure(
                bytes <= limits.prepared.max_file_bytes as u64,
                "cache entry byte limit",
            )?;
            Ok(Entry {
                hash: data[..32].try_into().unwrap(),
                bytes,
            })
        };
        let geometry: Vec<_> = bytes[84..]
            .chunks_exact(40)
            .map(parse)
            .collect::<Result<_>>()?;
        let mut total_geometry = 0u64;
        for entry in &geometry {
            total_geometry = total_geometry
                .checked_add(entry.bytes)
                .ok_or_else(|| Error("cache geometry byte overflow".into()))?;
            ensure(
                total_geometry <= limits.prepared.max_file_bytes as u64,
                "cache aggregate geometry byte limit",
            )?;
        }
        Ok(Self {
            version: u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            key: bytes[8..40].try_into().unwrap(),
            payload: parse(&bytes[40..80])?,
            geometry,
        })
    }
}
fn entry_path(cache: &Path, entry: &Entry, kind: &str) -> PathBuf {
    cache.join(kind).join(hex(&entry.hash))
}
fn verified_entry(cache: &Path, entry: &Entry, kind: &str) -> Result<Vec<u8>> {
    let path = entry_path(cache, entry, kind);
    let bytes = read_bounded(&path, entry.bytes as usize)?;
    ensure(
        bytes.len() as u64 == entry.bytes && hash(&bytes) == entry.hash,
        format!(
            "cache {kind} content hash/length mismatch: {}",
            path.display()
        ),
    )?;
    Ok(bytes)
}

pub struct LoadedAsset {
    pub asset: Asset,
    pub key: String,
    pub payload_hash: String,
    pub geometry_hashes: Vec<String>,
    pub prepared_bytes: usize,
    /// Sidecar storage duplicates geometry in the self-contained prepared payload.
    pub geometry_sidecar_bytes: usize,
}

fn load(
    cache: &Path,
    manifest: &Manifest,
    limits: CookLimits,
) -> Result<(LoadedAsset, u128, u128)> {
    let lookup = Instant::now();
    let bytes = verified_entry(cache, &manifest.payload, "assets")?;
    let mut geometry_sidecar_bytes = 0;
    for entry in &manifest.geometry {
        // Verify and drop one at a time, avoiding a second whole geometry allocation.
        let geometry = verified_entry(cache, entry, "geometry")?;
        geometry_sidecar_bytes += geometry.len();
    }
    let lookup_us = lookup.elapsed().as_micros();
    let decode = Instant::now();
    if manifest.version == 1 {
        ensure(
            bytes.get(4..8) == Some(&1u32.to_le_bytes()),
            "extended payload in legacy manifest",
        )?;
    }
    let asset = renderlab_asset::decode(&bytes, limits.prepared)?;
    ensure(
        asset.meshes.len() == manifest.geometry.len(),
        "manifest mesh count mismatch",
    )?;
    for (mesh, entry) in asset.meshes.iter().zip(&manifest.geometry) {
        let geometry = renderlab_asset::encode_mesh(mesh, limits.prepared)?;
        ensure(
            hash(&geometry) == entry.hash && geometry.len() as u64 == entry.bytes,
            "manifest geometry does not match prepared payload",
        )?;
    }
    let decode_us = decode.elapsed().as_micros();
    Ok((
        LoadedAsset {
            asset,
            key: hex(&manifest.key),
            payload_hash: hex(&manifest.payload.hash),
            geometry_hashes: manifest.geometry.iter().map(|e| hex(&e.hash)).collect(),
            prepared_bytes: bytes.len(),
            geometry_sidecar_bytes,
        },
        lookup_us,
        decode_us,
    ))
}

pub fn load_manifest(
    cache: &Path,
    manifest_path: &Path,
    limits: CookLimits,
) -> Result<LoadedAsset> {
    let bytes = read_bounded(manifest_path, limits.manifest_bytes)?;
    let manifest = Manifest::decode(&bytes, limits)?;
    Ok(load(cache, &manifest, limits)?.0)
}

#[derive(Debug)]
pub struct CookReport {
    pub cache_hit: bool,
    /// Missing/corrupt cache diagnostics are retained even after successful repair.
    pub cache_miss_reason: Option<String>,
    pub source_bytes: usize,
    pub prepared_bytes: usize,
    pub geometry_sidecar_bytes: usize,
    pub reused_geometry_entries: usize,
    pub reused_geometry_bytes: usize,
    pub counts: Counts,
    pub source_read_us: u128,
    pub lookup_us: u128,
    pub prepare_us: u128,
    pub encode_us: u128,
    pub load_us: u128,
    pub publication_us: u128,
    pub total_us: u128,
}
pub struct CookResult {
    pub loaded: LoadedAsset,
    pub report: CookReport,
}

/// Entry files are complete before either the immutable key or current manifest is published.
pub fn cook_file(
    source_path: &Path,
    cache: &Path,
    current_manifest: &Path,
    settings: Settings,
    limits: CookLimits,
) -> Result<CookResult> {
    let total = Instant::now();
    let start = Instant::now();
    let source = read_bounded(source_path, limits.source_bytes)?;
    let source_read_us = start.elapsed().as_micros();
    let lookup = Instant::now();
    let identity = key(&source, settings, limits);
    let key_path = cache
        .join(if settings.normal_maps {
            "normal-v2/keys"
        } else {
            "keys"
        })
        .join(hex(&identity));
    let cached = (|| {
        let manifest = Manifest::decode(&read_bounded(&key_path, limits.manifest_bytes)?, limits)?;
        ensure(manifest.key == identity, "cache key identity mismatch")?;
        let (loaded, _, load_us) = load(cache, &manifest, limits)?;
        Ok((manifest, loaded, load_us))
    })();
    let lookup_total = lookup.elapsed().as_micros();
    let miss_reason = match cached {
        Ok((manifest, loaded, load_us)) => {
            let publication = Instant::now();
            atomic_publish(current_manifest, |file| {
                file.write_all(&manifest.encode())?;
                Ok(())
            })?;
            let publication_us = publication.elapsed().as_micros();
            let counts = loaded.asset.validate(limits.prepared)?;
            let report = CookReport {
                cache_hit: true,
                cache_miss_reason: None,
                source_bytes: source.len(),
                prepared_bytes: loaded.prepared_bytes,
                geometry_sidecar_bytes: loaded.geometry_sidecar_bytes,
                reused_geometry_entries: manifest.geometry.len(),
                reused_geometry_bytes: loaded.geometry_sidecar_bytes,
                counts,
                source_read_us,
                lookup_us: lookup_total.saturating_sub(load_us),
                prepare_us: 0,
                encode_us: 0,
                load_us,
                publication_us,
                total_us: total.elapsed().as_micros(),
            };
            return Ok(CookResult { loaded, report });
        }
        Err(Error(reason)) => reason,
    };
    let prepare_start = Instant::now();
    let asset = prepare(&source, settings, limits)?;
    let prepare_us = prepare_start.elapsed().as_micros();
    let encode_start = Instant::now();
    let bytes = renderlab_asset::encode(&asset, limits.prepared)?;
    let geometry = asset
        .meshes
        .iter()
        .map(|m| renderlab_asset::encode_mesh(m, limits.prepared).map_err(Error::from))
        .collect::<Result<Vec<_>>>()?;
    let manifest = Manifest {
        version: if settings.normal_maps { 2 } else { 1 },
        key: identity,
        payload: Entry {
            hash: hash(&bytes),
            bytes: bytes.len() as u64,
        },
        geometry: geometry
            .iter()
            .map(|b| Entry {
                hash: hash(b),
                bytes: b.len() as u64,
            })
            .collect(),
    };
    let manifest = Manifest::decode(&manifest.encode(), limits)?;
    let encode_us = encode_start.elapsed().as_micros();
    let publication = Instant::now();
    immutable_entry(&entry_path(cache, &manifest.payload, "assets"), &bytes)?;
    let (mut reused_geometry_entries, mut reused_geometry_bytes) = (0, 0);
    for (entry, bytes) in manifest.geometry.iter().zip(&geometry) {
        if immutable_entry(&entry_path(cache, entry, "geometry"), bytes)? {
            reused_geometry_entries += 1;
            reused_geometry_bytes += bytes.len();
        }
    }
    let (loaded, verify_us, load_us) = load(cache, &manifest, limits)?;
    let manifest_bytes = manifest.encode();
    immutable_entry(&key_path, &manifest_bytes)?;
    atomic_publish(current_manifest, |file| {
        file.write_all(&manifest_bytes)?;
        Ok(())
    })?;
    let publication_us = publication
        .elapsed()
        .as_micros()
        .saturating_sub(verify_us + load_us);
    let report = CookReport {
        cache_hit: false,
        cache_miss_reason: Some(miss_reason),
        source_bytes: source.len(),
        prepared_bytes: bytes.len(),
        geometry_sidecar_bytes: geometry.iter().map(Vec::len).sum(),
        reused_geometry_entries,
        reused_geometry_bytes,
        counts: asset.validate(limits.prepared)?,
        source_read_us,
        lookup_us: lookup_total + verify_us,
        prepare_us,
        encode_us,
        load_us,
        publication_us,
        total_us: total.elapsed().as_micros(),
    };
    Ok(CookResult { loaded, report })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn published_default_lookup_key_stays_compatible() {
        let [(name, source), _] = crate::fixtures::sources().unwrap();
        assert_eq!(name, "instanced-boxes.glb");
        let settings = Settings::default();
        let limits = CookLimits::default();
        let default_key = key(&source, settings, limits);
        assert_eq!(
            hex(&default_key),
            "a90a6bab6dc8542b560933e68715879bf3dc4be7fca65289bf1397d6469e94ac"
        );
        assert_ne!(
            default_key,
            key(
                &source,
                settings,
                CookLimits {
                    samplers: limits.samplers + 1,
                    ..limits
                }
            )
        );
    }
    #[test]
    fn manifest_rejects_aggregate_sidecar_io_before_entry_reads() {
        let entry = Entry {
            hash: [0; 32],
            bytes: 30,
        };
        let manifest = Manifest {
            version: 1,
            key: [0; 32],
            payload: entry.clone(),
            geometry: vec![entry.clone(), entry],
        };
        let mut limits = CookLimits::default();
        limits.prepared.max_file_bytes = 40;
        assert!(
            Manifest::decode(&manifest.encode(), limits)
                .err()
                .unwrap()
                .0
                .contains("aggregate geometry")
        );
    }
    #[test]
    fn failed_atomic_write_preserves_previous_manifest() {
        let dir = std::env::temp_dir().join(format!("renderlab-atomic-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("current");
        atomic_publish(&path, |f| {
            f.write_all(b"accepted complete manifest")?;
            Ok(())
        })
        .unwrap();
        assert!(
            atomic_publish(&path, |f| {
                f.write_all(b"partial")?;
                Err(Error("injected publication write failure".into()))
            })
            .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), b"accepted complete manifest");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_file(path).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
