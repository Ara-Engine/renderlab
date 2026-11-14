//! Opt-in non-RT software reflection packing snapshot. Driver-built RT BLAS/TLAS are
//! not visible here. No GPU readback or retained copy.
use super::*;
use crate::reflection_software::{self as geometry, Blas};
use std::{
    fs::{self, File},
    hash::{Hash, Hasher},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock, Weak},
};

struct Config {
    directory: PathBuf,
    frame: u64,
    attempted: Mutex<Vec<Weak<()>>>,
}

const DISK_FLOOR: u64 = 10 * 1024 * 1024 * 1024;

fn render_identity(renderer: &Renderer) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    renderer.source.hash(&mut h);
    renderer.source_digest.hash(&mut h);
    renderer.hardware_source.hash(&mut h);
    renderer.fresh_source.hash(&mut h);
    for program in &renderer.programs {
        program.source.hash(&mut h);
    }
    if let Some(state) = renderer.view.primary.as_ref() {
        state.bindings.as_ref().map(|b| &b.key).hash(&mut h);
    }
    h.finish()
}

#[cfg(windows)]
fn free_space(_: &Path) -> io::Result<u64> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            path: *const u16,
            available: *mut u64,
            total: *mut u64,
            free: *mut u64,
        ) -> i32;
    }
    let mut available = 0;
    // The unit's output disk is D:, regardless of the checkout's drive.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            [68u16, 58, 92, 0].as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(available)
    }
}

#[cfg(not(windows))]
fn free_space(path: &Path) -> io::Result<u64> {
    let mut existing = path;
    while !existing.exists() {
        existing = existing
            .parent()
            .ok_or_else(|| io::Error::other("dump path has no existing ancestor"))?;
    }
    let output = std::process::Command::new("df")
        .args(["-Pk", "--"])
        .arg(existing)
        .output()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let blocks = text
        .lines()
        .last()
        .and_then(|l| l.split_whitespace().nth(3))
        .and_then(|n| n.parse::<u64>().ok());
    blocks
        .filter(|_| output.status.success())
        .and_then(|n| n.checked_mul(1024))
        .ok_or_else(|| io::Error::other("cannot determine dump free space"))
}

