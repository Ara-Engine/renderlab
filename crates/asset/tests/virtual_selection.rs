use renderlab_asset::{Asset, Material, virtual_mesh::*};

fn graph(groups: &[&[Option<u32>]]) -> VirtualMesh {
    let mut mesh = VirtualMesh {
        groups: vec![],
        clusters: vec![],
    };
    let mut incoming = vec![false; groups.len()];
    for members in groups {
        for id in members.iter().flatten() {
            incoming[*id as usize] = true;
        }
    }
    for (i, members) in groups.iter().enumerate() {
        let bounds = Bounds {
            center: [0.; 3],
            radius: 1.,
            error: if incoming[i] { i as f32 + 1. } else { f32::MAX },
        };
        mesh.groups.push(Group {
            depth: i as u32,
            simplified: bounds,
            clusters: Span {
                offset: mesh.clusters.len() as u32,
                count: members.len() as u32,
            },
        });
        for refined in *members {
            mesh.clusters.push(Cluster {
                owner: GroupId(i as u32),
                refined: refined.map(GroupId),
                bounds,
                material: 0,
                page: PageId(mesh.clusters.len() as u32),
                indices: Span {
                    offset: 0,
                    count: 3,
                },
            });
        }
    }
    mesh
}
fn budget() -> SelectionBudget {
    SelectionBudget {
        geometry_bytes: usize::MAX,
        max_candidates: usize::MAX,
        max_clusters: usize::MAX,
        max_scratch_bytes: 8 << 20,
    }
}
fn legal(mesh: &VirtualMesh, active: &[bool]) -> bool {
    mesh.terminal_groups().all(|g| active[g.index()])
        && mesh.clusters.iter().all(|c| {
            c.refined
                .is_none_or(|r| !active[r.index()] || active[c.owner.index()])
        })
}
/// Independent oracle: transcribes upstream group-rule predicates, never calls select.
fn reference(mesh: &VirtualMesh, errors: &[f64], threshold: f64) -> Vec<ClusterId> {
    mesh.clusters
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            errors[c.owner.index()] > threshold
                && c.refined.is_none_or(|r| errors[r.index()] <= threshold)
        })
        .map(|(i, _)| ClusterId(i as u32))
        .collect()
}
fn exact_once(mesh: &VirtualMesh, cut: &Cut) {
    let active: Vec<_> = (0..mesh.groups.len())
        .map(|g| cut.active_groups.contains(&GroupId(g as u32)))
        .collect();
    assert!(legal(mesh, &active));
    let mut selected = vec![false; mesh.clusters.len()];
    for g in mesh.terminal_groups() {
        for i in mesh.groups[g.index()]
            .clusters
            .range(selected.len())
            .unwrap()
        {
            selected[i] = true;
        }
    }
    // Replay each replacement once, removing every incoming shard before emitting children.
    for g in (0..mesh.groups.len())
        .rev()
        .filter(|&g| active[g] && mesh.groups[g].simplified.error != f32::MAX)
    {
        let incoming: Vec<_> = mesh
            .clusters
            .iter()
            .enumerate()
            .filter(|(_, c)| c.refined == Some(GroupId(g as u32)))
            .map(|(i, _)| i)
            .collect();
        assert!(!incoming.is_empty());
        for i in incoming {
            assert!(selected[i], "missing or twice-replaced coarse shard");
            selected[i] = false;
        }
        for i in mesh.groups[g].clusters.range(selected.len()).unwrap() {
            assert!(!selected[i], "duplicate group cover");
            selected[i] = true;
        }
    }
    assert_eq!(
        cut.clusters,
        selected
            .iter()
            .enumerate()
            .filter(|(_, v)| **v)
            .map(|(i, _)| ClusterId(i as u32))
            .collect::<Vec<_>>()
    );
}

#[test]
fn candidate_pressure_refines_highest_error_before_finest_group_id() {
    let mesh = graph(&[
        &[None],
        &[Some(0)],
        &[None],
        &[Some(2)],
        &[Some(1)],
        &[Some(3)],
    ]);
    let errors = [10., 100., 10., 200., f64::INFINITY, f64::INFINITY];
    let all = vec![true; mesh.clusters.len()];
    for candidates in [1, 2] {
        let result = select(
            &mesh,
            &errors,
            1.,
            &all,
            SelectionBudget {
                max_candidates: candidates,
                ..budget()
            },
        )
        .unwrap();
        exact_once(&mesh, &result.cut);
        assert!(
            result.cut.active_groups.contains(&GroupId(3)),
            "highest-error region starved by group ID: {:?}",
            result.cut
        );
        assert!(
            !result.cut.active_groups.contains(&GroupId(0)),
            "fine detail consumed the coarse-region allowance"
        );
        assert_eq!(result.candidates, candidates);
        assert_eq!(
            result.debt.maximum_denied_error,
            if candidates == 1 { 100. } else { 10. }
        );
    }
}

