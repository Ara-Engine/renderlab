use renderlab_cook::*;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "renderlab-cook-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn source() -> Vec<u8> {
    fixtures::sources().unwrap()[0].1.clone()
}
fn mutate(bytes: &[u8], change: impl FnOnce(&mut Value, &mut Vec<u8>)) -> Vec<u8> {
    let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let mut root: Value = serde_json::from_slice(&bytes[20..20 + json_len]).unwrap();
    let mut bin = bytes[28 + json_len..].to_vec();
    change(&mut root, &mut bin);
    let mut json = serde_json::to_vec(&root).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let mut output = b"glTF".to_vec();
    output.extend(2u32.to_le_bytes());
    output.extend(((28 + json.len() + bin.len()) as u32).to_le_bytes());
    output.extend((json.len() as u32).to_le_bytes());
    output.extend(0x4e4f534au32.to_le_bytes());
    output.extend(json);
    output.extend((bin.len() as u32).to_le_bytes());
    output.extend(0x004e4942u32.to_le_bytes());
    output.extend(bin);
    output
}
fn rejected(bytes: &[u8], needle: &str) {
    let error = prepare(bytes, Settings::default(), CookLimits::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains(needle), "expected {needle:?}, got {error:?}");
}

#[test]
fn real_fixtures_preserve_owned_geometry_materials_transforms_roles_and_samplers() {
    let first = fixtures::sources().unwrap();
    assert_eq!(first, fixtures::sources().unwrap());
    let a = prepare(&first[0].1, Settings::default(), CookLimits::default()).unwrap();
    let c = a.validate(CookLimits::default().prepared).unwrap();
    assert_eq!(
        (
            c.meshes,
            c.primitives,
            c.placements,
            c.materials,
            c.textures,
            c.vertices,
            c.indices
        ),
        (1, 2, 4, 2, 2, 24, 36)
    );
    assert!(a.placements.iter().all(|p| p.mesh == 0));
    assert_eq!(a.placements[0].transform[3], [-1.5, 0.25, 0.0, 1.0]);
    assert_eq!(a.placements[1].transform[0][0], -0.7);
    assert_eq!(a.placements[2].transform[3], [2.0, 1.9000001, 0.0, 1.0]);
    assert_eq!(a.meshes[0].primitives[0].vertices[0].uv, [0.0, 0.0]);
    assert_eq!(a.meshes[0].primitives[0].vertices[2].uv, [1.0, 1.0]);
    assert_eq!(
        a.meshes[0].primitives[0].vertices[4].color,
        [128.0 / 255.0, 1.0, 191.0 / 255.0, 1.0]
    );
    assert_eq!(a.materials[0].base_color, [1.0, 0.75, 0.5, 1.0]);
    let color = a.materials[0].base_color_texture.unwrap();
    let data = a.materials[0].metallic_roughness_texture.unwrap();
    assert_ne!(color.texture, data.texture);
    assert_ne!(color.sampler, data.sampler);
    assert_eq!(
        a.textures[color.texture as usize].role,
        renderlab_asset::TextureRole::BaseColorSrgb
    );
    assert_eq!(
        a.textures[data.texture as usize].role,
        renderlab_asset::TextureRole::MetallicRoughnessLinear
    );
    assert_eq!(
        a.textures[color.texture as usize].mips[0].rgba8,
        a.textures[data.texture as usize].mips[0].rgba8
    );
    assert_ne!(
        a.textures[color.texture as usize].mips[1].rgba8,
        a.textures[data.texture as usize].mips[1].rgba8
    );
    let b = prepare(&first[1].1, Settings::default(), CookLimits::default()).unwrap();
    let c = b.validate(CookLimits::default().prepared).unwrap();
    assert_eq!(
        (
            c.meshes,
            c.primitives,
            c.placements,
            c.vertices,
            c.indices,
            c.textures
        ),
        (1, 2, 2, 1333, 6984, 3)
    );
    assert_eq!(
        b.meshes[0].primitives[1].vertices[0].normal,
        [0.0, 0.0, 1.0]
    );
    assert_eq!(
        b.textures[0]
            .mips
            .iter()
            .map(|m| (m.width, m.height))
            .collect::<Vec<_>>(),
        [(5, 3), (2, 1), (1, 1)]
    );
    assert_eq!(
        b.textures[2]
            .mips
            .iter()
            .map(|m| (m.width, m.height))
            .collect::<Vec<_>>(),
        [(7, 5), (3, 2), (1, 1)]
    );
    assert!(
        prepare(
            &first[1].1,
            Settings {
                missing_normals: MissingNormals::Reject
            },
            CookLimits::default()
        )
        .is_err()
    );
}

