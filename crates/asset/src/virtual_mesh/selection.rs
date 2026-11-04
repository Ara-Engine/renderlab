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
    /// A denied transaction stops touching additions once its capacity denial is certain.
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

/// A package validated once for repeated selection, owning each mesh's incoming-link index.
/// Only shared access exists, so neither the metadata nor its index can change afterwards.
///
/// ```compile_fail
/// fn mutate(prepared: &mut renderlab_asset::virtual_mesh::PreparedPackage) {
///     prepared.meshes.clear();
/// }
/// ```
#[derive(Debug)]
pub struct PreparedPackage {
    package: Package,
    incoming: Vec<incremental::Incoming>,
}
impl PreparedPackage {
    /// Applies `Package::validate`, then charges the retained indices and one build cursor
    /// together with the package's element metadata against `max_metadata_bytes` before
    /// allocating them. Validation scratch is released before the indices are built.
    pub fn new(package: Package, limits: VirtualLimits) -> Result<Self> {
        let counts = package.validate(limits)?;
        let mut retained = product(
            package.meshes.len(),
            std::mem::size_of::<incremental::Incoming>(),
        )?;
        let mut cursor = 0;
        for mesh in &package.meshes {
            let (index, build) = incremental::Incoming::charge(mesh)?;
            add(&mut retained, index)?;
            cursor = cursor.max(build);
        }
        let mut peak = counts.metadata_host_bytes;
        add(&mut peak, retained)?;
        add(&mut peak, cursor)?;
        cap(
            peak,
            limits.max_metadata_bytes,
            "prepared selection index budget",
        )?;
        let incoming = package
            .meshes
            .iter()
            .map(incremental::Incoming::new)
            .collect();
        Ok(Self { package, incoming })
    }
    pub fn mesh(&self, mesh: usize) -> Option<PreparedMesh<'_>> {
        Some(PreparedMesh {
            mesh: self.package.meshes.get(mesh)?,
            incoming: &self.incoming[mesh],
            pages: self.package.pages.len(),
        })
    }
    /// Actual retained index capacity, excluding the package's own metadata.
    pub fn index_bytes(&self) -> usize {
        self.incoming.capacity() * std::mem::size_of::<incremental::Incoming>()
            + self
                .incoming
                .iter()
                .map(incremental::Incoming::bytes)
                .sum::<usize>()
    }
}
impl std::ops::Deref for PreparedPackage {
    type Target = Package;
    fn deref(&self) -> &Package {
        &self.package
    }
}

/// One mesh of a `PreparedPackage`. Selection through it skips structural validation and the
/// per-call incoming index; every per-view input is still checked.
#[derive(Clone, Copy, Debug)]
pub struct PreparedMesh<'a> {
    mesh: &'a VirtualMesh,
    incoming: &'a incremental::Incoming,
    pages: usize,
}
impl<'a> PreparedMesh<'a> {
    pub fn mesh(self) -> &'a VirtualMesh {
        self.mesh
    }
    /// The policy and outputs of `select_with_protected` (`initial` = None) or
    /// `select_from_cut`. `available` must have exactly one entry per package page.
    pub fn select(
        self,
        projected_errors: &[f64],
        threshold: f64,
        available: &[bool],
        budget: SelectionBudget,
        protected: &[PageId],
        initial: Option<&Cut>,
    ) -> Result<Selection> {
        select_internal(
            Metadata::Prepared(self),
            projected_errors,
            threshold,
            available,
            budget,
            protected,
            initial,
        )
    }
}