#[test]
fn equal_infinite_errors_balance_coarse_levels_before_fine_detail() {
    let mut mesh = graph(&[
        &[None],
        &[Some(0)],
        &[None],
        &[Some(2)],
        &[Some(1)],
        &[Some(3)],
    ]);
    for (g, depth) in mesh.groups.iter_mut().zip([0, 1, 0, 1, 2, 2]) {
        g.depth = depth;
    }
    let result = select(
        &mesh,
        &[f64::INFINITY; 6],
        1.,
        &vec![true; mesh.clusters.len()],
        SelectionBudget {
            max_candidates: 2,
            ..budget()
        },
    )
    .unwrap();
    exact_once(&mesh, &result.cut);
    assert_eq!(
        result.cut.active_groups,
        [GroupId(1), GroupId(3), GroupId(4), GroupId(5)]
    );
    assert_eq!(result.candidates, 2);
}

#[test]
fn exhaustive_small_dags_availability_capacity_and_upstream_rule() {
    let graphs = [
        graph(&[&[None], &[Some(0)], &[Some(1)]]),
        graph(&[&[None], &[None], &[Some(0), Some(1)], &[Some(0)]]),
        graph(&[&[None], &[None], &[Some(0)]]),
        graph(&[&[None], &[Some(0)], &[Some(0)], &[Some(1), Some(2)]]),
    ];
    let mut cases = 0;
    for mesh in graphs {
        let n = mesh.groups.len();
        let pc = mesh.clusters.len();
        let all = vec![true; pc];
        let roots = select(
            &mesh,
            &mesh
                .groups
                .iter()
                .map(|g| {
                    if g.simplified.error == f32::MAX {
                        f64::INFINITY
                    } else {
                        0.
                    }
                })
                .collect::<Vec<_>>(),
            1.,
            &all,
            budget(),
        )
        .unwrap();
        let minimum = roots.cut.clusters.len();
        let pinned = roots.cut.pages.len();
        for mask in 0..1usize << n {
            let active: Vec<_> = (0..n).map(|g| mask & (1 << g) != 0).collect();
            if !legal(&mesh, &active) {
                continue;
            }
            let errors: Vec<_> = active.iter().map(|&a| if a { 2. } else { 0. }).collect();
            let expected = reference(&mesh, &errors, 1.);
            let unlimited = select(&mesh, &errors, 1., &all, budget()).unwrap();
            assert_eq!(unlimited.cut.clusters, expected);
            exact_once(&mesh, &unlimited.cut);
            for availability in 0..1usize << pc {
                let available: Vec<_> = (0..pc).map(|p| availability & (1 << p) != 0).collect();
                for output in minimum.saturating_sub(1)..=pc + 1 {
                    for capacity in pinned.saturating_sub(1)..=pc + 1 {
                        for candidates in 0..=n + 1 {
                            let b = SelectionBudget {
                                geometry_bytes: capacity * PAGE_BYTES,
                                max_clusters: output,
                                max_candidates: candidates,
                                ..budget()
                            };
                            let result = select(&mesh, &errors, 1., &available, b);
                            cases += 1;
                            if output < minimum
                                || capacity < pinned
                                || roots.cut.pages.iter().any(|p| !available[p.index()])
                            {
                                assert!(result.is_err());
                                continue;
                            }
                            let s = result.unwrap();
                            exact_once(&mesh, &s.cut);
                            assert!(s.cut.clusters.len() <= output);
                            assert!(s.candidates <= candidates.min(n));
                            assert!(s.cut.pages.iter().all(|p| available[p.index()]));
                            assert_eq!(
                                s.demand.requested_pages,
                                s.demand
                                    .required_pages
                                    .iter()
                                    .copied()
                                    .filter(|p| !available[p.index()])
                                    .collect::<Vec<_>>()
                            );
                            let mut protected = s.cut.pages.clone();
                            protected.extend(&s.demand.required_pages);
                            protected.sort_unstable();
                            protected.dedup();
                            assert!(protected.len() <= capacity);
                            if !s.demand.requested_pages.is_empty() {
                                // Publishing exactly demanded pages must permit a complete legal transition.
                                let mut published = available.clone();
                                for p in &s.demand.requested_pages {
                                    published[p.index()] = true;
                                }
                                let next = select(&mesh, &errors, 1., &published, b).unwrap();
                                exact_once(&mesh, &next.cut);
                                assert!(next.cut.active_groups.len() > s.cut.active_groups.len());
                            }
                        }
                    }
                }
            }
        }
    }
    println!("exhaustive DAG availability/capacity cases: {cases}");
    assert!(cases > 10_000);
}

