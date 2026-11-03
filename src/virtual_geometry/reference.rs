use super::*;
use asset::virtual_mesh::{ClusterId, Cut, DebtReason, Demand, GroupId, QualityDebt};
use std::collections::BTreeSet;
/// Deliberately simple reference: fixed-point owner closure and ordered sets. Does not call the
/// production selector. Uses identical policy order/caps so Unit C can compare equal budgets.
pub(super) fn select(
    mesh: &VirtualMesh,
    errors: &[f64],
    threshold: f64,
    available: &[bool],
    budget: SelectionBudget,
    protected: &[PageId],
    initial: Option<&Cut>,
) -> Result<Selection, Error> {
    let scratch = mesh
        .groups
        .len()
        .checked_mul(32)
        .and_then(|n| n.checked_add(mesh.clusters.len().checked_mul(48)?))
        .and_then(|n| n.checked_add(available.len().checked_mul(16)?))
        .and_then(|n| n.checked_add(8))
        .ok_or(Error::InvalidInput("reference scratch overflow"))?;
    if scratch > budget.max_scratch_bytes
        || errors.len() != mesh.groups.len()
        || !threshold.is_finite()
        || threshold < 0.
        || errors.iter().any(|e| e.is_nan() || *e < 0.)
        || protected.iter().any(|p| p.index() >= available.len())
    {
        return Err(Error::InvalidInput("reference domain/scratch capacity"));
    }
    mesh.validate(
        available.len(),
        usize::MAX,
        asset::virtual_mesh::VirtualLimits {
            max_groups: mesh.groups.len(),
            max_clusters: mesh.clusters.len(),
            max_metadata_bytes: budget.max_scratch_bytes,
            ..Default::default()
        },
    )
    .map_err(|e| Error::InvalidInput(e.0))?;
    if mesh
        .terminal_groups()
        .any(|g| errors[g.index()] <= threshold)
        || mesh.clusters.iter().any(|c| {
            c.refined
                .is_some_and(|r| errors[c.owner.index()] < errors[r.index()])
        })
    {
        return Err(Error::InvalidInput("reference projected error domain"));
    }
    let mut active: BTreeSet<_> = mesh.terminal_groups().collect();
    let cut = |a: &BTreeSet<GroupId>| -> Vec<ClusterId> {
        mesh.clusters
            .iter()
            .enumerate()
            .filter(|(_, c)| a.contains(&c.owner) && c.refined.is_none_or(|r| !a.contains(&r)))
            .map(|(i, _)| ClusterId(i as u32))
            .collect()
    };
    let pages = |ids: &[ClusterId]| -> BTreeSet<PageId> {
        ids.iter().map(|c| mesh.clusters[c.index()].page).collect()
    };
    let mut selected = cut(&active);
    let pinned = pages(&selected);
    let other: BTreeSet<_> = protected.iter().copied().collect();
    if selected.len() > budget.max_clusters
        || pinned.union(&other).count() * PAGE_BYTES > budget.geometry_bytes
        || pinned.iter().any(|p| !available[p.index()])
    {
        return Err(Error::InvalidInput(
            "reference minimum cover unavailable/capacity",
        ));
    }
    if let Some(initial) = initial {
        active = initial.active_groups.iter().copied().collect();
        if active.len() != initial.active_groups.len()
            || active.iter().any(|g| g.index() >= mesh.groups.len())
            || !mesh.terminal_groups().all(|g| active.contains(&g))
            || mesh.clusters.iter().any(|c| {
                c.refined
                    .is_some_and(|r| active.contains(&r) && !active.contains(&c.owner))
            })
        {
            return Err(Error::InvalidInput("reference initial active groups"));
        }
        selected = cut(&active);
        let current_pages = pages(&selected);
        let charged: BTreeSet<_> = current_pages
            .union(&pinned)
            .copied()
            .chain(other.iter().copied())
            .collect();
        if selected != initial.clusters
            || current_pages.iter().copied().collect::<Vec<_>>() != initial.pages
            || selected.len() > budget.max_clusters
            || current_pages.iter().any(|p| !available[p.index()])
            || charged.len() * PAGE_BYTES > budget.geometry_bytes
        {
            return Err(Error::InvalidInput("reference initial cut admission"));
        }
    }
    let mut reasons = vec![];
    let mut candidates = 0;
    let mut visits = 0;
    let mut published = active.clone();
    let mut order: Vec<_> = errors.iter().copied().enumerate().collect();
    order.sort_by(|(a, x), (b, y)| {
        y.total_cmp(x)
            .then_with(|| mesh.groups[*b].depth.cmp(&mesh.groups[*a].depth))
            .then_with(|| a.cmp(b))
    });
    let mut closure = |base: &BTreeSet<GroupId>, seed: GroupId| {
        let mut proposal = base.clone();
        proposal.insert(seed);
        loop {
            let owners: Vec<_> = mesh
                .clusters
                .iter()
                .filter(|c| c.refined.is_some_and(|r| proposal.contains(&r)))
                .map(|c| c.owner)
                .collect();
            visits += mesh.clusters.len();
            let before = proposal.len();
            proposal.extend(owners);
            if proposal.len() == before {
                break;
            }
        }
        proposal
    };
    let target = |a: &BTreeSet<GroupId>| -> BTreeSet<PageId> {
        pages(&cut(a)).union(&pinned).copied().collect()
    };
    let charged = |p: &BTreeSet<PageId>, u: &[ClusterId]| -> usize {
        p.iter()
            .copied()
            .chain(pages(u))
            .chain(other.iter().copied())
            .collect::<BTreeSet<_>>()
            .len()
    };
    for (g, error) in order {
        let seed = GroupId(g as u32);
        if published.contains(&seed) || error <= threshold {
            continue;
        }
        if candidates >= budget.max_candidates {
            reasons.push(DebtReason::CandidateCapacity);
            break;
        }
        candidates += 1;
        if !active.contains(&seed) {
            let proposal = closure(&active, seed);
            let next = cut(&proposal);
            let reason = if next.len() > budget.max_clusters {
                Some(DebtReason::OutputCapacity)
            } else if charged(&target(&proposal), &selected) * PAGE_BYTES > budget.geometry_bytes {
                Some(DebtReason::GeometryCapacity)
            } else {
                None
            };
            if let Some(r) = reason {
                if !reasons.contains(&r) {
                    reasons.push(r);
                }
                continue;
            }
            active = proposal;
        }
        let planned_pages = target(&active);
        if planned_pages.iter().all(|p| available[p.index()]) {
            published = active.clone();
            selected = cut(&published);
            continue;
        }
        let proposal = closure(&published, seed);
        assert!(proposal.is_subset(&active));
        let next = cut(&proposal);
        let reason = if next.len() > budget.max_clusters {
            Some(DebtReason::OutputCapacity)
        } else if charged(&planned_pages, &next) * PAGE_BYTES > budget.geometry_bytes {
            Some(DebtReason::GeometryCapacity)
        } else {
            if pages(&next).iter().all(|p| available[p.index()]) {
                published = proposal;
                selected = next;
            }
            None
        };
        if let Some(r) = reason
            && !reasons.contains(&r)
        {
            reasons.push(r);
        }
    }
    let required = target(&active);
    let requested: Vec<_> = required
        .iter()
        .copied()
        .filter(|p| !available[p.index()])
        .collect();
    if !requested.is_empty() {
        reasons.push(DebtReason::MissingPages);
    }
    let denied: Vec<_> = (0..mesh.groups.len())
        .filter(|g| !published.contains(&GroupId(*g as u32)) && errors[*g] > threshold)
        .collect();
    Ok(Selection {
        cut: Cut {
            active_groups: published.into_iter().collect(),
            pages: pages(&selected).into_iter().collect(),
            clusters: selected,
        },
        demand: Demand {
            required_pages: required.into_iter().collect(),
            requested_pages: requested,
        },
        debt: QualityDebt {
            unresolved_groups: denied.len(),
            maximum_denied_error: denied.iter().map(|g| errors[*g]).fold(0., f64::max),
            reasons,
        },
        candidates,
        cluster_visits: visits,
        scratch_bound_bytes: scratch,
    })
}
