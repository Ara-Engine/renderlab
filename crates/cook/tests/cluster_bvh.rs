use renderlab_asset::{
    self as asset,
    cluster_bvh::{self as format, LEAF_BIT, Limits, Ray},
    virtual_mesh::{self, VirtualLimits},
};
use renderlab_cook::{
    cluster_bvh::{self, Settings},
    cluster_bvh_cache, virtual_cache, virtual_cook, virtual_fixtures,
};
use sha2::{Digest, Sha256};
fn fixture() -> (asset::Asset, virtual_cook::Cooked) {
    let a = virtual_fixtures::closed_sphere(24).unwrap();
    let c = virtual_cook::prepare(
        &a,
        virtual_cook::Settings::default(),
        VirtualLimits::default(),
    )
    .unwrap();
    (a, c)
}
fn build(c: &virtual_cook::Cooked, errors: Vec<f32>) -> format::Bvh {
    cluster_bvh::prepare(
        &c.package,
        &Settings {
            errors,
            limits: Limits::default(),
        },
        VirtualLimits::default(),
        |p| {
            Ok(virtual_mesh::decode_page(
                &c.package.pages[p],
                &c.pages[p],
                Sha256::digest(&c.pages[p]).into(),
            )?)
        },
    )
    .unwrap()
}
#[test]
fn legal_lod_cuts_quantization_and_cpu_rays_are_deterministic() {
    assert_eq!(std::mem::size_of::<format::Node>(), format::NODE_BYTES);
    assert_eq!(std::mem::size_of::<format::Leaf>(), format::LEAF_BYTES);
    assert_eq!(std::mem::size_of::<format::Level>(), format::LEVEL_BYTES);
    for a in [
        virtual_fixtures::closed_sphere(24).unwrap(),
        virtual_fixtures::thin_sheet(12).unwrap(),
    ] {
        let c = virtual_cook::prepare(
            &a,
            virtual_cook::Settings::default(),
            VirtualLimits::default(),
        )
        .unwrap();
        let b = build(&c, vec![0., 0.02, 0.1]);
        let bytes = format::encode(&b, Limits::default()).unwrap();
        assert_eq!(format::decode(&bytes, Limits::default()).unwrap(), b);
        for _ in 0..8 {
            assert_eq!(
                format::encode(&build(&c, vec![0., 0.02, 0.1]), Limits::default()).unwrap(),
                bytes
            );
        }
        for level in 0..b.levels.len() {
            let l = b.levels[level];
            let chosen = b.levels[b.select_level(l.mesh, l.requested_error, 1.).unwrap()];
            assert!(
                chosen.max_error <= l.requested_error
                    && chosen.requested_error >= l.requested_error
            );
            for i in 0..300 {
                let theta = i as f64 * 2.399963229728653;
                let y = (i as f64 + 0.5) / 150. - 1.;
                let r = (1. - y * y).sqrt();
                let dir = [r * theta.cos(), y, r * theta.sin()];
                let ray = Ray {
                    origin: dir.map(|v| v * 4.),
                    direction: dir.map(|v| -v),
                    t_min: 0.,
                    t_max: 10.,
                };
                assert_eq!(b.trace(level, ray), b.trace_brute(level, ray));
            }
            for dir in [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]] {
                for origin in [[0.; 3], [20.; 3]] {
                    let ray = Ray {
                        origin,
                        direction: dir,
                        t_min: 0.,
                        t_max: f64::INFINITY,
                    };
                    assert_eq!(b.trace(level, ray), b.trace_brute(level, ray));
                }
            }
        }
        assert!(b.select_level(0, f32::NAN, 1.).is_none());
        assert!(b.select_level(99, 1., 1.).is_none());
        assert!(b.select_level(0, 1., -1.).is_none());
    }
}
#[test]
fn attachments_are_opt_in_and_old_rla_rvg_bytes_are_preserved() {
    let (a, c) = fixture();
    let b = build(&c, vec![0.02, 0.1]);
    assert!(b.select_level(0, 0., 1.).is_none());
    let limits = VirtualLimits {
        max_metadata_bytes: 32 << 20,
        ..VirtualLimits::default()
    };
    let old = asset::encode(&a, limits.prepared).unwrap();
    let new = asset::encode_with_bvh(&a, Some(&b), limits.prepared).unwrap();
    assert_eq!(&new[4..8], &asset::CLUSTER_BVH_SCHEMA_VERSION.to_le_bytes());
    assert_eq!(
        asset::decode_with_bvh(&new, limits.prepared).unwrap(),
        (a.clone(), Some(b.clone()))
    );
    assert_eq!(asset::decode(&new, limits.prepared).unwrap(), a);
    assert_eq!(
        asset::encode_with_bvh(&a, None, limits.prepared).unwrap(),
        old
    );
    let old = virtual_mesh::encode(&c.package, limits).unwrap();
    let new = virtual_mesh::encode_with_bvh(&c.package, Some(&b), limits).unwrap();
    assert_eq!(&new[4..8], &virtual_mesh::CLUSTER_BVH_VERSION.to_le_bytes());
    assert_eq!(
        virtual_mesh::decode_with_bvh(&new, limits).unwrap(),
        (c.package.clone(), Some(b.clone()))
    );
    assert_eq!(
        virtual_mesh::encode_with_bvh(&c.package, None, limits).unwrap(),
        old
    );
    let mut wrong = b.clone();
    wrong.leaves[0].page += 1;
    assert!(virtual_mesh::encode_with_bvh(&c.package, Some(&wrong), limits).is_err());
}
#[test]
fn malformed_bvhs_and_budgets_fail_before_publication() {
    let (_, c) = fixture();
    let b = build(&c, vec![0., 0.1]);
    let mut bad = b.clone();
    let last = bad.nodes.len() - 1;
    bad.nodes[last].children[0] = last as u32;
    assert!(bad.validate(Limits::default()).is_err());
    let mut bad = b.clone();
    bad.nodes[0].step[0] = 0.;
    assert!(bad.validate(Limits::default()).is_err());
    let mut bad = b.clone();
    bad.nodes[0].errors[0] = 100.;
    assert!(bad.validate(Limits::default()).is_err());
    let mut bad = b.clone();
    bad.levels[0].root = LEAF_BIT | (bad.leaves.len() as u32);
    assert!(bad.validate(Limits::default()).is_err());
    let mut bad = b.clone();
    bad.level_leaves[0] = bad.level_leaves[1];
    assert!(bad.validate(Limits::default()).is_err());
    let bytes = format::encode(&b, Limits::default()).unwrap();
    assert!(format::decode(&bytes[..bytes.len() - 1], Limits::default()).is_err());
    assert!(
        format::decode(
            &bytes,
            Limits {
                max_bytes: bytes.len() - 1
            }
        )
        .is_err()
    );
    let mut bad = bytes.clone();
    bad[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(format::decode(&bad, Limits::default()).is_err());
    assert!(
        Settings {
            errors: vec![0.1, 0.1],
            limits: Limits::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        cluster_bvh::prepare(
            &c.package,
            &Settings {
                errors: vec![0.1],
                limits: Limits { max_bytes: 1 }
            },
            VirtualLimits::default(),
            |_| panic!("must pre-admit before page reads")
        )
        .is_err()
    );
}
#[test]
fn opt_in_cache_reuses_repairs_and_separates_lod_settings() {
    let (a, _) = fixture();
    let d = std::env::temp_dir().join(format!("renderlab-r0c-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let limits = VirtualLimits::default();
    let source = d.join("source.rla");
    std::fs::write(&source, asset::encode(&a, limits.prepared).unwrap()).unwrap();
    let raster = d.join("raster.manifest");
    virtual_cache::cook_file(
        &source,
        &d,
        &raster,
        virtual_cook::Settings::default(),
        limits,
    )
    .unwrap();
    let input = cluster_bvh_cache::load_geometry(&d, &raster, limits).unwrap();
    let before = std::fs::read(&raster).unwrap();
    let manifest = d.join("rays.manifest");
    let settings = Settings {
        errors: vec![0.02, 0.1],
        limits: Limits::default(),
    };
    let cold = cluster_bvh_cache::cook(&input, &d, &d, &manifest, &settings, limits).unwrap();
    assert!(!cold.cache_hit);
    let warm = cluster_bvh_cache::cook(&input, &d, &d, &manifest, &settings, limits).unwrap();
    assert!(warm.cache_hit);
    assert_eq!(cold.digest, warm.digest);
    let package = d.join(format!(
        "cluster-bvh-v1/rla1-rvg1/packages/{}",
        cold.digest
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect::<String>()
    ));
    std::fs::write(package, b"broken").unwrap();
    let repaired = cluster_bvh_cache::cook(&input, &d, &d, &manifest, &settings, limits).unwrap();
    assert!(!repaired.cache_hit);
    assert_eq!(cold.digest, repaired.digest);
    let changed = cluster_bvh_cache::cook(
        &input,
        &d,
        &d,
        &manifest,
        &Settings {
            errors: vec![0.05, 0.2],
            ..settings
        },
        limits,
    )
    .unwrap();
    assert_ne!(cold.key, changed.key);
    let published = std::fs::read(&manifest).unwrap();
    assert!(
        cluster_bvh_cache::cook(
            &input,
            &d,
            &d,
            &manifest,
            &Settings {
                errors: vec![0.1, 0.1],
                limits: Limits::default()
            },
            limits
        )
        .is_err()
    );
    assert_eq!(published, std::fs::read(&manifest).unwrap());
    assert_eq!(before, std::fs::read(raster).unwrap());
    std::fs::remove_dir_all(d).unwrap();
}
