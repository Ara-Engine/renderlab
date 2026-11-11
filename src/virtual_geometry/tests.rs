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
            tangents: None,
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
fn page_union_matches_sorted_dedup_for_ordered_and_unordered_sets() {
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    for case in 0..4000 {
        let set = |next: &mut dyn FnMut(u64) -> u64| -> Vec<PageId> {
            (0..next(40)).map(|_| PageId(next(64) as u32)).collect()
        };
        let (mut a, mut b) = (set(&mut next), set(&mut next));
        // Most runtime inputs are already strictly increasing; keep both kinds.
        if case % 4 != 0 {
            for s in [&mut a, &mut b] {
                s.sort_unstable();
                s.dedup();
            }
        }
        let mut expected = a.clone();
        expected.extend(&b);
        expected.sort_unstable();
        expected.dedup();
        let mut actual = a.clone();
        union_into(&mut actual, &b);
        assert_eq!(actual, expected, "a={a:?} b={b:?}");
    }
}
#[test]
fn page_pool_leases_lowest_free_slots_within_the_charged_page_count() {
    // A slot holds any page the codec admits, on a vertex boundary.
    assert!(pool::SLOT_BYTES >= PAGE_BYTES as u64);
    assert_eq!(pool::SLOT_BYTES % 48, 0);
    let mut pool = pool::PagePool::new(4 * PAGE_BYTES + PAGE_BYTES - 1);
    let mut held: Vec<_> = (0..4).map(|_| pool.acquire().unwrap()).collect();
    assert_eq!(
        held.iter().map(|s| s.index).collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
    assert!(pool.acquire().is_none(), "one slot per whole charged page");
    held.remove(2);
    held.remove(0);
    let first = pool.acquire().unwrap();
    let second = pool.acquire().unwrap();
    assert_eq!(
        (first.index, second.index),
        (0, 2),
        "dropped leases return, lowest first"
    );
    assert!(pool.acquire().is_none());
    drop(second);
    assert_eq!(pool.acquire().map(|s| s.index), Some(2));
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
fn same_view_reserves_later_placements_published_output() {
    for selector in [SelectorMode::Baseline, SelectorMode::SimpleReference] {
        let (mut package, _) = fixture(2);
        // One coarse cluster becomes two fine clusters. Each placement has independent pages.
        package.meshes[0].clusters.pop();
        package.meshes[0].clusters[2].indices.count = 6;
        package.meshes[0].groups[1].clusters.count = 1;
        let mut second = package.meshes[0].clone();
        for c in &mut second.clusters {
            c.page.0 += 2;
        }
        package.meshes.push(second);
        package.pages.extend(package.pages.clone());
        let mut asset = VirtualAsset::new(
            package,
            directory("same-view-output"),
            StreamOptions {
                max_clusters: 4,
                selector,
                ..Default::default()
            },
        )
        .unwrap();
        // A separate offscreen scene's minimum stays reserved in the aggregate cap.
        let other = VirtualScene::new(
            &asset,
            &[VirtualPlacement {
                mesh: 0,
                transform: Mat4::from_translation(Vec3::new(100000., 0., 0.)),
            }],
        )
        .unwrap();
        let mut scene = VirtualScene::new(
            &asset,
            &[
                VirtualPlacement {
                    mesh: 0,
                    transform: Mat4::IDENTITY,
                },
                VirtualPlacement {
                    mesh: 1,
                    transform: Mat4::IDENTITY,
                },
            ],
        )
        .unwrap();
        scene
            .select(&mut asset, &[false, true, true, true], &camera(4.), 1080)
            .unwrap();
        assert_eq!(
            scene
                .selections
                .iter()
                .map(|s| s.cut.clusters.len())
                .collect::<Vec<_>>(),
            [1, 2]
        );
        let later = scene.selections[1].cut.clone();
        asset.revision += 1;
        scene
            .select(&mut asset, &[true; 4], &camera(4.), 1080)
            .unwrap();
        assert_eq!(scene.selections[1].cut, later);
        assert_eq!(
            scene
                .selections
                .iter()
                .map(|s| s.cut.clusters.len())
                .sum::<usize>(),
            3
        );
        assert_eq!(asset.stats.cut_clusters, 4);
        // New views can rebuild/coarsen, but admission and the other scene's minimum still hold.
        scene
            .select(&mut asset, &[true; 4], &camera(4000.), 1080)
            .unwrap();
        assert_eq!(asset.stats.cut_clusters, 3);
        drop(other);
    }
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
    let io = PageIo::new(dir.clone(), Duration::from_millis(100), READ_SLOTS).unwrap();
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
    let io = PageIo::new(dir, Duration::from_secs(60), READ_SLOTS).unwrap();
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

// A frozen conservative watermark isolates admission from asynchronous queue progress. The
// separate wave test still requires real unfinished draws while evicting their last-use pages.
pub(crate) fn admission_accounting(r: &mut Renderer) {
    // Recounted from every state, independently of the incremental totals.
    fn scanned_charge(a: &VirtualAsset) -> usize {
        a.states
            .iter()
            .filter_map(PageState::gpu)
            .map(|p| PAGE_BYTES + p.staging)
            .sum()
    }
    fn scan_policy(a: &mut VirtualAsset, charge: usize, protected: &[PageId], completed: u64) {
        for id in 0..a.states.len() {
            if scanned_charge(a) + charge <= a.options.geometry_bytes {
                break;
            }
            if protected.binary_search(&PageId(id as u32)).is_ok() {
                continue;
            }
            if matches!(a.states[id], PageState::Resident(_)) {
                let PageState::Resident(p) = a.set_state(id, PageState::Missing) else {
                    unreachable!()
                };
                a.stats.inflight_retirements += u64::from(p.last_use > completed);
                a.set_state(id, PageState::Retiring(p));
                a.stats.evictions += 1;
                a.revision += 1;
                a.snapshot();
            }
            a.complete(completed);
        }
    }
    fn state_summary(a: &VirtualAsset, completed: u64) -> Vec<(PageStatus, usize, bool, bool)> {
        a.states
            .iter()
            .map(|s| {
                let p = s.gpu();
                (
                    s.status(),
                    p.map_or(0, |p| p.staging),
                    p.is_some_and(|p| p.upload <= completed),
                    p.is_some_and(|p| p.last_use <= completed),
                )
            })
            .collect()
    }
    fn fixture_asset(r: &mut Renderer) -> (VirtualAsset, u64, u64) {
        let (package, blobs) = fixture(10);
        let data = decode_page(
            &package.pages[0],
            &blobs[0],
            Sha256::digest(&blobs[0]).into(),
        )
        .unwrap();
        let mut a = VirtualAsset::new(
            package,
            PathBuf::from("measurements/m4-stream/admission/accounting-test"),
            StreamOptions {
                geometry_bytes: 8 * PAGE_BYTES + 624,
                ..Default::default()
            },
        )
        .unwrap();
        for id in [0, 1, 2, 3, 5, 6, 9] {
            let mut batch = None;
            let mut page = r
                .try_upload_virtual_page(&data, &mut a.pool, &mut batch)
                .unwrap()
                .unwrap();
            r.submit_virtual_uploads(batch).unwrap();
            r.test_drain("admission fixture upload");
            if id != 3 {
                page.staging = 0;
            }
            a.set_state(
                id,
                match id {
                    3 => PageState::Uploading(page),
                    5 | 6 => PageState::Retiring(page),
                    _ => PageState::Resident(page),
                },
            );
        }
        let completed = r.completed_submission();
        let mut batch = None;
        let page = r
            .try_upload_virtual_page(&data, &mut a.pool, &mut batch)
            .unwrap()
            .unwrap();
        r.submit_virtual_uploads(batch).unwrap();
        let later = page.upload;
        assert!(later > completed);
        a.set_state(4, PageState::Uploading(page));
        if let PageState::Resident(p) = &mut a.states[1] {
            p.last_use = later;
        }
        if let PageState::Retiring(p) = &mut a.states[6] {
            p.last_use = later;
        }
        a.set_state(7, PageState::Queued);
        a.set_state(8, PageState::Ready(data));
        a.outstanding = 2;
        a.snapshot();
        r.test_drain("admission frozen watermark fixture complete");
        (a, completed, later)
    }
    for protected in [
        vec![PageId(0), PageId(9)],
        vec![PageId(0), PageId(1), PageId(2), PageId(9)],
        (0..10).map(PageId).collect(),
    ] {
        for charge in [0, 312, PAGE_BYTES, 2 * PAGE_BYTES, 5 * PAGE_BYTES] {
            let (mut actual, completed, later) = fixture_asset(r);
            let (mut expected, reference_completed, _) = fixture_asset(r);
            actual.make_room(charge, &protected, completed);
            scan_policy(&mut expected, charge, &protected, reference_completed);
            actual.snapshot();
            expected.snapshot();
            assert_eq!(
                state_summary(&actual, completed),
                state_summary(&expected, reference_completed)
            );
            let a = actual.stats();
            let b = expected.stats();
            assert_eq!(
                (
                    a.states,
                    [
                        a.geometry_bytes,
                        a.geometry_peak,
                        a.published_bytes,
                        a.retiring_bytes,
                        a.retiring_peak,
                        a.staging_bytes,
                        a.staging_peak,
                        a.owned_gpu_geometry_bytes,
                        a.payload_bytes,
                        a.pinned_bytes,
                        a.cpu_ready_bytes,
                    ],
                    a.evictions,
                    a.inflight_retirements
                ),
                (
                    b.states,
                    [
                        b.geometry_bytes,
                        b.geometry_peak,
                        b.published_bytes,
                        b.retiring_bytes,
                        b.retiring_peak,
                        b.staging_bytes,
                        b.staging_peak,
                        b.owned_gpu_geometry_bytes,
                        b.payload_bytes,
                        b.pinned_bytes,
                        b.cpu_ready_bytes,
                    ],
                    b.evictions,
                    b.inflight_retirements
                ),
                "charge={charge} protected={protected:?}"
            );
            assert_eq!(actual.revision, expected.revision);
            assert_eq!(a.geometry_bytes, actual.geometry_charge());
            assert_eq!(scanned_charge(&actual), actual.geometry_charge());
            assert!(a.geometry_peak <= actual.options.geometry_bytes);
            assert_eq!(actual.status(PageId(0)), Some(PageStatus::Resident));
            assert_eq!(actual.status(PageId(9)), Some(PageStatus::Resident));
            assert_eq!(actual.status(PageId(4)), Some(PageStatus::Uploading));
            assert_eq!(actual.status(PageId(6)), Some(PageStatus::Retiring));
            assert!(actual.states[4].gpu().unwrap().staging > 0);
            if charge == 5 * PAGE_BYTES {
                assert!(actual.geometry_charge() + charge > actual.options.geometry_bytes);
            }
            actual.complete(later);
            actual.snapshot();
            assert_eq!(actual.status(PageId(4)), Some(PageStatus::Resident));
            assert_eq!(actual.status(PageId(6)), Some(PageStatus::Missing));
            assert_eq!(actual.stats().staging_bytes, 0);
            assert_eq!(actual.stats().retiring_bytes, 0);
            assert_eq!(actual.stats().geometry_bytes, actual.geometry_charge());
        }
    }
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
    assert_eq!(io::load(&dir, &path, StreamOptions::default()).unwrap(), p);
    let mut corrupt = bytes.clone();
    corrupt[30] ^= 1;
    fs::write(&file, &corrupt).unwrap();
    assert!(io::load(&dir, &path, StreamOptions::default()).is_err());
    // A large manifest with only a header must fail its working-set gate before reading/allocating the declared body.
    let resources = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as u64;
    manifest[72..80].copy_from_slice(&(METADATA_LIMIT as u64 + resources).to_le_bytes());
    fs::write(&path, &manifest).unwrap();
    fs::write(&file, &bytes[..24]).unwrap();
    let error = io::load(&dir, &path, StreamOptions::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains("bootstrap capacity"), "{error}");
}

#[test]
fn forced_candidate_motion_reports_retained_cut_liveness() {
    for cap in [1, 64] {
        let levels = cap + 8;
        let (mut package, _) = fixture(1);
        let template = package.meshes[0].clone();
        package.meshes[0].groups.clear();
        package.meshes[0].clusters.clear();
        package.pages = vec![package.pages[0].clone(); levels];
        for g in 0..levels {
            let mut group = template.groups[0].clone();
            group.depth = g as u32;
            group.clusters.offset = (g * 2) as u32;
            group.simplified.error = if g + 1 == levels {
                f32::MAX
            } else {
                (g + 1) as f32 * 0.02
            };
            package.meshes[0].groups.push(group);
            for c in &template.clusters {
                let mut c = c.clone();
                c.owner = GroupId(g as u32);
                c.refined = g.checked_sub(1).map(|r| GroupId(r as u32));
                c.page = PageId(g as u32);
                package.meshes[0].clusters.push(c);
            }
        }
        for selector in [SelectorMode::Baseline, SelectorMode::SimpleReference] {
            let mut a = VirtualAsset::new(
                package.clone(),
                directory("motion-liveness"),
                StreamOptions {
                    max_candidates: cap,
                    selector,
                    ..Default::default()
                },
            )
            .unwrap();
            let mut scene = VirtualScene::from_package(&a).unwrap();
            let available: Vec<_> = (0..levels).map(|p| p >= 4).collect();
            for _ in 0..levels {
                scene.select(&mut a, &available, &camera(4.), 1080).unwrap();
            }
            assert_eq!(scene.selections()[0].cut.pages, [PageId(4)]);
            let mut stalled = 0;
            for frame in 0..5 {
                scene
                    .select(
                        &mut a,
                        &available,
                        &camera(4. + frame as f32 * 0.01 + 0.01),
                        1080,
                    )
                    .unwrap();
                let s = &scene.selections()[0];
                assert!(s.candidates <= cap);
                assert!(s.cut.pages.iter().all(|p| available[p.index()]));
                stalled += usize::from(
                    s.debt.unresolved_groups > 0 && s.demand.requested_pages.is_empty(),
                );
            }
            println!(
                "forced-motion cap={cap} selector={selector:?} debt-without-demand={stalled}/5 retained={}",
                a.stats.retained_cuts
            );
            assert_eq!(stalled, 0, "moving retention discarded useful root demand");
        }
    }
}

#[test]
fn moving_resident_root_prefix_cannot_starve_retained_refinement() {
    for selector in [SelectorMode::Baseline, SelectorMode::SimpleReference] {
        let (p, _) = fixture(4);
        let mut a = VirtualAsset::new(
            p,
            directory("resident-prefix"),
            StreamOptions {
                max_candidates: 1,
                selector,
                ..Default::default()
            },
        )
        .unwrap();
        let mut scene = VirtualScene::from_package(&a).unwrap();
        let mut available = [false, true, true, true];
        for _ in 0..8 {
            scene
                .select(&mut a, &available, &camera(22.), 1080)
                .unwrap();
        }
        assert_eq!(scene.selections()[0].cut.pages, [PageId(1)]);
        assert!(scene.selections()[0].demand.requested_pages.is_empty());
        let mut requested = false;
        for frame in 0..4 {
            scene
                .select(&mut a, &available, &camera(21. - frame as f32 * 0.01), 1080)
                .unwrap();
            let s = &scene.selections()[0];
            println!(
                "resident-prefix frame={frame} candidates={} debt={} requested={:?} cut={:?}",
                s.candidates, s.debt.unresolved_groups, s.demand.requested_pages, s.cut.pages
            );
            assert!(s.candidates <= 1);
            if s.demand.requested_pages.contains(&PageId(0)) {
                requested = true;
                available[0] = true;
                a.revision += 1;
            }
        }
        assert!(
            requested,
            "resident coarse prefix consumed every moving allowance"
        );
        assert_eq!(scene.selections()[0].cut.pages, [PageId(0)]);
        // Stop immediately on the retained turn at a far view. Its owed root query
        // must still coarsen without another camera change or availability event.
        drop(scene);
        let mut scene = VirtualScene::from_package(&a).unwrap();
        for _ in 0..8 {
            scene
                .select(&mut a, &available, &camera(22.), 1080)
                .unwrap();
        }
        assert_eq!(scene.selections()[0].cut.pages, [PageId(1)]);
        scene
            .select(&mut a, &available, &camera(21.), 1080)
            .unwrap();
        scene
            .select(&mut a, &available, &camera(4000.), 1080)
            .unwrap();
        assert_eq!(scene.selections()[0].cut.pages, [PageId(1)]);
        scene
            .select(&mut a, &available, &camera(4000.), 1080)
            .unwrap();
        assert_eq!(scene.selections()[0].cut.pages, [PageId(3)]);
        let before = a.stats().selection_runs;
        for _ in 0..300 {
            scene
                .select(&mut a, &available, &camera(4000.), 1080)
                .unwrap();
        }
        assert_eq!(a.stats().selection_runs, before);
        for frame in 0..12 {
            scene
                .select(&mut a, &available, &camera(21. - frame as f32 * 0.01), 1080)
                .unwrap();
            assert!(a.stats().candidates <= 1);
        }
        assert_eq!(
            scene.selections()[0].cut.pages,
            [PageId(0)],
            "near revisit pinned the far cover"
        );
    }
}

#[test]
fn capped_root_rebuild_releases_redundant_detail_and_refines_its_new_cover() {
    for selector in [SelectorMode::Baseline, SelectorMode::SimpleReference] {
        let (p, _) = fixture(4);
        let mut a = VirtualAsset::new(
            p,
            directory("coarsen-revisit"),
            StreamOptions {
                max_candidates: 1,
                selector,
                ..Default::default()
            },
        )
        .unwrap();
        let mut scene = VirtualScene::from_package(&a).unwrap();
        for _ in 0..12 {
            scene.select(&mut a, &[true; 4], &camera(4.), 1080).unwrap();
        }
        assert_eq!(scene.selections()[0].cut.pages, [PageId(0)]);
        for frame in 0..12 {
            scene
                .select(
                    &mut a,
                    &[true; 4],
                    &camera(22. + frame as f32 * 0.001),
                    1080,
                )
                .unwrap();
            assert!(a.stats().candidates <= 1);
        }
        println!(
            "capped-coarsening cut={:?} debt={}",
            scene.selections()[0].cut.pages,
            scene.selections()[0].debt.unresolved_groups
        );
        assert_eq!(
            scene.selections()[0].cut.pages,
            [PageId(1)],
            "resolved fine cut pinned redundant detail across capped root rebuilds"
        );
        for frame in 0..12 {
            scene
                .select(
                    &mut a,
                    &[true; 4],
                    &camera(21. - frame as f32 * 0.001),
                    1080,
                )
                .unwrap();
            assert!(a.stats().candidates <= 1);
        }
        assert_eq!(scene.selections()[0].cut.pages, [PageId(0)]);
    }
}

#[test]
fn bounded_turns_leave_unattainable_debt_quiet() {
    for output_limited in [false, true] {
        let (mut p, _) = fixture(if output_limited { 2 } else { 3 });
        if output_limited {
            p.meshes[0].clusters.pop();
            p.meshes[0].clusters[2].indices.count = 6;
            p.meshes[0].groups[1].clusters.count = 1;
        }
        let mut a = VirtualAsset::new(
            p,
            directory("unattainable-progress"),
            StreamOptions {
                max_candidates: 1,
                max_clusters: if output_limited { 1 } else { 8 },
                geometry_bytes: if output_limited {
                    8 << 20
                } else {
                    3 * PAGE_BYTES
                },
                ..Default::default()
            },
        )
        .unwrap();
        let mut scene = VirtualScene::from_package(&a).unwrap();
        let available = if output_limited {
            vec![false, true]
        } else {
            vec![false, true, true]
        };
        for frame in 0..8 {
            scene
                .select(&mut a, &available, &camera(4. + frame as f32 * 0.01), 1080)
                .unwrap();
            assert!(a.stats().candidates <= 1);
            assert!(scene.selections()[0].demand.requested_pages.is_empty());
            assert!(
                scene.selections()[0]
                    .cut
                    .pages
                    .iter()
                    .all(|p| available[p.index()])
            );
        }
        for _ in 0..4 {
            scene
                .select(&mut a, &available, &camera(4.07), 1080)
                .unwrap();
        }
        assert!(
            scene.selections()[0]
                .debt
                .reasons
                .contains(&if output_limited {
                    DebtReason::OutputCapacity
                } else {
                    DebtReason::GeometryCapacity
                })
        );
        let before = a.stats().selection_runs;
        for _ in 0..300 {
            scene
                .select(&mut a, &available, &camera(4.07), 1080)
                .unwrap();
        }
        assert_eq!(a.stats().selection_runs, before);
    }
}

#[test]
fn capped_view_change_preserves_root_final_demand_with_multiple_placements() {
    for selector in [SelectorMode::Baseline, SelectorMode::SimpleReference] {
        let (p, _) = fixture(4);
        let mut a = VirtualAsset::new(
            p,
            directory("root-demand-retention"),
            StreamOptions {
                max_candidates: 1,
                selector,
                ..Default::default()
            },
        )
        .unwrap();
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
        let mut other = VirtualScene::new(
            &a,
            &[VirtualPlacement {
                mesh: 0,
                transform: Mat4::from_translation(Vec3::new(100000., 0., 0.)),
            }],
        )
        .unwrap();
        // Establish a finer resident cover, then remove an obsolete intermediate page.
        let mut available = [false, true, true, true];
        for _ in 0..8 {
            scene.select(&mut a, &available, &camera(4.), 1080).unwrap();
        }
        assert_eq!(scene.selections()[0].cut.pages, [PageId(1)]);
        available[2] = false;
        a.revision += 1;
        for frame in 0..3 {
            scene
                .select(
                    &mut a,
                    &available,
                    &camera(4. + frame as f32 * 0.01 + 0.01),
                    1080,
                )
                .unwrap();
            let s = &scene.selections()[0];
            assert_eq!(s.cut.pages, [PageId(1)]);
            assert_eq!(
                s.demand.requested_pages,
                [PageId(if frame == 0 { 2 } else { 0 })]
            );
            assert!(a.stats().candidates <= 1);
            assert!(scene.protected.len() * PAGE_BYTES <= a.options.geometry_bytes - PAGE_BYTES);
            other
                .select(
                    &mut a,
                    &available,
                    &camera(4. + frame as f32 * 0.01 + 0.01),
                    1080,
                )
                .unwrap();
            assert!(a.stats().candidates <= 1);
            assert_eq!(a.stats().cut_clusters, 6);
            assert_eq!(other.selections()[0].cut.clusters.len(), 2);
            let registry = a.registry.lock().unwrap();
            assert!(
                registry
                    .scenes
                    .iter()
                    .map(|e| e.output_reservation)
                    .sum::<usize>()
                    <= a.options.max_clusters
            );
            let mut union = vec![];
            for entry in &registry.scenes {
                union_into(&mut union, &entry.protected);
            }
            assert!(union.len() * PAGE_BYTES <= a.options.geometry_bytes - PAGE_BYTES);
        }
        // On a fresh fixed view the retained path receives the full allowance, requests
        // only final detail, publishes it and then stays cached for 300 CPU frames.
        scene
            .select(&mut a, &available, &camera(4.03), 1080)
            .unwrap();
        assert_eq!(scene.selections()[0].demand.requested_pages, [PageId(0)]);
        available[0] = true;
        a.revision += 1;
        for _ in 0..4 {
            scene
                .select(&mut a, &available, &camera(4.03), 1080)
                .unwrap();
        }
        assert_eq!(scene.selections()[0].cut.pages, [PageId(0)]);
        let before = a.stats().selection_runs;
        for _ in 0..300 {
            scene
                .select(&mut a, &available, &camera(4.03), 1080)
                .unwrap();
        }
        assert_eq!(a.stats().selection_runs, before);
    }
}

#[test]
fn shrinking_cuts_keep_minimum_output_admitted_across_scenes() {
    for selector in [SelectorMode::Baseline, SelectorMode::SimpleReference] {
        let (mut p, _) = fixture(2);
        p.meshes[0].clusters.remove(1);
        p.meshes[0].clusters[0].indices.count = 6;
        p.meshes[0].groups[0].clusters.count = 1;
        p.meshes[0].groups[1].clusters.offset = 1;
        let mut a = VirtualAsset::new(
            p,
            directory("shrinking-output"),
            StreamOptions {
                max_clusters: 4,
                selector,
                ..Default::default()
            },
        )
        .unwrap();
        let placements = [VirtualPlacement {
            mesh: 0,
            transform: Mat4::IDENTITY,
        }; 2];
        let mut scene = VirtualScene::new(&a, &placements).unwrap();
        scene.select(&mut a, &[true; 2], &camera(4.), 1080).unwrap();
        assert_eq!(
            scene
                .selections
                .iter()
                .map(|s| s.cut.clusters.len())
                .collect::<Vec<_>>(),
            [1, 1]
        );
        assert_eq!(a.stats.cut_clusters, 2);
        assert!(VirtualScene::new(&a, &placements[..1]).is_err());
        a.revision += 1;
        scene.select(&mut a, &[true; 2], &camera(4.), 1080).unwrap();
        scene
            .select(&mut a, &[true; 2], &camera(4000.), 1080)
            .unwrap();
        assert_eq!(a.stats.cut_clusters, 4);
    }
}

#[test]
fn saturated_debt_does_not_discard_a_resident_required_boundary() {
    let (mut p, _) = fixture(3);
    let (branch, _) = fixture(2);
    let offset = p.meshes[0].clusters.len() as u32;
    for mut group in branch.meshes[0].groups.clone() {
        group.clusters.offset += offset;
        p.meshes[0].groups.push(group);
    }
    for mut c in branch.meshes[0].clusters.clone() {
        c.owner.0 += 3;
        c.refined = c.refined.map(|g| GroupId(g.0 + 3));
        c.page.0 += 3;
        p.meshes[0].clusters.push(c);
    }
    p.pages.extend(branch.pages);
    p.validate(VirtualLimits::default()).unwrap();
    let mesh = &p.meshes[0];
    let budget = SelectionBudget {
        geometry_bytes: 5 * PAGE_BYTES,
        max_candidates: 20,
        max_clusters: 20,
        max_scratch_bytes: METADATA_LIMIT,
    };
    let old = select_with_protected(
        mesh,
        &[10., 20., f64::INFINITY, f64::INFINITY, f64::INFINITY],
        1.,
        &[true, false, true, false, true],
        budget,
        &[],
    )
    .unwrap();
    assert_eq!(old.cut.pages, [PageId(0), PageId(4)]);
    let errors = [0., 20., f64::INFINITY, f64::INFINITY, f64::INFINITY];
    let available = [true, false, true, true, true];
    let protected = [PageId(0), PageId(2), PageId(4)];
    let mut next =
        select_with_protected(mesh, &errors, 1., &available, budget, &protected).unwrap();
    assert_eq!(next.cut.pages, [PageId(2), PageId(3)]);
    assert_eq!(next.debt.maximum_denied_error, 20.);
    assert!(
        retain_better_cut(
            Some(&old),
            &mut next,
            &errors,
            1.,
            &available,
            &protected,
            RetentionBudget {
                roots: &[PageId(2), PageId(4)],
                geometry_bytes: budget.geometry_bytes,
                max_clusters: 20
            }
        ),
        "finite debt discarded a required resident boundary because unrelated debt was infinite"
    );
    assert_eq!(next.cut, old.cut);
    assert!(next.debt.maximum_denied_error.is_infinite());
    let refined =
        select_from_cut(mesh, &errors, 1., &available, budget, &protected, &next.cut).unwrap();
    assert_eq!(refined.cut.pages, [PageId(0), PageId(3)]);
    assert_eq!(refined.debt.unresolved_groups, 0);
    let finite_errors = [0., 20., f64::INFINITY, 40., f64::INFINITY];
    let mut finite =
        select_with_protected(mesh, &finite_errors, 1., &available, budget, &protected).unwrap();
    assert!(
        !retain_better_cut(
            Some(&old),
            &mut finite,
            &finite_errors,
            1.,
            &available,
            &protected,
            RetentionBudget {
                roots: &[PageId(2), PageId(4)],
                geometry_bytes: budget.geometry_bytes,
                max_clusters: 20
            }
        ),
        "finite debt comparisons must retain their existing ordering"
    );
}

#[test]
fn configured_data_and_runtime_limits_reach_bootstrap_and_preparation() {
    let (mut package, _) = fixture(3);
    let mut w = 4097u32;
    let mut mips = vec![];
    loop {
        mips.push(asset::Mip {
            width: w,
            height: 1,
            rgba8: vec![128; w as usize * 4],
        });
        if w == 1 {
            break;
        }
        w = (w / 2).max(1);
    }
    package.resources.textures.push(asset::Texture {
        role: asset::TextureRole::BaseColorSrgb,
        format: asset::TextureFormat::Uncompressed,
        mips,
    });
    assert!(package.validate(VirtualLimits::default()).is_err());
    let limits = VirtualLimits::unlimited();
    let bytes = asset::virtual_mesh::encode(&package, limits).unwrap();
    let dir = directory("configured-bootstrap");
    let packages = dir.join("virtual-v1/packages");
    fs::create_dir_all(&packages).unwrap();
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    fs::write(packages.join(io::hex(&digest)), &bytes).unwrap();
    let mut manifest = Vec::from(&b"RVM\0\x01\0\0\0"[..]);
    manifest.extend([0; 32]);
    manifest.extend(digest);
    manifest.extend((bytes.len() as u64).to_le_bytes());
    let path = dir.join("current");
    fs::write(&path, &manifest).unwrap();
    let options = StreamOptions {
        limits,
        metadata_bytes: 32 << 10,
        texture_bytes: 64 << 10,
        bootstrap_bytes: 128 << 10,
        geometry_bytes: 4 * PAGE_BYTES,
        read_slots: 1,
        uploads_per_tick: 1,
        ..Default::default()
    };
    let asset = VirtualAsset::open(&dir, &path, options).unwrap();
    assert_eq!(asset.package.resources.textures[0].mips[0].width, 4097);
    assert_eq!(asset.stats.cpu_io_bound_bytes, 2 * PAGE_BYTES + 1);
    VirtualScene::from_package(&asset).unwrap();
    for bad in [
        StreamOptions {
            bootstrap_bytes: bytes.len() - 1,
            ..options
        },
        StreamOptions {
            metadata_bytes: 100,
            ..options
        },
        StreamOptions {
            texture_bytes: 1,
            ..options
        },
        StreamOptions {
            geometry_bytes: PAGE_BYTES,
            ..options
        },
        StreamOptions {
            read_slots: 0,
            ..options
        },
        StreamOptions {
            uploads_per_tick: 0,
            ..options
        },
        StreamOptions {
            limits: VirtualLimits {
                max_pages: 2,
                ..limits
            },
            ..options
        },
        StreamOptions {
            limits: VirtualLimits {
                prepared: asset::Limits {
                    max_dimension: 4096,
                    ..asset::Limits::unlimited()
                },
                ..limits
            },
            ..options
        },
    ] {
        assert!(VirtualAsset::open(&dir, &path, bad).is_err(), "{bad:?}");
    }
    let mut malformed = package;
    malformed.meshes[0].clusters[0].refined = Some(GroupId(0));
    assert!(malformed.validate(limits).is_err());
    for n in 0..24 {
        assert!(asset::virtual_mesh::decode(&bytes[..n], limits).is_err());
    }
    let mut corrupt = bytes.clone();
    corrupt[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(asset::virtual_mesh::decode(&corrupt, limits).is_err());
    let large = VirtualAsset::new(
        asset::virtual_mesh::decode(&bytes, limits).unwrap(),
        dir,
        StreamOptions {
            read_slots: usize::MAX - 1,
            uploads_per_tick: usize::MAX - 1,
            ..options
        },
    )
    .unwrap();
    assert_eq!(large.stats.cpu_io_bound_bytes, 4 * PAGE_BYTES + 1);
}
