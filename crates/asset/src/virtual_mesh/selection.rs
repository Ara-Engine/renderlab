use super::*;

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
    pub cluster_visits: usize,
    pub scratch_bound_bytes: usize,
}

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
        || product(pinned.len(), PAGE_BYTES)? > budget.geometry_bytes
    {
        return Err(Error("capacity below minimum cover"));
    }
    if pinned.iter().any(|p| !available[p.index()]) {
        return Err(Error("minimum cover pages unavailable"));
    }
    let mut required = pinned.clone();
    let mut requested = vec![];
    let mut reasons = vec![];
    let mut candidates = 0;
    let mut visits = c;
    // Incoming owners are represented by clusters. Topological propagation computes
    // the complete closure before looking at pages; intermediate representations are never required.
    for seed in 0..n {
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
        let charged = union(&target, &pages(mesh, &selected));
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
            reasons.push(DebtReason::MissingPages);
            break;
        }
        active = proposal;
        selected = replacement;
        required = target;
    }
    let denied: Vec<_> = (0..n)
        .filter(|&g| !active[g] && projected_errors[g] > threshold)
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
            active_groups: active
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
