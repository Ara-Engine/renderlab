//! Preserved a18a0ed scan selector. Explicit correctness/timing feature only.
use super::*;

fn cluster_cut(mesh: &VirtualMesh, active: &[bool]) -> Vec<ClusterId> {
    mesh.clusters
        .iter()
        .enumerate()
        .filter(|(_, c)| active[c.owner.index()] && c.refined.is_none_or(|r| !active[r.index()]))
        .map(|(i, _)| ClusterId(i as u32))
        .collect()
}
fn pages(mesh: &VirtualMesh, clusters: &[ClusterId]) -> Vec<PageId> {
    let mut out: Vec<_> = clusters
        .iter()
        .map(|i| mesh.clusters[i.index()].page)
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}
fn union(a: &[PageId], b: &[PageId]) -> Vec<PageId> {
    let mut out = a.to_vec();
    out.extend(b);
    out.sort_unstable();
    out.dedup();
    out
}

/// Rebuild from all terminal groups over published availability; never publish half a group.
/// Errors must be nonnegative, finite (or +infinity), and monotone over every owner edge.
/// Projection and conservative transform/near handling belong to the caller, not this scalar API.
pub fn select(
    mesh: &VirtualMesh,
    projected_errors: &[f64],
    threshold: f64,
    available: &[bool],
    budget: SelectionBudget,
) -> Result<Selection> {
    select_with_protected(mesh, projected_errors, threshold, available, budget, &[])
}

/// Same legal selector with a package-global page union already protected by other instances.
/// Protected pages are charged once, including overlap with this instance's transition.
pub fn select_with_protected(
    mesh: &VirtualMesh,
    projected_errors: &[f64],
    threshold: f64,
    available: &[bool],
    budget: SelectionBudget,
    protected: &[PageId],
) -> Result<Selection> {
    select_internal(
        mesh,
        projected_errors,
        threshold,
        available,
        budget,
        protected,
        None,
    )
}

