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
    let mut required: BTreeSet<_> = pinned.union(&pages(&selected)).copied().collect();
    let mut requested = vec![];
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
    for (g, error) in order {
        if active.contains(&GroupId(g as u32)) || error <= threshold {
            continue;
        }
        if candidates >= budget.max_candidates {
            reasons.push(DebtReason::CandidateCapacity);
            break;
        }
        candidates += 1;
        let mut proposal = active.clone();
        proposal.insert(GroupId(g as u32));
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
        let next = cut(&proposal);
        visits += mesh.clusters.len();
        let reason = if next.len() > budget.max_clusters {
            Some(DebtReason::OutputCapacity)
        } else {
            let target: BTreeSet<_> = pages(&next).union(&pinned).copied().collect();
            let charged: BTreeSet<_> = target
                .union(&pages(&selected))
                .copied()
                .chain(other.iter().copied())
                .collect();
            if charged.len() * PAGE_BYTES > budget.geometry_bytes {
                Some(DebtReason::GeometryCapacity)
            } else {
                let missing: Vec<_> = target
                    .iter()
                    .copied()
                    .filter(|p| !available[p.index()])
                    .collect();
                required = target;
                if missing.is_empty() {
                    active = proposal;
                    published = active.clone();
                    selected = next;
                    requested.clear();
                    None
                } else {
                    requested = missing;
                    active = proposal;
                    None
                }
            }
        };
        if let Some(r) = reason
            && !reasons.contains(&r)
        {
            reasons.push(r);
        }
    }
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
        scratch_bound_bytes: mesh.groups.len() * 32
            + mesh.clusters.len() * 48
            + available.len() * 16
            + 8,
    })
}
