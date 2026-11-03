use super::*;
use crate::Vec3;
use asset::{Vertex, virtual_mesh::*};
use sha2::{Digest, Sha256};
use std::fs;

pub(crate) fn fixture(levels: usize) -> (Package, Vec<Vec<u8>>) {
    let mut mesh = VirtualMesh {
        groups: vec![],
        clusters: vec![],
    };
    let mut pages = vec![];
    let mut blobs = vec![];
    for level in 0..levels {
        let mut bytes = vec![0u8; PAGE_BYTES];
        let mut offset = 0;
        for x in [-0.65, 0.65] {
            for position in [[x - 0.6, -0.7, 0.], [x + 0.6, -0.7, 0.], [x, 0.8, 0.]] {
                let v = Vertex {
                    position,
                    normal: [0., 0., 1.],
                    uv: [0., 0.],
                    color: [1.; 4],
                };
                for value in v
                    .position
                    .into_iter()
                    .chain(v.normal)
                    .chain(v.uv)
                    .chain(v.color)
                {
                    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
                    offset += 4;
                }
            }
        }
        for i in 0u32..6 {
            bytes[offset..offset + 4].copy_from_slice(&i.to_le_bytes());
            offset += 4;
        }
        // Different page identity with identical positions, but different unused UV.
        bytes[24..28].copy_from_slice(&(level as f32).to_le_bytes());
        pages.push(Page {
            vertices: 6,
            indices: 6,
            payload_bytes: offset as u32,
            sha256: Sha256::digest(&bytes).into(),
        });
        blobs.push(bytes);
        let b = Bounds {
            center: [0.; 3],
            radius: 2.,
            error: if level == levels - 1 {
                f32::MAX
            } else {
                0.02 * 10_f32.powi(level as i32)
            },
        };
        mesh.groups.push(Group {
            depth: level as u32,
            simplified: b,
            clusters: Span {
                offset: mesh.clusters.len() as u32,
                count: 2,
            },
        });
        for material in 0..2 {
            mesh.clusters.push(Cluster {
                owner: GroupId(level as u32),
                refined: if level > 0 {
                    Some(GroupId(level as u32 - 1))
                } else {
                    None
                },
                bounds: b,
                material,
                page: PageId(level as u32),
                indices: Span {
                    offset: material * 3,
                    count: 3,
                },
            });
        }
    }
    let package = Package {
        meshes: vec![mesh],
        pages,
        placements: vec![asset::Placement {
            mesh: 0,
            transform: Mat4::IDENTITY.to_cols_array_2d(),
        }],
        resources: asset::Asset {
            materials: vec![
                asset::Material {
                    base_color: [1., 0.1, 0.1, 1.],
                    double_sided: false,
                    ..Default::default()
                },
                asset::Material {
                    base_color: [0.1, 0.8, 0.2, 1.],
                    double_sided: false,
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
    };
    package.validate(VirtualLimits::default()).unwrap();
    (package, blobs)
}
pub(crate) fn directory(label: &str) -> PathBuf {
    let p = PathBuf::from("measurements/m4-runtime-tests").join(format!(
        "{}-{}-{}",
        std::process::id(),
        label,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&p).unwrap();
    p
}
pub(crate) fn write_pages(package: &Package, blobs: &[Vec<u8>], dir: &Path) {
    for (p, b) in package.pages.iter().zip(blobs) {
        fs::write(dir.join(io::hex(&p.sha256)), b).unwrap();
    }
}
fn camera(distance: f32) -> Camera {
    Camera::look_at(
        Vec3::new(0., 0., distance),
        Vec3::ZERO,
        Vec3::Y,
        1.,
        0.05,
        10000.,
    )
    .unwrap()
}
#[test]
fn aggregate_admission_shared_pages_offscreen_and_deterministic_demand() {
    let (package, _) = fixture(3);
    let dir = directory("admission");
    let mut a = VirtualAsset::new(
        package,
        dir,
        StreamOptions {
            geometry_bytes: 4 * PAGE_BYTES,
            max_clusters: 4,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        VirtualScene::new(
            &a,
            &[VirtualPlacement {
                mesh: 0,
                transform: Mat4::IDENTITY
            }; 3]
        )
        .is_err()
    );
    let mut scene = VirtualScene::new(
        &a,
        &[
            VirtualPlacement {
                mesh: 0,
                transform: Mat4::IDENTITY,
            },
            VirtualPlacement {
                mesh: 0,
                transform: Mat4::from_translation(Vec3::new(100000., 0., 0.)),
            },
        ],
    )
    .unwrap();
    scene
        .select(&mut a, &[false, false, true], &camera(4.), 1080)
        .unwrap();
    assert_eq!(
        scene
            .selections
            .iter()
            .map(|s| s.cut.clusters.len())
            .sum::<usize>(),
        4
    );
    assert_eq!(scene.protected, vec![PageId(0), PageId(2)]);
    let selections = scene.selections.clone();
    scene.key = None;
    scene
        .select(&mut a, &[false, false, true], &camera(4.), 1080)
        .unwrap();
    assert_eq!(selections, scene.selections);
    assert_eq!(a.stats.reads, 0);
    assert_eq!(a.geometry_charge(), 0);
    let roots = a.minimum_geometry_bytes();
    let (package, _) = fixture(3);
    assert!(
        VirtualAsset::new(
            package.clone(),
            directory("too-small"),
            StreamOptions {
                geometry_bytes: roots - 1,
                ..Default::default()
            }
        )
        .is_err()
    );
    let mut minimal = VirtualAsset::new(
        package,
        directory("minimum"),
        StreamOptions {
            geometry_bytes: roots,
            ..Default::default()
        },
    )
    .unwrap();
    let mut scene = VirtualScene::from_package(&minimal).unwrap();
    scene
        .select(&mut minimal, &[false, false, true], &camera(4.), 1080)
        .unwrap();
    assert_eq!(scene.protected, vec![PageId(2)]);
    assert!(minimal.stats.residency_debt);
}
#[test]
fn conservative_projection_near_affine_scale_mirror_and_domain() {
    let (p, _) = fixture(3);
    let mesh = &p.meshes[0];
    let identity = project_errors(mesh, &camera(10.), Mat4::IDENTITY, 1080).unwrap();
    for scale in [Vec3::splat(2.), Vec3::new(-2., 1., 0.5)] {
        let errors = project_errors(mesh, &camera(10.), Mat4::from_scale(scale), 1080).unwrap();
        assert!(errors[0] > identity[0]);
        assert!(errors.windows(2).all(|e| e[0] <= e[1]));
    }
    assert!(
        project_errors(mesh, &camera(1.), Mat4::IDENTITY, 1080)
            .unwrap()
            .iter()
            .all(|e| e.is_infinite())
    );
    assert!(validate_transform(Mat4::from_scale(Vec3::new(0., 1., 1.))).is_err());
    assert!(validate_transform(Mat4::perspective_rh(1., 1., 0.1, 100.)).is_err());
}
#[test]
fn filesystem_worker_actual_hash_failure_delay_repair_and_shutdown() {
    let (p, blobs) = fixture(3);
    let dir = directory("worker");
    write_pages(&p, &blobs, &dir);
    let io = PageIo::new(dir.clone(), Duration::from_millis(100));
    let started = Instant::now();
    assert!(io.request(PageId(0), &p.pages[0]));
    assert!(io.receive().is_none());
    let result = loop {
        if let Some(r) = io.receive() {
            break r;
        }
        assert!(started.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(2));
    };
    assert!(started.elapsed() >= Duration::from_millis(100));
    assert!(result.geometry.is_ok());
    fs::write(dir.join(io::hex(&p.pages[0].sha256)), [0; 4]).unwrap();
    assert!(io.request(PageId(0), &p.pages[0]));
    let start = Instant::now();
    loop {
        if let Some(r) = io.receive() {
            assert!(r.geometry.is_err());
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(2));
    }
    fs::remove_file(dir.join(io::hex(&p.pages[1].sha256))).unwrap();
    assert!(io.request(PageId(1), &p.pages[1]));
    loop {
        if let Some(r) = io.receive() {
            assert!(r.geometry.is_err());
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(2));
    }
    write_pages(&p, &blobs, &dir);
    assert!(io.request(PageId(0), &p.pages[0]));
    loop {
        if let Some(r) = io.receive() {
            assert!(r.geometry.is_ok());
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(2));
    }
    drop(io);
    let io = PageIo::new(dir, Duration::from_secs(60));
    for i in 0..4 {
        assert!(io.request(PageId(i % 3), &p.pages[i as usize % 3]));
    }
    let started = Instant::now();
    drop(io);
    assert!(started.elapsed() < Duration::from_secs(1));
}
// Calls the production admission/retirement policy after a real render submission. This isolates
// the incomplete-last-use boundary from disk/hash timing without faking completion identities.
pub(crate) fn retire_for_inflight_test(a: &mut VirtualAsset, completed: u64) {
    a.make_room(
        PAGE_BYTES + 312,
        &[PageId(1), PageId(2), PageId(3)],
        completed,
    );
}
#[test]
fn simple_reference_matches_baseline_at_equal_budgets_and_shared_admission_releases() {
    let (p, _) = fixture(4);
    let m = &p.meshes[0];
    for mask in 0..16u32 {
        for capacity in 1..=6 {
            for output in [1, 2, 4, 8] {
                for candidates in 0..=4 {
                    let available: Vec<_> = (0..4).map(|p| mask & (1 << p) != 0).collect();
                    let errors = [2., 4., 8., f64::INFINITY];
                    let b = SelectionBudget {
                        geometry_bytes: capacity * PAGE_BYTES,
                        max_candidates: candidates,
                        max_clusters: output,
                        max_scratch_bytes: METADATA_LIMIT,
                    };
                    let actual = select_with_protected(m, &errors, 1., &available, b, &[]);
                    let simple = reference::select(m, &errors, 1., &available, b, &[], None);
                    match (actual, simple) {
                        (Ok(a), Ok(b)) => {
                            assert_eq!(a.cut, b.cut);
                            assert_eq!(a.demand, b.demand);
                            assert_eq!(a.debt, b.debt);
                        }
                        (Err(_), Err(_)) => {}
                        _ => panic!("selector admission disagreement"),
                    }
                }
            }
        }
    }
    let a = VirtualAsset::new(
        p,
        directory("shared-scenes"),
        StreamOptions {
            max_clusters: 4,
            ..Default::default()
        },
    )
    .unwrap();
    let first = VirtualScene::from_package(&a).unwrap();
    let second = VirtualScene::from_package(&a).unwrap();
    assert!(VirtualScene::from_package(&a).is_err());
    drop(first);
    assert!(VirtualScene::from_package(&a).is_ok());
    drop(second);
}
#[test]
fn better_published_cut_survives_missing_detail_without_exceeding_union_budget() {
    let (p, _) = fixture(3);
    let m = &p.meshes[0];
    let available = [true, false, true];
    let b = SelectionBudget {
        geometry_bytes: 3 * PAGE_BYTES,
        max_candidates: 10,
        max_clusters: 20,
        max_scratch_bytes: METADATA_LIMIT,
    };
    let old = select_with_protected(m, &[2., 4., f64::INFINITY], 1., &available, b, &[]).unwrap();
    assert_eq!(old.cut.pages, vec![PageId(0)]);
    let mut next =
        select_with_protected(m, &[0., 4., f64::INFINITY], 1., &available, b, &[]).unwrap();
    assert_eq!(next.cut.pages, vec![PageId(2)]);
    assert!(retain_better_cut(
        Some(&old),
        &mut next,
        &[0., 4., f64::INFINITY],
        1.,
        &available,
        &[],
        RetentionBudget {
            roots: &[PageId(2)],
            geometry_bytes: 3 * PAGE_BYTES,
            max_clusters: 20
        }
    ));
    assert_eq!(next.cut, old.cut);
    assert_eq!(next.debt.unresolved_groups, 0);
    assert!(next.demand.requested_pages.is_empty());
    let mut next =
        select_with_protected(m, &[0., 4., f64::INFINITY], 1., &available, b, &[]).unwrap();
    assert!(!retain_better_cut(
        Some(&old),
        &mut next,
        &[0., 4., f64::INFINITY],
        1.,
        &available,
        &[],
        RetentionBudget {
            roots: &[PageId(2)],
            geometry_bytes: PAGE_BYTES,
            max_clusters: 20
        }
    ));
}
#[test]
fn stationary_pressure_protects_current_cover_and_reports_capacity_without_reread_demand() {
    let (p, _) = fixture(3);
    let mut a = VirtualAsset::new(
        p,
        directory("no-self-eviction"),
        StreamOptions {
            geometry_bytes: 3 * PAGE_BYTES,
            ..Default::default()
        },
    )
    .unwrap();
    let mut scene = VirtualScene::from_package(&a).unwrap();
    scene
        .select(&mut a, &[false, true, true], &camera(100.), 1080)
        .unwrap();
    assert_eq!(scene.selections[0].cut.pages, [PageId(1)]);
    scene
        .select(&mut a, &[false, true, true], &camera(4.), 1080)
        .unwrap();
    assert_eq!(scene.selections[0].cut.pages, [PageId(1)]);
    assert!(scene.selections[0].demand.requested_pages.is_empty());
    assert!(
        scene.selections[0]
            .debt
            .reasons
            .contains(&asset::virtual_mesh::DebtReason::GeometryCapacity)
    );
}

#[test]
fn one_candidate_streaming_advances_from_published_cut_without_reloading_intermediates() {
    for selector in [SelectorMode::Baseline, SelectorMode::SimpleReference] {
        let (p, _) = fixture(4);
        let mut a = VirtualAsset::new(
            p,
            directory("one-candidate"),
            StreamOptions {
                max_candidates: 1,
                selector,
                ..Default::default()
            },
        )
        .unwrap();
        let mut scene = VirtualScene::from_package(&a).unwrap();
        let mut available = [false, false, false, true];
        for _ in 0..8 {
            let previous = scene.selections().first().map(|s| s.cut.clone());
            scene.select(&mut a, &available, &camera(4.), 1080).unwrap();
            assert!(a.stats().candidates <= 1);
            if scene.selections()[0].cut.pages == [PageId(0)] {
                break;
            }
            let request = scene.selections()[0].demand.requested_pages.clone();
            assert!(
                !request.is_empty() || previous.as_ref() != Some(&scene.selections()[0].cut),
                "candidate allowance spent on an obsolete root path: {:?}",
                scene.selections()
            );
            for p in request {
                assert!(!available[p.index()], "page requested twice");
                available[p.index()] = true;
                a.revision += 1;
            }
        }
        assert_eq!(scene.selections()[0].cut.pages, [PageId(0)]);
        let settled = a.stats().selection_runs;
        scene.select(&mut a, &available, &camera(4.), 1080).unwrap();
        assert_eq!(
            a.stats().selection_runs,
            settled,
            "finished candidate work did not quiesce"
        );
    }
}
#[test]
fn simple_current_cut_refinement_matches_baseline_and_avoids_obsolete_representations() {
    let (p, _) = fixture(4);
    let m = &p.meshes[0];
    let all = vec![true; 4];
    let b = SelectionBudget {
        geometry_bytes: 6 * PAGE_BYTES,
        max_candidates: 20,
        max_clusters: 20,
        max_scratch_bytes: METADATA_LIMIT,
    };
    let old = select_with_protected(m, &[0., 2., 4., f64::INFINITY], 1., &all, b, &[]).unwrap();
    for mask in 0..16 {
        let available: Vec<_> = (0..4).map(|i| mask & (1 << i) != 0).collect();
        for pages in 1..=5 {
            for candidates in 0..=4 {
                let b = SelectionBudget {
                    geometry_bytes: pages * PAGE_BYTES,
                    max_candidates: candidates,
                    ..b
                };
                let errors = [2., 4., 8., f64::INFINITY];
                let baseline =
                    select_from_cut(m, &errors, 1., &available, b, &old.cut.pages, &old.cut);
                let simple = reference::select(
                    m,
                    &errors,
                    1.,
                    &available,
                    b,
                    &old.cut.pages,
                    Some(&old.cut),
                );
                match (baseline, simple) {
                    (Ok(a), Ok(b)) => {
                        assert_eq!(a.cut, b.cut);
                        assert_eq!(a.demand, b.demand);
                        assert_eq!(a.debt, b.debt);
                    }
                    (Err(_), Err(_)) => {}
                    _ => panic!("current cut oracle disagreement"),
                }
            }
        }
    }
}
#[test]
fn metadata_bootstrap_hashes_real_bytes_and_rejects_oversized_declared_payload_before_read() {
    let (p, _) = fixture(3);
    let dir = directory("metadata-bootstrap");
    let packages = dir.join("virtual-v1/packages");
    fs::create_dir_all(&packages).unwrap();
    let bytes = asset::virtual_mesh::encode(&p, VirtualLimits::default()).unwrap();
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    let file = packages.join(io::hex(&digest));
    fs::write(&file, &bytes).unwrap();
    let mut manifest = Vec::from(&b"RVM\0\x01\0\0\0"[..]);
    manifest.extend([0; 32]);
    manifest.extend(digest);
    manifest.extend((bytes.len() as u64).to_le_bytes());
    let path = dir.join("current.manifest");
    fs::write(&path, &manifest).unwrap();
    assert_eq!(io::load(&dir, &path).unwrap(), p);
    let mut corrupt = bytes.clone();
    corrupt[30] ^= 1;
    fs::write(&file, &corrupt).unwrap();
    assert!(io::load(&dir, &path).is_err());
    // A large manifest with only a header must fail its working-set gate before reading/allocating the declared body.
    manifest[72..80].copy_from_slice(&(METADATA_LIMIT as u64).to_le_bytes());
    fs::write(&path, &manifest).unwrap();
    fs::write(&file, &bytes[..24]).unwrap();
    let error = io::load(&dir, &path).unwrap_err().to_string();
    assert!(error.contains("bootstrap capacity"), "{error}");
}