#[derive(Clone, Copy)]
enum Metadata<'a> {
    /// Arbitrary caller metadata: validated and indexed for this call only.
    Checked(&'a VirtualMesh),
    Prepared(PreparedMesh<'a>),
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
        Metadata::Checked(mesh),
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
        Metadata::Checked(mesh),
        projected_errors,
        threshold,
        available,
        budget,
        protected,
        Some(initial),
    )
}
fn select_internal(
    metadata: Metadata<'_>,
    projected_errors: &[f64],
    threshold: f64,
    available: &[bool],
    budget: SelectionBudget,
    protected: &[PageId],
    initial: Option<&Cut>,
) -> Result<Selection> {
    #[cfg(feature = "selection-profile")]
    let profile = std::time::Instant::now();
    let mesh = match metadata {
        Metadata::Checked(mesh) => mesh,
        Metadata::Prepared(prepared) => prepared.mesh,
    };
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
    match metadata {
        Metadata::Checked(_) => mesh.validate(
            available.len(),
            usize::MAX,
            VirtualLimits {
                max_groups: n,
                max_clusters: c,
                max_metadata_bytes: budget.max_scratch_bytes,
                ..VirtualLimits::default()
            },
        )?,
        Metadata::Prepared(prepared) if available.len() != prepared.pages => {
            return Err(Error(
                "availability must cover exactly the prepared package pages",
            ));
        }
        Metadata::Prepared(_) => {}
    }
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
    let checked;
    let (incoming, index_visits) = match metadata {
        Metadata::Prepared(prepared) => (prepared.incoming, 0),
        Metadata::Checked(_) => {
            checked = incremental::Incoming::new(mesh);
            (&checked, 2 * c)
        }
    };
    let mut state = incremental::State::new(mesh, incoming, available, active, &pinned, protected);
    #[cfg(feature = "selection-profile")]
    let indexed_ns = profile.elapsed().as_nanos();
    let mut reasons = vec![];
    let mut candidates = 0;
    // Published groups never deactivate below, and groups within the threshold never seed,
    // so only the remaining groups need the (total) candidate order.
    let mut order: Vec<u32> = (0..n as u32)
        .filter(|&g| {
            !state.published.active[g as usize] && projected_errors[g as usize] > threshold
        })
        .collect();
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
    let capacity = incremental::Capacity {
        clusters: budget.max_clusters.min(c),
        geometry_bytes: budget.geometry_bytes,
    };
    #[cfg(feature = "selection-profile")]
    let mut attribution = Attribution::new(n);
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
            #[cfg(feature = "selection-profile")]
            let (start, visits) = (std::time::Instant::now(), state.visits);
            let denial = state.propose(seed, Track::Plan, capacity)?;
            #[cfg(feature = "selection-profile")]
            attribution.plan.record(&state, visits, denial.is_some());
            if let Some(reason) = denial {
                if !reasons.contains(&reason) {
                    reasons.push(reason);
                }
                state.rollback(Track::Plan);
                #[cfg(feature = "selection-profile")]
                {
                    attribution.plan.ns += start.elapsed().as_nanos();
                }
                continue;
            }
            state.accept(Track::Plan);
            #[cfg(feature = "selection-profile")]
            {
                attribution.plan.ns += start.elapsed().as_nanos();
            }
        }
        if state.planned.missing == 0 {
            #[cfg(feature = "selection-profile")]
            let (start, visits) = (std::time::Instant::now(), state.visits);
            state.publish_plan()?;
            #[cfg(feature = "selection-profile")]
            {
                attribution.sync.record(&state, visits, false);
                attribution.sync.ns += start.elapsed().as_nanos();
            }
            continue;
        }
        if state.publication_blocked(seed, capacity) {
            #[cfg(feature = "selection-profile")]
            {
                attribution.blocked += 1;
            }
            continue;
        }
        // Publication is a separate transaction inside the admitted plan. Its complete cut
        // must be resident; it cannot introduce IO demand or bypass transition accounting.
        #[cfg(feature = "selection-profile")]
        let (start, visits, settled) = (
            std::time::Instant::now(),
            state.visits,
            reasons.contains(&DebtReason::OutputCapacity)
                && reasons.contains(&DebtReason::GeometryCapacity),
        );
        let denial = state.propose(seed, Track::Publication, capacity)?;
        #[cfg(feature = "selection-profile")]
        attribution.publication(&state, visits, denial, settled);
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
        #[cfg(feature = "selection-profile")]
        {
            attribution.publish.ns += start.elapsed().as_nanos();
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
    // A prepared index adds no per-call CSR passes.
    let visits = state.visits + index_visits + c * if initial.is_some() { 4 } else { 2 };
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
        "selection_profile initial={} prepared={} candidates={} domain_ns={} admission_ns={} index_ns={} sort_ns={} proposals_ns={} emit_ns={} blocked={} {}",
        initial.is_some(),
        matches!(metadata, Metadata::Prepared(_)),
        candidates,
        domain_ns,
        admitted_ns - domain_ns,
        indexed_ns - admitted_ns,
        sorted_ns - indexed_ns,
        proposed_ns - sorted_ns,
        profile.elapsed().as_nanos() - proposed_ns,
        attribution.blocked,
        attribution.line()
    );
    Ok(selection)
}