/// The unset case returns before inspecting draws or repacking any geometry.
pub(super) fn maybe_dump(renderer: &Renderer, draws: &[PreparedDraw<'_>], reserved: usize) {
    static CONFIG: OnceLock<Option<Config>> = OnceLock::new();
    let Some(config) = CONFIG.get_or_init(|| {
        std::env::var_os("RENDERLAB_REFLECTION_GEOMETRY_DUMP")
            .filter(|p| !p.is_empty())
            .map(|directory| Config {
                directory: directory.into(),
                frame: std::env::var("RENDERLAB_REFLECTION_GEOMETRY_DUMP_FRAME")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(30),
                attempted: Mutex::new(Vec::new()),
            })
    }) else {
        return;
    };
    if renderer.frames != config.frame {
        return;
    }
    let Some(state) = renderer
        .view
        .primary
        .as_ref()
        .and_then(|p| p.reflections.as_ref())
    else {
        return;
    };
    let renderer_id = Arc::downgrade(&renderer.identity);
    let mut attempted = config.attempted.lock().unwrap();
    if attempted.iter().any(|id| id.ptr_eq(&renderer_id)) {
        return;
    }
    attempted.push(renderer_id);
    drop(attempted);
    let bytes = state.buffer.value.size().saturating_add(1024 * 1024);
    match free_space(&config.directory) {
        Ok(free) if free >= DISK_FLOOR.saturating_add(bytes) => {}
        free => {
            eprintln!(
                "reflection_geometry_dump refused: needs payload plus {DISK_FLOOR} bytes free; available={free:?}"
            );
            return;
        }
    }
    let before = render_identity(renderer);
    let stem = format!(
        "renderer-{:x}-view-{}-frame-{}-generation-{}",
        Arc::as_ptr(&renderer.identity) as usize,
        renderer.view_key.0,
        renderer.frames,
        state.generation,
    );
    if config.directory.join(format!("{stem}.json")).exists() {
        return;
    }
    let meshes: Vec<_> = state.meshes.iter().map(|m| &**m).collect();
    // These are the original model bits. Packed records contain their inverse, not the
    // original matrix; provenance labels are absent from Mesh/PreparedDraw and stay null.
    let mut sources = String::from("[");
    let mut slot = 0u32;
    for (draw_index, draw) in draws.iter().enumerate() {
        if draw_index != 0 {
            sources.push(',');
        }
        let mesh = draw
            .mesh
            .reflection
            .as_ref()
            .and_then(|b| state.meshes.iter().position(|m| Arc::ptr_eq(m, b)));
        let mesh_json = mesh.map_or_else(|| "null".into(), |i| i.to_string());
        sources.push_str(&format!(
            "{{\"draw\":{draw_index},\"mesh\":{mesh_json},\"label\":null,\"origin\":null,\"skinned\":{},\"index_range\":[{},{}],\"placements\":[",
            draw.mesh.skin.is_some(), draw.indices.start, draw.indices.end,
        ));
        for (i, model) in draw.instances.state.models.iter().enumerate() {
            if i != 0 {
                sources.push(',');
            }
            sources.push_str(&format!(
                "{{\"slot\":{slot},\"model_bits\":{:?},\"casts_shadows\":{},\"included\":{}}}",
                model.to_cols_array().map(f32::to_bits),
                draw.instances.state.casts_shadows(i),
                mesh.is_some() && draw.mesh.skin.is_none() && model.determinant() != 0.0,
            ));
            slot += 1;
        }
        sources.push_str("]}");
    }
    sources.push(']');
    match write_snapshot(
        &config.directory,
        &stem,
        renderer.frames,
        renderer.view_key.0,
        state.generation,
        reserved,
        &meshes,
        &state.bases,
        &state.records,
        &sources,
    ) {
        // Diagnostics must not reject a render because a destination is unavailable.
        Err(error) => eprintln!("reflection_geometry_dump: {error}"),
        Ok(bytes) => eprintln!(
            "reflection_geometry_dump path=nonrt-software frame={} bytes={bytes} file={stem}.json",
            renderer.frames
        ),
    }
    assert_eq!(
        before,
        render_identity(renderer),
        "CPU dump changed module text, shading heads or bindings key"
    );
}

#[allow(clippy::too_many_arguments)]
fn write_snapshot(
    directory: &Path,
    stem: &str,
    frame: u64,
    view: u64,
    generation: u64,
    reserved: usize,
    meshes: &[&Blas],
    bases: &[[u32; 2]],
    records: &[u32],
    sources: &str,
) -> io::Result<u64> {
    let mut packed = geometry::pack(meshes, reserved);
    if packed.bases != bases
        || records.len() > packed.words.len()
        || records.len() < geometry::HEADER_WORDS
    {
        return Err(io::Error::other(
            "packing snapshot does not match live geometry",
        ));
    }
    packed.words[..records.len()].copy_from_slice(records);
    fs::create_dir_all(directory)?;
    let mut payload = BufWriter::new(File::create(directory.join(format!("{stem}.words.bin")))?);
    for word in &packed.words {
        payload.write_all(&word.to_le_bytes())?;
    }
    payload.flush()?;
    // Publish metadata last: an interrupted binary write is not a complete snapshot.
    let mut meta = BufWriter::new(File::create(directory.join(format!("{stem}.json")))?);
    write!(
        meta,
        "{{\"version\":1,\"path\":\"nonrt-software-reflections\",\"hardware_acceleration_structures\":false,\"format\":\"reflection-bvh2-u32-le\",\"frame\":{frame},\"view\":{view},\"generation\":{generation},\"written_instances\":{},\"reserved_instances\":{reserved},\"header_words\":{},\"instance_words\":{},\"records_words\":{},\"payload_words\":{},\"tlas\":{{\"base_word\":{},\"root\":{},\"refits\":{}}},\"meshes\":[",
        records[0],
        geometry::HEADER_WORDS,
        geometry::INSTANCE_WORDS,
        records.len(),
        packed.words.len(),
        records[1],
        records[2],
        records[3],
    )?;
    for (i, mesh) in meshes.iter().enumerate() {
        if i != 0 {
            write!(meta, ",")?;
        }
        let info = mesh.geometry_info();
        write!(
            meta,
            "{{\"mesh\":{i},\"generation_id\":\"{generation}:{i}\",\"label\":null,\"origin\":null,\"node_base_word\":{},\"triangle_base_word\":{},\"nodes\":{},\"triangles\":{},\"root\":{},\"depth\":{},\"bounds_bits\":{:?}}}",
            bases[i][0],
            bases[i][1],
            info.nodes,
            info.triangles,
            info.root,
            info.depth,
            info.bounds.map(|b| b.map(f32::to_bits)),
        )?;
    }
    write!(meta, "],\"source_draws\":{sources}}}\n")?;
    meta.flush()?;
    Ok(
        fs::metadata(directory.join(format!("{stem}.words.bin")))?.len()
            + fs::metadata(directory.join(format!("{stem}.json")))?.len(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_contains_exact_live_records_and_mesh_words() {
        let mesh = Blas::build(&[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]], &[0, 1, 2]).unwrap();
        let refs = [&mesh];
        let packed = geometry::pack(&refs, 2);
        let mut records = geometry::instance_words(&[
            (7, Mat4::IDENTITY, &mesh, packed.bases[0], false),
            (8, Mat4::ZERO, &mesh, packed.bases[0], true),
        ]);
        records.resize(geometry::HEADER_WORDS + 2 * geometry::INSTANCE_WORDS, 0);
        crate::instance_bvh::place(
            &mut records,
            geometry::HEADER_WORDS,
            geometry::INSTANCE_WORDS,
            2,
            &[],
        );
        let path = std::env::temp_dir().join(format!(
            "renderlab-reflection-dump-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        write_snapshot(
            &path,
            "sample",
            30,
            0,
            2,
            2,
            &refs,
            &packed.bases,
            &records,
            "[]",
        )
        .unwrap();
        let meta: serde_json::Value =
            serde_json::from_slice(&fs::read(path.join("sample.json")).unwrap()).unwrap();
        assert_eq!(meta["written_instances"], 1);
        assert_eq!(meta["reserved_instances"], 2);
        assert_eq!(meta["meshes"][0]["triangles"], 1);
        assert_eq!(meta["meshes"][0]["depth"], 0);
        let mut expected = packed.words;
        expected[..records.len()].copy_from_slice(&records);
        let bytes: Vec<u8> = expected.iter().flat_map(|w| w.to_le_bytes()).collect();
        assert_eq!(fs::read(path.join("sample.words.bin")).unwrap(), bytes);
        fs::remove_dir_all(path).unwrap();
    }
}