#[test]
fn atomic_closure_does_not_require_unrequested_intermediate_pages() {
    let m = graph(&[&[None], &[Some(0)], &[Some(1)]]);
    let s = select(
        &m,
        &[2., 3., f64::INFINITY],
        1.,
        &[true, false, true],
        budget(),
    )
    .unwrap();
    assert_eq!(s.cut.clusters, vec![ClusterId(0)]);
    assert!(s.demand.requested_pages.is_empty());
    let missing = select(
        &m,
        &[2., 3., f64::INFINITY],
        1.,
        &[false, false, true],
        budget(),
    )
    .unwrap();
    assert_eq!(missing.demand.requested_pages, vec![PageId(0)]);
    assert_eq!(missing.demand.required_pages, vec![PageId(0), PageId(2)]);
}

#[test]
fn virtual_codec_rejects_bad_counts_cycles_references_and_domains() {
    let m = graph(&[&[None], &[Some(0)]]);
    let p = Package {
        pages: (0..2)
            .map(|_| Page {
                vertices: 3,
                indices: 3,
                payload_bytes: 156,
                sha256: [0; 32],
            })
            .collect(),
        meshes: vec![m],
        placements: vec![],
        resources: Asset {
            materials: vec![Material::default()],
            ..Asset::default()
        },
    };
    let l = VirtualLimits::default();
    let bytes = encode(&p, l).unwrap();
    assert_eq!(decode(&bytes, l).unwrap(), p);
    for end in 0..bytes.len() {
        assert!(decode(&bytes[..end], l).is_err());
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(decode(&extra, l).is_err());
    let mut count = bytes.clone();
    count[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode(&count, l).is_err());
    let mut geometry_resources = bytes.clone();
    geometry_resources[32..36].copy_from_slice(&1u32.to_le_bytes());
    assert_eq!(
        decode(&geometry_resources, l).unwrap_err().0,
        "virtual resources contain geometry or are truncated"
    );
    let mut bad = p.clone();
    bad.meshes[0].clusters[1].refined = Some(GroupId(1));
    assert!(bad.validate(l).is_err());
    let mut bad = p.clone();
    bad.meshes[0].clusters[0].page = PageId(99);
    assert!(bad.validate(l).is_err());
    let mut bad = p.clone();
    bad.meshes[0].groups[0].simplified.radius = f32::NAN;
    assert!(bad.validate(l).is_err());
    let mut bad = p.clone();
    bad.meshes[0].clusters[0].indices.count = u32::MAX;
    assert!(bad.validate(l).is_err());
    assert!(
        decode(
            &bytes,
            VirtualLimits {
                max_metadata_bytes: 1,
                ..l
            }
        )
        .is_err()
    );
    assert!(select(&p.meshes[0], &[2., 1.], 0., &[true; 2], budget()).is_err());
    assert!(
        select(
            &p.meshes[0],
            &[f64::NAN, f64::INFINITY],
            1.,
            &[true; 2],
            budget()
        )
        .is_err()
    );
}
#[test]
fn published_cut_refinement_preserves_closed_coverage_and_transition_budget() {
    let graphs = [
        graph(&[&[None], &[Some(0)], &[Some(1)]]),
        graph(&[&[None], &[None], &[Some(0), Some(1)], &[Some(0)]]),
        graph(&[&[None], &[Some(0)], &[Some(0)], &[Some(1), Some(2)]]),
    ];
    let mut checked = 0;
    for mesh in graphs {
        let n = mesh.groups.len();
        let pc = mesh.clusters.len();
        for mask in 0..1usize << n {
            let active: Vec<_> = (0..n).map(|g| mask & (1 << g) != 0).collect();
            if !legal(&mesh, &active) {
                continue;
            }
            let ids: Vec<_> = mesh
                .clusters
                .iter()
                .enumerate()
                .filter(|(_, c)| {
                    active[c.owner.index()] && c.refined.is_none_or(|r| !active[r.index()])
                })
                .map(|(i, _)| ClusterId(i as u32))
                .collect();
            let mut pages: Vec<_> = ids.iter().map(|c| mesh.clusters[c.index()].page).collect();
            pages.sort_unstable();
            pages.dedup();
            let initial = Cut {
                active_groups: active
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| **a)
                    .map(|(g, _)| GroupId(g as u32))
                    .collect(),
                clusters: ids,
                pages,
            };
            let errors = vec![f64::INFINITY; n];
            for availability in 0..1usize << pc {
                let available: Vec<_> = (0..pc).map(|p| availability & (1 << p) != 0).collect();
                for capacity in 1..=pc + 1 {
                    for output in 1..=pc + 1 {
                        for candidates in 0..=n {
                            let b = SelectionBudget {
                                geometry_bytes: capacity * PAGE_BYTES,
                                max_candidates: candidates,
                                max_clusters: output,
                                ..budget()
                            };
                            if let Ok(next) = select_from_cut(
                                &mesh,
                                &errors,
                                1.,
                                &available,
                                b,
                                &initial.pages,
                                &initial,
                            ) {
                                checked += 1;
                                exact_once(&mesh, &next.cut);
                                assert!(
                                    initial
                                        .active_groups
                                        .iter()
                                        .all(|g| next.cut.active_groups.contains(g))
                                );
                                assert!(next.cut.pages.iter().all(|p| available[p.index()]));
                                assert!(next.cut.clusters.len() <= output);
                                let mut protected = initial.pages.clone();
                                protected.extend(&next.cut.pages);
                                protected.extend(&next.demand.required_pages);
                                protected.sort_unstable();
                                protected.dedup();
                                assert!(protected.len() * PAGE_BYTES <= b.geometry_bytes);
                            }
                        }
                    }
                }
            }
            let mut invalid = initial.clone();
            invalid.active_groups.clear();
            assert!(
                select_from_cut(&mesh, &errors, 1., &vec![true; pc], budget(), &[], &invalid)
                    .is_err()
            );
            let mut invalid = initial.clone();
            invalid.clusters.clear();
            assert!(
                select_from_cut(&mesh, &errors, 1., &vec![true; pc], budget(), &[], &invalid)
                    .is_err()
            );
        }
    }
    assert!(checked > 1000);
    println!("current_cut_checked={checked}");
}