/// Refine a validated currently published legal cut. This avoids requesting obsolete
/// intermediate representations when a streaming runtime retains an already finer cover.
pub fn select_from_cut(
    mesh: &VirtualMesh,
    projected_errors: &[f64],
    threshold: f64,
    available: &[bool],
    budget: SelectionBudget,
    protected: &[PageId],
    initial: &Cut,
) -> Result<Selection> {
    select_internal(
        mesh,
        projected_errors,
        threshold,
        available,
        budget,
        protected,
        Some(initial),
    )
}
fn select_internal(
    mesh: &VirtualMesh,
    projected_errors: &[f64],
    threshold: f64,
    available: &[bool],
    budget: SelectionBudget,
    protected: &[PageId],
    initial: Option<&Cut>,
) -> Result<Selection> {
    if protected.iter().any(|p| p.index() >= available.len()) {
        return Err(Error("protected page outside package"));
    }
    let n = mesh.groups.len();
    let c = mesh.clusters.len();
    let scratch = product(n, 32)?
        .checked_add(product(c, 48)?)
        .ok_or(Error("selection scratch overflow"))?;
    cap(
        scratch,
        budget.max_scratch_bytes,
        "selection scratch capacity",
    )?;
    mesh.validate(
        available.len(),
        usize::MAX,
        VirtualLimits {
            max_groups: n,
            max_clusters: c,
            max_metadata_bytes: budget.max_scratch_bytes,
            ..VirtualLimits::default()
        },
    )?;
    if projected_errors.len() != n
        || !threshold.is_finite()
        || threshold < 0.
        || projected_errors.iter().any(|e| e.is_nan() || *e < 0.)
    {
        return Err(Error("invalid projected error domain"));
    }
    for group in mesh.terminal_groups() {
        if projected_errors[group.index()] <= threshold {
            return Err(Error("terminal error must exceed threshold"));
        }
    }
    for cluster in &mesh.clusters {
        if let Some(r) = cluster.refined
            && projected_errors[cluster.owner.index()] < projected_errors[r.index()]
        {
            return Err(Error("nonmonotone projected group errors"));
        }
    }
    let mut active = vec![false; n];
    for g in mesh.terminal_groups() {
        active[g.index()] = true;
    }
    let mut selected = cluster_cut(mesh, &active);
    let pinned = pages(mesh, &selected);
    if selected.len() > budget.max_clusters
        || product(union(&pinned, protected).len(), PAGE_BYTES)? > budget.geometry_bytes
    {
        return Err(Error("capacity below minimum cover"));
    }
    if pinned.iter().any(|p| !available[p.index()]) {
        return Err(Error("minimum cover pages unavailable"));
    }
    if let Some(initial) = initial {
        active.fill(false);
        for g in &initial.active_groups {
            if g.index() >= n || active[g.index()] {
                return Err(Error("invalid initial active group"));
            }
            active[g.index()] = true;
        }
        if mesh.terminal_groups().any(|g| !active[g.index()])
            || mesh.clusters.iter().any(|c| {
                c.refined
                    .is_some_and(|r| active[r.index()] && !active[c.owner.index()])
            })
        {
            return Err(Error("initial cut is not a closed minimum cover"));
        }
        selected = cluster_cut(mesh, &active);
        let current_pages = pages(mesh, &selected);
        if selected != initial.clusters
            || current_pages != initial.pages
            || selected.len() > budget.max_clusters
            || current_pages.iter().any(|p| !available[p.index()])
            || product(
                union(&union(&pinned, &current_pages), protected).len(),
                PAGE_BYTES,
            )? > budget.geometry_bytes
        {
            return Err(Error("initial cut ranges/availability/capacity"));
        }
    }
    let mut required = union(&pinned, &pages(mesh, &selected));
    let mut requested = vec![];
    let mut reasons = vec![];
    let mut candidates = 0;
    let mut visits = c;
    let mut published = active.clone();
    // Incoming owners are represented by clusters. Topological propagation computes
    // the complete closure before looking at pages; intermediate representations are never required.
    let mut order: Vec<_> = (0..n).collect();
    // Near-plane errors can tie at infinity. Coarser levels must share detail
    // before a low-ID region spends the pool on fine geometry.
    order.sort_unstable_by(|&a, &b| {
        projected_errors[b]
            .total_cmp(&projected_errors[a])
            .then_with(|| mesh.groups[b].depth.cmp(&mesh.groups[a].depth))
            .then_with(|| a.cmp(&b))
    });
    for seed in order {
        if active[seed] || projected_errors[seed] <= threshold {
            continue;
        }
        if candidates >= budget.max_candidates.min(n) {
            reasons.push(DebtReason::CandidateCapacity);
            break;
        }
        candidates += 1;
        let mut proposal = active.clone();
        proposal[seed] = true;
        for cluster in &mesh.clusters {
            if cluster.refined.is_some_and(|r| proposal[r.index()]) {
                proposal[cluster.owner.index()] = true;
            }
        }
        // Clusters are in increasing owner order, so newly marked owners propagate in this pass.
        let replacement = cluster_cut(mesh, &proposal);
        visits = visits.saturating_add(c.saturating_mul(2));
        if replacement.len() > budget.max_clusters.min(c) {
            if !reasons.contains(&DebtReason::OutputCapacity) {
                reasons.push(DebtReason::OutputCapacity);
            }
            continue;
        }
        let target = union(&pinned, &pages(mesh, &replacement));
        let charged = union(&union(&target, &pages(mesh, &selected)), protected);
        if product(charged.len(), PAGE_BYTES)? > budget.geometry_bytes {
            if !reasons.contains(&DebtReason::GeometryCapacity) {
                reasons.push(DebtReason::GeometryCapacity);
            }
            continue;
        }
        let missing: Vec<_> = target
            .iter()
            .copied()
            .filter(|p| !available[p.index()])
            .collect();
        if !missing.is_empty() {
            required = target;
            requested = missing;
            active = proposal;
            // Continue the bounded plan so a finer final cover can eliminate
            // missing intermediate pages. Only available complete cuts publish.
            continue;
        }
        active = proposal;
        published.clone_from(&active);
        selected = replacement;
        required = target;
        requested.clear();
    }
    if !requested.is_empty() {
        reasons.push(DebtReason::MissingPages);
    }
    let denied: Vec<_> = (0..n)
        .filter(|&g| !published[g] && projected_errors[g] > threshold)
        .collect();
    let debt = QualityDebt {
        unresolved_groups: denied.len(),
        maximum_denied_error: denied
            .iter()
            .map(|&g| projected_errors[g])
            .fold(0., f64::max),
        reasons,
    };
    Ok(Selection {
        cut: Cut {
            active_groups: published
                .iter()
                .enumerate()
                .filter(|(_, v)| **v)
                .map(|(g, _)| GroupId(g as u32))
                .collect(),
            pages: pages(mesh, &selected),
            clusters: selected,
        },
        demand: Demand {
            required_pages: required,
            requested_pages: requested,
        },
        debt,
        candidates,
        cluster_visits: visits,
        scratch_bound_bytes: scratch,
    })
}
