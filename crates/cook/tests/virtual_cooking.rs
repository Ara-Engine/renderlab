use renderlab_asset::virtual_mesh::{self, VirtualLimits};
use renderlab_cook::{
    virtual_cache,
    virtual_cook::{self, Settings},
    virtual_fixtures, virtual_proof,
};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
fn dir(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("renderlab-virtual-{name}-{}", std::process::id()));
    fs::create_dir_all(&p).unwrap();
    p
}
#[test]
fn closed_meshopt_all_small_legal_cuts_and_material_page_integrity() {
    let l = VirtualLimits::default();
    for (asset, closed) in [
        (virtual_fixtures::closed_sphere(24).unwrap(), true),
        (virtual_fixtures::thin_sheet(12).unwrap(), false),
    ] {
        let cooked = virtual_cook::prepare(&asset, Settings::default(), l).unwrap();
        let pages: Vec<_> = cooked
            .package
            .pages
            .iter()
            .zip(&cooked.pages)
            .map(|(p, b)| virtual_mesh::decode_page(p, b, Sha256::digest(b).into()).unwrap())
            .collect();
        println!(
            "groups {} clusters {}",
            cooked.package.meshes[0].groups.len(),
            cooked.package.meshes[0].clusters.len()
        );
        println!(
            "{}",
            virtual_proof::prove(&cooked.package, &pages, closed, true).unwrap()
        );
        let leaf_indices: usize = cooked.package.meshes[0]
            .clusters
            .iter()
            .filter(|c| c.refined.is_none())
            .map(|c| c.indices.count as usize)
            .sum();
        assert_eq!(
            leaf_indices,
            asset.meshes[0]
                .primitives
                .iter()
                .map(|p| p.indices.len())
                .sum::<usize>()
        );
        assert_eq!(cooked.package.resources.materials, asset.materials);
        for (p, b) in cooked.package.pages.iter().zip(&cooked.pages) {
            let mut corrupt = b.clone();
            corrupt[0] ^= 1;
            assert!(
                virtual_mesh::decode_page(p, &corrupt, Sha256::digest(&corrupt).into()).is_err()
            );
            assert!(
                virtual_mesh::decode_page(p, &b[..b.len() - 1], Sha256::digest(b).into()).is_err()
            );
            let mut invalid_index = b.clone();
            invalid_index[p.vertices as usize * 48..p.vertices as usize * 48 + 4]
                .copy_from_slice(&p.vertices.to_le_bytes());
            let mut expected = p.clone();
            expected.sha256 = Sha256::digest(&invalid_index).into();
            assert!(virtual_mesh::decode_page(&expected, &invalid_index, expected.sha256).is_err());
            let mut invalid_scalar = b.clone();
            invalid_scalar[..4].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
            expected.sha256 = Sha256::digest(&invalid_scalar).into();
            assert!(
                virtual_mesh::decode_page(&expected, &invalid_scalar, expected.sha256).is_err()
            );
        }
        let again = virtual_cook::prepare(&asset, Settings::default(), l).unwrap();
        assert_eq!(again.package, cooked.package);
        assert_eq!(again.pages, cooked.pages);
    }
}
#[test]
fn virtual_upstream_numeric_domain_is_explicit() {
    let mut a = virtual_fixtures::closed_sphere(4).unwrap();
    a.meshes[0].primitives[0].vertices[0].position[0] = 1e30;
    a.validate(renderlab_asset::Limits::default()).unwrap();
    let result = virtual_cook::prepare(&a, Settings::default(), VirtualLimits::default());
    assert!(matches!(result,Err(error) if error.0.contains("magnitude")));
}
#[test]
fn virtual_cache_cold_warm_changed_source_settings_corruption_and_partial_manifest() {
    let d = dir("cache");
    let source = d.join("source.rla");
    let current = d.join("current");
    let l = VirtualLimits::default();
    let mut a = virtual_fixtures::closed_sphere(8).unwrap();
    fs::write(&source, renderlab_asset::encode(&a, l.prepared).unwrap()).unwrap();
    let cold = virtual_cache::cook_file(&source, &d, &current, Settings::default(), l).unwrap();
    assert!(!cold.report.cache_hit);
    let warm = virtual_cache::cook_file(&source, &d, &current, Settings::default(), l).unwrap();
    assert!(warm.report.cache_hit);
    assert_eq!(cold.loaded.package_sha256, warm.loaded.package_sha256);
    let page = virtual_cache::page_path(&d, &warm.loaded.package.pages[0]);
    let original = fs::read(&page).unwrap();
    let mut bad = original.clone();
    bad[0] ^= 1;
    fs::write(&page, bad).unwrap();
    assert!(virtual_cache::load_manifest(&d, &current, l).is_err());
    let repaired = virtual_cache::cook_file(&source, &d, &current, Settings::default(), l).unwrap();
    assert!(!repaired.report.cache_hit);
    assert_eq!(fs::read(&page).unwrap(), original);
    let accepted = fs::read(&current).unwrap();
    fs::write(&current, &accepted[..17]).unwrap();
    assert!(virtual_cache::load_manifest(&d, &current, l).is_err());
    assert!(
        virtual_cache::cook_file(&source, &d, &current, Settings::default(), l)
            .unwrap()
            .report
            .cache_hit
    );
    let changed =
        virtual_cache::cook_file(&source, &d, &current, Settings { max_triangles: 64 }, l).unwrap();
    assert!(!changed.report.cache_hit);
    assert_ne!(changed.loaded.key, cold.loaded.key);
    a.materials[0].roughness = 0.5;
    fs::write(&source, renderlab_asset::encode(&a, l.prepared).unwrap()).unwrap();
    let changed = virtual_cache::cook_file(&source, &d, &current, Settings::default(), l).unwrap();
    assert!(!changed.report.cache_hit);
    assert_ne!(changed.loaded.key, cold.loaded.key);
    let accepted = fs::read(&current).unwrap();
    fs::write(&source, b"bad/truncated").unwrap();
    assert!(virtual_cache::cook_file(&source, &d, &current, Settings::default(), l).is_err());
    assert_eq!(fs::read(&current).unwrap(), accepted);
    // Every created path is a child of this explicit test directory; no recursive removal.
    fn clean(p: &std::path::Path) {
        for e in fs::read_dir(p).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                clean(&e.path());
                fs::remove_dir(e.path()).unwrap();
            } else {
                fs::remove_file(e.path()).unwrap();
            }
        }
    }
    clean(&d);
    fs::remove_dir(d).unwrap();
}

#[test]
fn original_materials_textures_and_placements_are_retained_without_full_detail_blob() {
    for (_, source) in renderlab_cook::fixtures::sources().unwrap() {
        let a = renderlab_cook::prepare(
            &source,
            renderlab_cook::Settings::default(),
            renderlab_cook::CookLimits::default(),
        )
        .unwrap();
        let cooked =
            virtual_cook::prepare(&a, Settings::default(), VirtualLimits::default()).unwrap();
        assert_eq!(cooked.package.resources.materials, a.materials);
        assert_eq!(cooked.package.resources.textures, a.textures);
        assert_eq!(cooked.package.placements, a.placements);
        assert!(cooked.package.resources.meshes.is_empty());
        assert!(cooked.package.resources.placements.is_empty());
        let b = virtual_mesh::encode(&cooked.package, VirtualLimits::default()).unwrap();
        assert_eq!(
            virtual_mesh::decode(&b, VirtualLimits::default()).unwrap(),
            cooked.package
        );
    }
}