#[cfg(feature = "selection-oracle")]
fn assert_equivalent(
    actual: &renderlab_asset::Result<Selection>,
    old: renderlab_asset::Result<Selection>,
) {
    assert_eq!(actual.is_ok(), old.is_ok(), "new={actual:?} old={old:?}");
    if let (Ok(a), Ok(b)) = (actual, old) {
        assert_eq!(a.cut, b.cut);
        assert_eq!(a.demand, b.demand);
        assert_eq!(a.debt, b.debt);
        assert_eq!(a.candidates, b.candidates);
    }
}
#[cfg(feature = "selection-oracle")]
fn select(
    mesh: &VirtualMesh,
    errors: &[f64],
    threshold: f64,
    available: &[bool],
    budget: SelectionBudget,
) -> renderlab_asset::Result<Selection> {
    let actual = renderlab_asset::virtual_mesh::select(mesh, errors, threshold, available, budget);
    assert_equivalent(
        &actual,
        selection_oracle::select(mesh, errors, threshold, available, budget),
    );
    actual
}
#[cfg(feature = "selection-oracle")]
fn select_from_cut(
    mesh: &VirtualMesh,
    errors: &[f64],
    threshold: f64,
    available: &[bool],
    budget: SelectionBudget,
    protected: &[PageId],
    initial: &Cut,
) -> renderlab_asset::Result<Selection> {
    let actual = renderlab_asset::virtual_mesh::select_from_cut(
        mesh, errors, threshold, available, budget, protected, initial,
    );
    assert_equivalent(
        &actual,
        selection_oracle::select_from_cut(
            mesh, errors, threshold, available, budget, protected, initial,
        ),
    );
    actual
}

