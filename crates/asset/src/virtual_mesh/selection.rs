use super::*;
mod incremental;
#[cfg(feature = "selection-oracle")]
pub mod oracle;

#[derive(Clone, Copy, Debug)]
pub struct SelectionBudget {
    /// Logical immutable page storage, including pinned roots and both sides of a transition.
    pub geometry_bytes: usize,
    pub max_candidates: usize,
    pub max_clusters: usize,
    pub max_scratch_bytes: usize,
}
impl Default for SelectionBudget {
    fn default() -> Self {
        Self {
            geometry_bytes: 8 << 20,
            max_candidates: 1024,
            max_clusters: 65536,
            max_scratch_bytes: 8 << 20,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cut {
    /// Active means group geometry can replace every incoming coarse shard atomically.
    pub active_groups: Vec<GroupId>,
    pub clusters: Vec<ClusterId>,
    pub pages: Vec<PageId>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Demand {
    /// Complete final cover for the next atomic transition, plus pinned fallback pages.
    pub required_pages: Vec<PageId>,
    pub requested_pages: Vec<PageId>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebtReason {
    MissingPages,
    CandidateCapacity,
    OutputCapacity,
    GeometryCapacity,
}
#[derive(Clone, Debug, PartialEq)]
pub struct QualityDebt {
    pub unresolved_groups: usize,
    pub maximum_denied_error: f64,
    pub reasons: Vec<DebtReason>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Selection {
    pub cut: Cut,
    pub demand: Demand,
    pub debt: QualityDebt,
    pub candidates: usize,
    /// Cluster predicate/link/shard touches. Excludes validation, scalar/group/page scans and sorting.
    /// The preserved scan oracle counts only whole cut/closure scans, a subset of these touches.
    pub cluster_visits: usize,
    pub scratch_bound_bytes: usize,
}

fn cluster_cut(mesh: &VirtualMesh, active: &[bool]) -> Vec<ClusterId> {
    let mut out = Vec::with_capacity(mesh.clusters.len());
    out.extend(
        mesh.clusters
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                active[c.owner.index()] && c.refined.is_none_or(|r| !active[r.index()])
            })
            .map(|(i, _)| ClusterId(i as u32)),
    );
    out
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
fn charged_pages(page_count: usize, sets: &[&[PageId]]) -> usize {
    let mut seen = vec![false; page_count];
    for page in sets.iter().flat_map(|s| s.iter()) {
        seen[page.index()] = true;
    }
    seen.into_iter().filter(|v| *v).count()
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
    #[cfg(feature = "selection-profile")]
    let profile = std::time::Instant::now();
    if protected.iter().any(|p| p.index() >= available.len()) {
        return Err(Error("protected page outside package"));
    }
    let n = mesh.groups.len();
    let c = mesh.clusters.len();
    let scratch = product(n, 32)?
        .checked_add(product(c, 48)?)
        .and_then(|v| v.checked_add(available.len().checked_mul(16)?))
        .and_then(|v| v.checked_add(8))
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
    #[cfg(feature = "selection-profile")]
    let domain_ns = profile.elapsed().as_nanos();
    let mut active = vec![false; n];
    for g in mesh.terminal_groups() {
        active[g.index()] = true;
    }
    let mut selected = cluster_cut(mesh, &active);
    let pinned = pages(mesh, &selected);
    if selected.len() > budget.max_clusters
        || product(
            charged_pages(available.len(), &[&pinned, protected]),
            PAGE_BYTES,
        )? > budget.geometry_bytes
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
                charged_pages(available.len(), &[&pinned, &current_pages, protected]),
                PAGE_BYTES,
            )? > budget.geometry_bytes
        {
            return Err(Error("initial cut ranges/availability/capacity"));
        }
    }
    drop(selected);
    #[cfg(feature = "selection-profile")]
    let admitted_ns = profile.elapsed().as_nanos();
    let mut state = incremental::State::new(mesh, available, active, &pinned, protected);
    #[cfg(feature = "selection-profile")]
    let indexed_ns = profile.elapsed().as_nanos();
    let mut reasons = vec![];
    let mut candidates = 0;
    let mut order: Vec<u32> = (0..n as u32).collect();
    order.sort_unstable_by(|&a, &b| {
        projected_errors[b as usize]
            .total_cmp(&projected_errors[a as usize])
            .then_with(|| {
                mesh.groups[b as usize]
                    .depth
                    .cmp(&mesh.groups[a as usize].depth)
            })
            .then_with(|| a.cmp(&b))
    });
    #[cfg(feature = "selection-profile")]
    let sorted_ns = profile.elapsed().as_nanos();
    use incremental::Track;
    for seed in order {
        let seed = seed as usize;
        if state.published.active[seed] || projected_errors[seed] <= threshold {
            continue;
        }
        if candidates >= budget.max_candidates.min(n) {
            reasons.push(DebtReason::CandidateCapacity);
            break;
        }
        candidates += 1;
        if !state.planned.active[seed] {
            state.propose(seed, Track::Plan);
            let denial = if state.planned.clusters > budget.max_clusters.min(c) {
                Some(DebtReason::OutputCapacity)
            } else if product(state.charged, PAGE_BYTES)? > budget.geometry_bytes {
                Some(DebtReason::GeometryCapacity)
            } else {
                None
            };
            if let Some(reason) = denial {
                if !reasons.contains(&reason) {
                    reasons.push(reason);
                }
                state.rollback(Track::Plan);
                continue;
            }
            state.accept(Track::Plan);
        }
        if state.planned.missing == 0 {
            state.publish_plan();
            continue;
        }
        // Publication is a separate transaction inside the admitted plan. Its complete cut
        // must be resident; it cannot introduce IO demand or bypass transition accounting.
        state.propose(seed, Track::Publication);
        let denial = if state.published.clusters > budget.max_clusters.min(c) {
            Some(DebtReason::OutputCapacity)
        } else if product(state.charged, PAGE_BYTES)? > budget.geometry_bytes {
            Some(DebtReason::GeometryCapacity)
        } else {
            None
        };
        if let Some(reason) = denial
            && !reasons.contains(&reason)
        {
            reasons.push(reason);
        }
        if denial.is_some() || state.published.missing != 0 {
            state.rollback(Track::Publication);
        } else {
            state.accept(Track::Publication);
        }
    }
    #[cfg(feature = "selection-profile")]
    let proposed_ns = profile.elapsed().as_nanos();
    let mut required = Vec::with_capacity(c);
    required.extend(
        state
            .planned
            .refs
            .iter()
            .enumerate()
            .filter(|(p, count)| **count > 0 || pinned.binary_search(&PageId(*p as u32)).is_ok())
            .map(|(p, _)| PageId(p as u32)),
    );
    let mut requested = Vec::with_capacity(c);
    requested.extend(required.iter().copied().filter(|p| !available[p.index()]));
    let selected = cluster_cut(mesh, &state.published.active);
    let published = &state.published.active;
    // Count cluster predicate/link/shard touches, excluding the unchanged boundary validator,
    // scalar error checks, group/page scans and sorting. The scan oracle counts only cut/closure
    // scans, a subset of these operations, so an old/new ratio is a conservative work reduction.
    let visits = state.visits + c * if initial.is_some() { 4 } else { 2 };
    if !requested.is_empty() {
        reasons.push(DebtReason::MissingPages);
    }
    let mut unresolved_groups = 0;
    let mut maximum_denied_error: f64 = 0.;
    for (g, &error) in projected_errors.iter().enumerate() {
        if !published[g] && error > threshold {
            unresolved_groups += 1;
            maximum_denied_error = maximum_denied_error.max(error);
        }
    }
    let debt = QualityDebt {
        unresolved_groups,
        maximum_denied_error,
        reasons,
    };
    let mut active_groups = Vec::with_capacity(n);
    active_groups.extend(
        published
            .iter()
            .enumerate()
            .filter(|(_, v)| **v)
            .map(|(g, _)| GroupId(g as u32)),
    );
    let selection = Selection {
        cut: Cut {
            active_groups,
            pages: pages(mesh, &selected),
            clusters: selected.into_boxed_slice().into_vec(),
        },
        demand: Demand {
            required_pages: required.into_boxed_slice().into_vec(),
            requested_pages: requested.into_boxed_slice().into_vec(),
        },
        debt,
        candidates,
        cluster_visits: visits,
        scratch_bound_bytes: scratch,
    };
    #[cfg(feature = "selection-profile")]
    eprintln!(
        "selection_profile initial={} candidates={} domain_ns={} admission_ns={} index_ns={} sort_ns={} proposals_ns={} emit_ns={}",
        initial.is_some(),
        candidates,
        domain_ns,
        admitted_ns - domain_ns,
        indexed_ns - admitted_ns,
        sorted_ns - indexed_ns,
        proposed_ns - sorted_ns,
        profile.elapsed().as_nanos() - proposed_ns
    );
    Ok(selection)
}
