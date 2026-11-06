//! Re-prepare existing composed assets without reimporting or changing their geometry.
use crate::{Result, cache};
use renderlab_asset::{Asset, Counts, Limits};
use std::{io::Write, path::Path, time::Instant};

/// Charge the complete derived stream before building; preserve existing prepared tables.
pub fn prepare(asset: &mut Asset, limits: Limits) -> Result<Counts> {
    let counts = asset.validate(limits)?;
    let missing = asset
        .meshes
        .iter()
        .flat_map(|m| &m.primitives)
        .filter(|p| p.adjacency.is_none())
        .try_fold(0usize, |bytes, p| {
            bytes.checked_add(p.indices.len().checked_mul(4)?)
        })
        .ok_or_else(|| crate::Error("adjacency byte overflow".into()))?;
    crate::ensure(
        counts
            .decoded_bytes
            .checked_add(missing)
            .is_some_and(|bytes| bytes <= limits.max_decoded_bytes),
        "adjacency decoded byte limit",
    )?;
    for primitive in asset.meshes.iter_mut().flat_map(|m| &mut m.primitives) {
        if primitive.adjacency.is_none() {
            primitive.adjacency = Some(
                renderlab_asset::adjacency::build(&primitive.vertices, &primitive.indices)
                    .neighbours,
            );
        }
    }
    Ok(asset.validate(limits)?)
}

/// Publish a verified v3 payload alongside its original, with bounded reads and atomic rename.
pub fn prepare_file(source: &Path, output: &Path, limits: Limits) -> Result<serde_json::Value> {
    crate::ensure(
        source.canonicalize()?
            != output
                .canonicalize()
                .unwrap_or_else(|_| output.to_path_buf()),
        "adjacency preparation requires a different output path",
    )?;
    let total = Instant::now();
    let bytes = cache::read_bounded(source, limits.max_file_bytes)?;
    let original_bytes = bytes.len();
    let source_sha256 = cache::hash_hex(&bytes);
    let mut asset = renderlab_asset::decode(&bytes, limits)?;
    drop(bytes);
    let start = Instant::now();
    let counts = prepare(&mut asset, limits)?;
    let prepare_ms = start.elapsed().as_secs_f64() * 1000.;
    let bytes = renderlab_asset::encode(&asset, limits)?;
    drop(asset); // Do not retain both decoded copies during verification of a large scan.
    let verified = renderlab_asset::decode(&bytes, limits)?;
    drop(verified);
    cache::atomic_publish(output, |file| {
        file.write_all(&bytes)?;
        Ok(())
    })?;
    Ok(
        serde_json::json!({"source":source,"output":output,"source_sha256":source_sha256,
        "payload_sha256":cache::hash_hex(&bytes),"original_bytes":original_bytes,
        "prepared_bytes":bytes.len(),"adjacency_bytes":counts.adjacency_bytes,
        "prepare_ms":prepare_ms,"total_ms":total.elapsed().as_secs_f64()*1000.}),
    )
}