#[test]
#[cfg(feature = "selection-oracle")]
fn seeded_shared_page_dags_match_preserved_scan_policy() {
    let mut seed = 0xfbe75c1a9b234d01u64;
    let mut draw = |limit: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed as usize) % limit
    };
    for _ in 0..6000 {
        let n = 2 + draw(23);
        let members: Vec<Vec<_>> = (0..n)
            .map(|g| {
                (0..1 + draw(4))
                    .map(|_| {
                        if g == 0 || draw(4) == 0 {
                            None
                        } else {
                            Some(draw(g) as u32)
                        }
                    })
                    .collect()
            })
            .collect();
        let mut mesh = graph(&members.iter().map(Vec::as_slice).collect::<Vec<_>>());
        let pc = 1 + draw(mesh.clusters.len());
        let mut offsets = vec![0; pc];
        for c in &mut mesh.clusters {
            c.page = PageId(draw(pc) as u32);
            c.indices.offset = offsets[c.page.index()];
            offsets[c.page.index()] += c.indices.count;
        }
        let mut errors = vec![0.; n];
        for g in 0..n {
            errors[g] = if mesh.groups[g].simplified.error == f32::MAX {
                f64::INFINITY
            } else {
                draw(5) as f64
            };
            for c in &mesh.clusters[mesh.groups[g].clusters.range(mesh.clusters.len()).unwrap()] {
                if let Some(r) = c.refined {
                    errors[g] = errors[g].max(errors[r.index()]);
                }
            }
        }
        let root_errors: Vec<_> = mesh
            .groups
            .iter()
            .map(|g| {
                if g.simplified.error == f32::MAX {
                    f64::INFINITY
                } else {
                    0.
                }
            })
            .collect();
        let roots = select(&mesh, &root_errors, 1., &vec![true; pc], budget())
            .unwrap()
            .cut;
        let mut available: Vec<_> = (0..pc).map(|_| draw(3) > 0).collect();
        for p in &roots.pages {
            available[p.index()] = true;
        }
        let protected: Vec<_> = (0..pc)
            .filter(|_| draw(4) == 0)
            .map(|p| PageId(p as u32))
            .collect();
        let b = SelectionBudget {
            geometry_bytes: draw(pc + 2) * PAGE_BYTES,
            max_clusters: draw(mesh.clusters.len() + 2),
            max_candidates: draw(n + 2),
            ..budget()
        };
        let actual = select_with_protected(&mesh, &errors, 1., &available, b, &protected);
        assert_equivalent(
            &actual,
            selection_oracle::select_with_protected(&mesh, &errors, 1., &available, b, &protected),
        );
        if let Ok(a) = actual {
            exact_once(&mesh, &a.cut);
        }
        let initial = select(
            &mesh,
            &errors,
            1.,
            &vec![true; pc],
            SelectionBudget {
                max_candidates: draw(n + 1),
                ..budget()
            },
        )
        .unwrap()
        .cut;
        for p in &initial.pages {
            available[p.index()] = true;
        }
        let next = select_from_cut(&mesh, &errors, 1., &available, b, &protected, &initial);
        if let Ok(a) = next {
            exact_once(&mesh, &a.cut);
        }
    }
    println!("seeded DAGs=6000 root+initial cases=12000 seed=fbe75c1a9b234d01");
}

#[test]
fn selection_charges_page_scratch_before_allocation() {
    let mesh = graph(&[&[None], &[Some(0)]]);
    let available = vec![true; 8192];
    let result = renderlab_asset::virtual_mesh::select(
        &mesh,
        &[2., f64::INFINITY],
        1.,
        &available,
        budget(),
    )
    .unwrap();
    let expected = 32 * mesh.groups.len() + 48 * mesh.clusters.len() + 16 * available.len() + 8;
    assert_eq!(result.scratch_bound_bytes, expected);
    assert!(
        renderlab_asset::virtual_mesh::select(
            &mesh,
            &[2., f64::INFINITY],
            1.,
            &available,
            SelectionBudget {
                max_scratch_bytes: expected - 1,
                ..budget()
            }
        )
        .is_err()
    );
    let bounded = renderlab_asset::virtual_mesh::select(
        &mesh,
        &[2., f64::INFINITY],
        1.,
        &available,
        SelectionBudget {
            max_clusters: 1,
            ..budget()
        },
    )
    .unwrap();
    assert!(bounded.cut.clusters.capacity() <= 1);
    assert!(bounded.demand.required_pages.capacity() <= available.len());
    assert!(bounded.demand.requested_pages.capacity() <= available.len());
    // Duplicate protected IDs do not allocate from the caller's unbounded list length.
    let duplicate = vec![PageId(0); 200_000];
    let result = select_with_protected(
        &mesh,
        &[2., f64::INFINITY],
        1.,
        &available,
        budget(),
        &duplicate,
    )
    .unwrap();
    assert_eq!(result.scratch_bound_bytes, expected);
}
