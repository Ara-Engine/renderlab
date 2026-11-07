//! Opt-in acceleration cache; never changes the raster package/page namespace.
use crate::{
    Error, Result,
    cache::{atomic_publish, hash, hex, immutable_entry, read_bounded},
    cluster_bvh::{self, Settings},
    ensure, virtual_cache,
};
use renderlab_asset::{
    Asset, Material,
    cluster_bvh::{self as format, Bvh},
    virtual_mesh::{self, Package, VirtualLimits},
};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::Instant,
};

pub struct GeometrySource {
    pub package: Package,
    pub digest: [u8; 32],
    pub rla_version: u32,
    pub rvg_version: u32,
}
/// Streams the original digest before parsing, skips immutable texel payloads,
/// and decodes only geometry metadata with placeholder materials. This value is
/// deliberately *not* a renderable asset. Geometry retains original material IDs.
pub fn load_geometry(
    cache: &Path,
    manifest: &Path,
    limits: VirtualLimits,
) -> Result<GeometrySource> {
    let m = read_bounded(manifest, 80)?;
    ensure(
        m.len() == 80 && &m[..4] == b"RVM\0",
        "invalid virtual manifest",
    )?;
    let version = u32::from_le_bytes(m[4..8].try_into().unwrap());
    ensure(
        matches!(version, 1 | 2),
        "unsupported source virtual schema",
    )?;
    let digest: [u8; 32] = m[40..72].try_into().unwrap();
    let length = u64::from_le_bytes(m[72..80].try_into().unwrap());
    ensure(
        length <= limits.prepared.max_file_bytes as u64,
        "source virtual file cap",
    )?;
    let mut f = File::open(cache.join(format!("virtual-v{version}/packages/{}", hex(&digest))))?;
    ensure(f.metadata()?.len() == length, "source virtual extent")?;
    let mut h = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        h.update(&buffer[..n]);
    }
    ensure(
        <[u8; 32]>::from(h.finalize()) == digest,
        "source virtual digest",
    )?;
    f.seek(SeekFrom::Start(0))?;
    let mut header = [0u8; 24];
    f.read_exact(&mut header)?;
    ensure(
        &header[..4] == b"RLV\0" && header[4..8] == version.to_le_bytes(),
        "source virtual header",
    )?;
    let resources = u32::from_le_bytes(header[20..24].try_into().unwrap()) as usize;
    let mut rla = [0u8; 24];
    f.read_exact(&mut rla)?;
    let rla_version = u32::from_le_bytes(rla[4..8].try_into().unwrap());
    let materials = u32::from_le_bytes(rla[16..20].try_into().unwrap()) as usize;
    ensure(
        resources >= 24
            && &rla[..4] == b"RLA\0"
            && matches!(rla_version, 1..=3)
            && rla[8..16] == [0; 8]
            && materials <= limits.prepared.max_materials
            && materials
                .checked_mul(36)
                .is_some_and(|n| n <= resources.saturating_sub(24))
            && materials
                .checked_mul(std::mem::size_of::<Material>())
                .is_some_and(|n| {
                    n <= limits
                        .max_metadata_bytes
                        .min(limits.prepared.max_decoded_bytes)
                }),
        "source virtual resource header",
    )?;
    let offset = 24u64 + resources as u64;
    let tail = length
        .checked_sub(offset)
        .ok_or_else(|| Error("source virtual resource extent".into()))?;
    ensure(
        tail <= limits.max_metadata_bytes as u64,
        "source virtual geometry metadata cap",
    )?;
    let placeholders = renderlab_asset::encode(
        &Asset {
            materials: vec![Material::default(); materials],
            ..Asset::default()
        },
        limits.prepared,
    )?;
    header[20..24].copy_from_slice(
        &u32::try_from(placeholders.len())
            .map_err(|_| Error("material placeholder extent".into()))?
            .to_le_bytes(),
    );
    let mut bytes = Vec::with_capacity(24 + placeholders.len() + tail as usize);
    bytes.extend(header);
    bytes.extend(placeholders);
    f.seek(SeekFrom::Start(offset))?;
    f.take(tail + 1).read_to_end(&mut bytes)?;
    ensure(
        bytes.len()
            == 24 + u32::from_le_bytes(header[20..24].try_into().unwrap()) as usize + tail as usize,
        "source geometry changed during read",
    )?;
    let package = virtual_mesh::decode(&bytes, limits)?;
    Ok(GeometrySource {
        package,
        digest,
        rla_version,
        rvg_version: version,
    })
}
fn namespace(cache: &Path, source: &GeometrySource) -> PathBuf {
    cache.join(format!(
        "cluster-bvh-v{}/rla{}-rvg{}",
        format::VERSION,
        source.rla_version,
        source.rvg_version
    ))
}
fn key(source: &GeometrySource, settings: &Settings) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(cluster_bvh::ALGORITHM);
    h.update(source.digest);
    h.update(source.rla_version.to_le_bytes());
    h.update(source.rvg_version.to_le_bytes());
    h.update(format::VERSION.to_le_bytes());
    for e in &settings.errors {
        h.update(e.to_le_bytes());
    }
    h.finalize().into()
}
pub struct Cooked {
    pub bvh: Bvh,
    pub digest: [u8; 32],
    pub key: [u8; 32],
    pub cache_hit: bool,
    pub build_us: u128,
    pub total_us: u128,
}
/// Inputs are an existing cooked DAG and its pages. No import or raster/shader
/// edits; publication occurs only after complete encode, decode and hash checks.
pub fn cook(
    source: &GeometrySource,
    source_cache: &Path,
    cache: &Path,
    current: &Path,
    settings: &Settings,
    limits: VirtualLimits,
) -> Result<Cooked> {
    let start = Instant::now();
    settings.validate()?;
    let key = key(source, settings);
    let directory = namespace(cache, source);
    let key_path = directory.join("keys").join(hex(&key));
    let cached = (|| -> Result<_> {
        let m = read_bounded(&key_path, 112)?;
        ensure(
            m.len() == 112
                && &m[..4] == b"RBM\0"
                && m[4..8] == format::VERSION.to_le_bytes()
                && m[8..40] == key
                && m[72..104] == source.digest
                && m[104..108] == source.rla_version.to_le_bytes()
                && m[108..112] == source.rvg_version.to_le_bytes(),
            "BVH manifest identity",
        )?;
        let digest: [u8; 32] = m[40..72].try_into().unwrap();
        let bytes = read_bounded(
            &directory.join("packages").join(hex(&digest)),
            settings.limits.max_bytes,
        )?;
        ensure(hash(&bytes) == digest, "BVH payload digest")?;
        let bvh = format::decode(&bytes, settings.limits)?;
        ensure(
            bvh.levels.len() == source.package.meshes.len() * settings.errors.len()
                && bvh.levels.iter().enumerate().all(|(i, l)| {
                    l.mesh as usize == i / settings.errors.len()
                        && l.requested_error == settings.errors[i % settings.errors.len()]
                }),
            "BVH cached level settings",
        )?;
        Ok((m, digest, bvh))
    })();
    if let Ok((m, digest, bvh)) = cached {
        atomic_publish(current, |f| {
            f.write_all(&m)?;
            Ok(())
        })?;
        return Ok(Cooked {
            bvh,
            digest,
            key,
            cache_hit: true,
            build_us: 0,
            total_us: start.elapsed().as_micros(),
        });
    }
    let build = Instant::now();
    let bvh = cluster_bvh::prepare(&source.package, settings, limits, |page| {
        virtual_cache::load_page(source_cache, &source.package.pages[page])
    })?;
    let build_us = build.elapsed().as_micros();
    let bytes = format::encode(&bvh, settings.limits)?;
    let digest = hash(&bytes);
    // Round-trip once on a retained cook; release that second copy before publishing.
    ensure(
        format::decode(&bytes, settings.limits)? == bvh,
        "BVH round trip",
    )?;
    immutable_entry(&directory.join("packages").join(hex(&digest)), &bytes)?;
    let mut manifest = b"RBM\0".to_vec();
    manifest.extend(format::VERSION.to_le_bytes());
    manifest.extend(key);
    manifest.extend(digest);
    manifest.extend(source.digest);
    manifest.extend(source.rla_version.to_le_bytes());
    manifest.extend(source.rvg_version.to_le_bytes());
    immutable_entry(&key_path, &manifest)?;
    atomic_publish(current, |f| {
        f.write_all(&manifest)?;
        Ok(())
    })?;
    Ok(Cooked {
        bvh,
        digest,
        key,
        cache_hit: false,
        build_us,
        total_us: start.elapsed().as_micros(),
    })
}