#[test]
fn malformed_spans_indices_normalization_and_limits_fail_before_readers() {
    let bytes = source();
    for n in [0, 3, 12, 19, 20, bytes.len() - 1] {
        assert!(prepare(&bytes[..n], Settings::default(), CookLimits::default()).is_err());
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    rejected(&trailing, "length mismatch");
    rejected(
        &mutate(&bytes, |r, _| {
            r["bufferViews"][0]["byteLength"] = json!(u32::MAX)
        }),
        "span",
    );
    rejected(
        &mutate(&bytes, |r, _| r["accessors"][0]["byteOffset"] = json!(4000)),
        "span",
    );
    rejected(
        &mutate(&bytes, |r, _| r["bufferViews"][0]["byteStride"] = json!(8)),
        "stride",
    );
    rejected(
        &mutate(&bytes, |r, _| r["accessors"][0]["count"] = json!(u32::MAX)),
        "count limit",
    );
    rejected(
        &mutate(&bytes, |r, _| r["accessors"][1]["count"] = json!(11)),
        "attribute counts",
    );
    rejected(
        &mutate(&bytes, |r, _| {
            r["accessors"][1]["normalized"] = json!(false)
        }),
        "normalization",
    );
    rejected(
        &mutate(&bytes, |r, b| {
            let index = r["meshes"][0]["primitives"][0]["indices"].as_u64().unwrap() as usize;
            let view = r["accessors"][index]["bufferView"].as_u64().unwrap() as usize;
            let start = r["bufferViews"][view]["byteOffset"].as_u64().unwrap() as usize;
            b[start..start + 4].copy_from_slice(&999u32.to_le_bytes());
        }),
        "vertex index",
    );
    rejected(
        &mutate(&bytes, |_, b| {
            b[..4].copy_from_slice(&f32::NAN.to_le_bytes())
        }),
        "nonfinite",
    );
    let limits = CookLimits {
        source_bytes: 1,
        ..CookLimits::default()
    };
    assert!(prepare(&bytes, Settings::default(), limits).is_err());
    let limits = CookLimits {
        image_decoded_bytes: 1,
        ..CookLimits::default()
    };
    assert!(prepare(&bytes, Settings::default(), limits).is_err());
    let limits = CookLimits {
        prepared: renderlab_asset::Limits {
            max_dimension: 2,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(prepare(&bytes, Settings::default(), limits).is_err());
}

#[test]
fn unsupported_features_missing_resources_and_invalid_hierarchies_are_named() {
    let bytes = source();
    rejected(
        &mutate(&bytes, |r, _| r["buffers"][0]["uri"] = json!("file.bin")),
        "URI",
    );
    rejected(
        &mutate(&bytes, |r, _| {
            r["images"][0].as_object_mut().unwrap().remove("bufferView");
            r["images"][0]["uri"] = json!("missing.png");
        }),
        "URI",
    );
    rejected(
        &mutate(&bytes, |r, _| r["animations"] = json!([{}])),
        "animations",
    );
    rejected(&mutate(&bytes, |r, _| r["skins"] = json!([{}])), "skins");
    rejected(
        &mutate(&bytes, |r, _| {
            r["meshes"][0]["primitives"][0]["targets"] = json!([{"POSITION":0}])
        }),
        "morph",
    );
    rejected(
        &mutate(&bytes, |r, _| {
            r["meshes"][0]["primitives"][0]["mode"] = json!(1)
        }),
        "topology",
    );
    rejected(
        &mutate(&bytes, |r, _| {
            r["materials"][0]["alphaMode"] = json!("BLEND")
        }),
        "alphaMode",
    );
    rejected(
        &mutate(&bytes, |r, _| {
            r["materials"][0]["emissiveFactor"] = json!([1, 0, 0])
        }),
        "emissive",
    );
    rejected(
        &mutate(&bytes, |r, _| {
            r["materials"][0]["normalTexture"] = json!({"index":0})
        }),
        "normalTexture",
    );
    rejected(
        &mutate(&bytes, |r, _| {
            r["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["texCoord"] = json!(1)
        }),
        "coordinate set",
    );
    rejected(
        &mutate(&bytes, |r, _| {
            r["meshes"][0]["primitives"][0]["attributes"]
                .as_object_mut()
                .unwrap()
                .remove("TEXCOORD_0");
        }),
        "requires UV0",
    );
    rejected(
        &mutate(&bytes, |r, _| {
            r["materials"][0]["extensions"] = json!({"KHR_materials_unlit":{}})
        }),
        "extensions",
    );
    rejected(
        &mutate(&bytes, |r, _| r["nodes"][4]["children"] = json!([0])),
        "cycle",
    );
    rejected(
        &mutate(&bytes, |r, _| r["nodes"][1]["children"] = json!([4])),
        "multiple parents",
    );
    rejected(
        &mutate(&bytes, |r, _| r["nodes"][1]["scale"] = json!([0, 1, 1])),
        "nonsingular",
    );
    rejected(
        &mutate(&bytes, |r, _| {
            r["accessors"][0]
                .as_object_mut()
                .unwrap()
                .remove("bufferView");
        }),
        "Missing data",
    );
}

fn cook(dir: &Sandbox, source: &Path, manifest: &Path, settings: Settings) -> CookResult {
    cook_file(
        source,
        &dir.path("cache"),
        manifest,
        settings,
        CookLimits::default(),
    )
    .unwrap()
}
#[test]
fn cache_identity_repair_transform_reuse_and_failure_preservation() {
    let dir = Sandbox::new();
    let bytes = source();
    let source = dir.path("source.glb");
    let manifest = dir.path("current.manifest");
    fs::write(&source, &bytes).unwrap();
    let cold = cook(&dir, &source, &manifest, Settings::default());
    assert!(!cold.report.cache_hit);
    let warm = cook(&dir, &source, &manifest, Settings::default());
    assert!(warm.report.cache_hit);
    assert_eq!(cold.loaded.payload_hash, warm.loaded.payload_hash);
    let key_path = dir.path("cache/keys").join(&cold.loaded.key);
    fs::write(&key_path, b"partial manifest").unwrap();
    let repaired_manifest = cook(&dir, &source, &manifest, Settings::default());
    assert!(!repaired_manifest.report.cache_hit);
    assert!(
        repaired_manifest
            .report
            .cache_miss_reason
            .unwrap()
            .contains("truncated")
    );
    let corrupt = dir.path("cache/assets").join(&cold.loaded.payload_hash);
    fs::write(&corrupt, b"corrupt").unwrap();
    assert!(load_manifest(&dir.path("cache"), &manifest, CookLimits::default()).is_err());
    let repaired = cook(&dir, &source, &manifest, Settings::default());
    assert!(!repaired.report.cache_hit);
    assert!(
        repaired
            .report
            .cache_miss_reason
            .unwrap()
            .contains("mismatch")
    );
    assert_eq!(cold.loaded.payload_hash, repaired.loaded.payload_hash);
    let geom = dir
        .path("cache/geometry")
        .join(&cold.loaded.geometry_hashes[0]);
    let mut geometry = fs::read(&geom).unwrap();
    geometry[20] ^= 1;
    fs::write(&geom, &geometry).unwrap();
    assert!(load_manifest(&dir.path("cache"), &manifest, CookLimits::default()).is_err());
    assert!(
        !cook(&dir, &source, &manifest, Settings::default())
            .report
            .cache_hit
    );
    fs::remove_file(&geom).unwrap();
    assert!(
        !cook(&dir, &source, &manifest, Settings::default())
            .report
            .cache_hit
    );
    fs::write(
        &source,
        mutate(&bytes, |r, _| {
            r["nodes"][0]["translation"] = json!([-2.0, 0.25, 0.0])
        }),
    )
    .unwrap();
    let moved = cook(&dir, &source, &manifest, Settings::default());
    assert!(!moved.report.cache_hit);
    assert_ne!(cold.loaded.key, moved.loaded.key);
    assert_ne!(cold.loaded.payload_hash, moved.loaded.payload_hash);
    assert_eq!(cold.loaded.geometry_hashes, moved.loaded.geometry_hashes);
    assert_eq!(moved.report.reused_geometry_entries, 1);
    let settings = cook(
        &dir,
        &source,
        &manifest,
        Settings {
            missing_normals: MissingNormals::Reject,
        },
    );
    assert!(!settings.report.cache_hit);
    assert_ne!(settings.loaded.key, moved.loaded.key);
    assert_eq!(settings.loaded.payload_hash, moved.loaded.payload_hash);
    fs::write(
        &source,
        mutate(&bytes, |_, b| {
            b[..4].copy_from_slice(&1.25f32.to_le_bytes())
        }),
    )
    .unwrap();
    let edited_geometry = cook(&dir, &source, &manifest, Settings::default());
    assert!(!edited_geometry.report.cache_hit);
    assert_ne!(
        cold.loaded.geometry_hashes,
        edited_geometry.loaded.geometry_hashes
    );
    assert_eq!(edited_geometry.report.reused_geometry_entries, 0);
    let previous = fs::read(&manifest).unwrap();
    fs::write(&source, b"broken GLB").unwrap();
    assert!(
        cook_file(
            &source,
            &dir.path("cache"),
            &manifest,
            Settings::default(),
            CookLimits::default()
        )
        .is_err()
    );
    assert_eq!(previous, fs::read(&manifest).unwrap());
    assert!(load_manifest(&dir.path("cache"), &manifest, CookLimits::default()).is_ok());
}

#[test]
fn concurrent_writers_publish_complete_entries_and_manifests() {
    let dir = Sandbox::new();
    let source = dir.path("source.glb");
    fs::write(&source, source_bytes()).unwrap();
    let cache = dir.path("cache");
    let manifest = dir.path("current");
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    cook_file(
                        &source,
                        &cache,
                        &manifest,
                        Settings::default(),
                        CookLimits::default(),
                    )
                    .unwrap()
                    .loaded
                    .payload_hash
                })
            })
            .collect();
        let hashes: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
        assert!(hashes.iter().all(|h| h == &hashes[0]));
    });
    load_manifest(&cache, &manifest, CookLimits::default()).unwrap();
}
fn source_bytes() -> Vec<u8> {
    source()
}