/// Profile-only split of proposal work: per transaction kind, attempts, rejections, denials
/// decided before all additions (`stopped`), closure groups, cluster touches and elapsed time.
/// Rejected publications also record why they failed, whether both capacity reasons were already
/// recorded (`settled`), the touches before the first unavailable added cut page, and how many
/// distinct groups their closures covered.
#[cfg(feature = "selection-profile")]
#[derive(Default)]
struct Transactions {
    attempts: usize,
    rejected: usize,
    stopped: usize,
    groups: usize,
    visits: usize,
    ns: u128,
}
#[cfg(feature = "selection-profile")]
impl Transactions {
    fn record(&mut self, state: &incremental::State<'_>, before: usize, rejected: bool) {
        self.attempts += 1;
        self.rejected += usize::from(rejected);
        self.stopped += usize::from(rejected && state.stopped);
        self.groups += state.closure().len();
        self.visits += state.visits - before;
    }
    fn line(&self, name: &str) -> String {
        format!(
            "{name}_attempts={} {name}_rejected={} {name}_stopped={} {name}_groups={} {name}_visits={} {name}_ns={}",
            self.attempts, self.rejected, self.stopped, self.groups, self.visits, self.ns
        )
    }
}
#[cfg(feature = "selection-profile")]
struct Attribution {
    blocked: usize,
    plan: Transactions,
    sync: Transactions,
    publish: Transactions,
    missing_only: usize,
    capacity: usize,
    settled: usize,
    witnessed: usize,
    witness_visits: usize,
    covered: Vec<bool>,
}
#[cfg(feature = "selection-profile")]
impl Attribution {
    fn new(groups: usize) -> Self {
        Self {
            blocked: 0,
            plan: Transactions::default(),
            sync: Transactions::default(),
            publish: Transactions::default(),
            missing_only: 0,
            capacity: 0,
            settled: 0,
            witnessed: 0,
            witness_visits: 0,
            covered: vec![false; groups],
        }
    }
    fn publication(
        &mut self,
        state: &incremental::State<'_>,
        before: usize,
        denial: Option<DebtReason>,
        settled: bool,
    ) {
        let rejected = denial.is_some() || state.published.missing != 0;
        self.publish.record(state, before, rejected);
        if !rejected {
            return;
        }
        self.capacity += usize::from(denial.is_some());
        self.missing_only += usize::from(denial.is_none());
        self.settled += usize::from(settled);
        if let Some(at) = state.witness {
            self.witnessed += 1;
            self.witness_visits += at - before;
        }
        for &g in state.closure() {
            self.covered[g as usize] = true;
        }
    }
    fn line(&self) -> String {
        format!(
            "{} {} {} publish_missing_only={} publish_capacity={} publish_settled={} publish_witnessed={} publish_witness_visits={} publish_distinct_groups={}",
            self.plan.line("plan"),
            self.sync.line("sync"),
            self.publish.line("publish"),
            self.missing_only,
            self.capacity,
            self.settled,
            self.witnessed,
            self.witness_visits,
            self.covered.iter().filter(|v| **v).count()
        )
    }
}
