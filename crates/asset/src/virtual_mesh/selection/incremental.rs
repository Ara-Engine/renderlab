use super::*;
use std::{cmp::Reverse, collections::BinaryHeap};

pub(super) struct Cover {
    pub active: Vec<bool>,
    pub refs: Vec<u32>,
    pub clusters: usize,
    pub missing: usize,
}

#[derive(Clone, Copy)]
pub(super) enum Track {
    Plan,
    Publication,
}

/// The caps that deny a transaction. `clusters` is already limited to the mesh's cluster count.
#[derive(Clone, Copy)]
pub(super) struct Capacity {
    pub clusters: usize,
    pub geometry_bytes: usize,
}
impl Capacity {
    /// Output before geometry; `geometry` false leaves only the output cap to decide.
    fn denial(self, clusters: usize, charged: usize, geometry: bool) -> Result<Option<DebtReason>> {
        Ok(if clusters > self.clusters {
            Some(DebtReason::OutputCapacity)
        } else if geometry && product(charged, PAGE_BYTES)? > self.geometry_bytes {
            Some(DebtReason::GeometryCapacity)
        } else {
            None
        })
    }
}

/// During additions the cut and charge only grow, so a denial seen there is final. Geometry may
/// decide only when the output cap provably holds for the completed transaction.
#[derive(Clone, Copy)]
struct Stop {
    capacity: Capacity,
    geometry: bool,
}

/// CSR of the clusters whose `refined` names each group, in ascending cluster order, and the
/// package page range spanned by the mesh's clusters. Per-call page state covers only that
/// range. Immutable after construction; a prepared package retains one per mesh.
#[derive(Debug)]
pub(super) struct Incoming {
    offsets: Vec<usize>,
    links: Vec<u32>,
    pages: std::ops::Range<usize>,
}

impl Incoming {
    /// Retained bytes and the transient build cursor bytes, known before allocating either.
    pub fn charge(mesh: &VirtualMesh) -> Result<(usize, usize)> {
        let links = mesh.clusters.iter().filter(|c| c.refined.is_some()).count();
        let offsets = product(mesh.groups.len() + 1, std::mem::size_of::<usize>())?;
        let retained = offsets
            .checked_add(product(links, std::mem::size_of::<u32>())?)
            .ok_or(Error("incoming index overflow"))?;
        Ok((
            retained,
            product(mesh.groups.len(), std::mem::size_of::<usize>())?,
        ))
    }

    pub fn new(mesh: &VirtualMesh) -> Self {
        let n = mesh.groups.len();
        let mut offsets = vec![0; n + 1];
        for c in &mesh.clusters {
            if let Some(r) = c.refined {
                offsets[r.index() + 1] += 1;
            }
        }
        for g in 0..n {
            offsets[g + 1] += offsets[g];
        }
        let mut links = vec![0; offsets[n]];
        let mut cursor = offsets[..n].to_vec();
        let (mut first, mut last) = (usize::MAX, 0);
        for (id, c) in mesh.clusters.iter().enumerate() {
            first = first.min(c.page.index());
            last = last.max(c.page.index());
            if let Some(r) = c.refined {
                links[cursor[r.index()]] = id as u32;
                cursor[r.index()] += 1;
            }
        }
        Self {
            offsets,
            links,
            pages: first.min(last + 1)..last + 1,
        }
    }

    /// Package pages from the mesh's lowest to its highest cluster page.
    pub fn pages(&self) -> std::ops::Range<usize> {
        self.pages.clone()
    }

    pub fn bytes(&self) -> usize {
        self.offsets.capacity() * std::mem::size_of::<usize>()
            + self.links.capacity() * std::mem::size_of::<u32>()
    }
}

/// One call owns its covers and journals. No mutable selection state survives the call.
/// Cover references are indexed from `window.start`; every cluster page lies in `window`.
pub(super) struct State<'a> {
    mesh: &'a VirtualMesh,
    available: &'a [bool],
    incoming: &'a Incoming,
    /// Pinned and externally protected package pages.
    fixed: PageSet,
    pub window: std::ops::Range<usize>,
    pub planned: Cover,
    pub published: Cover,
    journal: Vec<u32>,
    /// Discovered but unvisited publication-closure groups, smallest ID first.
    frontier: BinaryHeap<Reverse<u32>>,
    gap: Vec<u32>,
    proposal: Vec<bool>,
    deltas: Vec<(PageId, bool)>,
    pub charged: usize,
    /// Unique pages in this mesh together with pinned and externally protected pages.
    all_pages: usize,
    pub visits: usize,
    /// Profile only: `visits` when the current proposal first added an unavailable cut page.
    #[cfg(feature = "selection-profile")]
    pub witness: Option<usize>,
    /// Profile only: the current proposal's denial was decided before all additions.
    #[cfg(feature = "selection-profile")]
    pub stopped: bool,
}

impl<'a> State<'a> {
    pub fn new(
        mesh: &'a VirtualMesh,
        incoming: &'a Incoming,
        available: &'a [bool],
        active: Vec<bool>,
        fixed: PageSet,
    ) -> Self {
        let n = mesh.groups.len();
        let window = incoming.pages();
        let mut used = vec![false; window.len()];
        let mut refs = vec![0; window.len()];
        let mut clusters = 0;
        for c in &mesh.clusters {
            let i = c.page.index() - window.start;
            used[i] = true;
            if active[c.owner.index()] && c.refined.is_none_or(|r| !active[r.index()]) {
                refs[i] += 1;
                clusters += 1;
            }
        }
        // Fixed pages are charged once; window pages add only when not already fixed.
        let unfixed = |p: usize, used: bool| used && !fixed.contains(p);
        let all_pages = fixed.len()
            + window
                .clone()
                .zip(&used)
                .filter(|&(p, &u)| unfixed(p, u))
                .count();
        let charged = fixed.len()
            + window
                .clone()
                .zip(&refs)
                .filter(|&(p, &r)| unfixed(p, r > 0))
                .count();
        Self {
            mesh,
            available,
            incoming,
            fixed,
            window,
            published: Cover {
                active: active.clone(),
                refs: refs.clone(),
                clusters,
                missing: 0,
            },
            planned: Cover {
                active,
                refs,
                clusters,
                missing: 0,
            },
            journal: Vec::with_capacity(n),
            frontier: BinaryHeap::with_capacity(n),
            gap: Vec::with_capacity(n),
            proposal: vec![false; n],
            deltas: Vec::with_capacity(mesh.clusters.len()),
            charged,
            all_pages,
            // The initial cover scan. Proposal/publication count each closure/boundary record
            // read below. Rollback replays pages without cluster reads.
            visits: mesh.clusters.len(),
            #[cfg(feature = "selection-profile")]
            witness: None,
            #[cfg(feature = "selection-profile")]
            stopped: false,
        }
    }

    /// A publication is a subset of the admitted plan. It cannot replace a shard that
    /// the plan itself still needs. Decide it here only when capacity cannot deny it,
    /// so the ordinary transaction still reports every possible capacity reason. Then the
    /// transaction succeeds exactly when none of its added cut clusters has a missing page.
    pub fn publication_blocked(&mut self, seed: usize, capacity: Capacity) -> bool {
        if capacity.clusters < self.mesh.clusters.len()
            || self.all_pages > capacity.geometry_bytes / PAGE_BYTES
        {
            return false;
        }
        // Every inactive owner in the publication closure must become active too. Its added cut
        // clusters are those whose refinement stays inactive; removals cannot introduce a missing
        // page. Validated IDs order every refined group before its owners, so visiting the
        // closure in ascending ID order has already found every closure group that a visited
        // cluster could be refined by. The first added cluster with a missing page therefore
        // decides the whole publication before the rest of the closure is built. No cut changes.
        self.journal.clear();
        self.journal.push(seed as u32);
        self.proposal[seed] = true;
        self.frontier.clear();
        self.frontier.push(Reverse(seed as u32));
        let mut blocked = false;
        'closure: while let Some(Reverse(g)) = self.frontier.pop() {
            let g = g as usize;
            let span = self.mesh.groups[g].clusters;
            for id in span.offset as usize..(span.offset + span.count) as usize {
                self.visits += 1;
                let c = &self.mesh.clusters[id];
                if !self.available[c.page.index()]
                    && c.refined.is_none_or(|r| {
                        !self.published.active[r.index()] && !self.proposal[r.index()]
                    })
                {
                    blocked = true;
                    break 'closure;
                }
            }
            for i in self.incoming.offsets[g]..self.incoming.offsets[g + 1] {
                self.visits += 1;
                let owner = self.mesh.clusters[self.incoming.links[i] as usize]
                    .owner
                    .index();
                if !self.published.active[owner] && !self.proposal[owner] {
                    self.proposal[owner] = true;
                    self.journal.push(owner as u32);
                    self.frontier.push(Reverse(owner as u32));
                }
            }
        }
        for &g in &self.journal {
            self.proposal[g as usize] = false;
        }
        blocked
    }

    #[cfg(feature = "selection-profile")]
    pub fn closure(&self) -> &[u32] {
        &self.journal
    }

    fn cover(&self, track: Track) -> &Cover {
        match track {
            Track::Plan => &self.planned,
            Track::Publication => &self.published,
        }
    }

    fn cover_mut(&mut self, track: Track) -> &mut Cover {
        match track {
            Track::Plan => &mut self.planned,
            Track::Publication => &mut self.published,
        }
    }

    fn page_delta(&mut self, page: PageId, add: bool, track: Track) {
        let (fixed, available) = (
            self.fixed.contains(page.index()),
            self.available[page.index()],
        );
        let p = page.index() - self.window.start;
        let before = fixed || self.planned.refs[p] > 0 || self.published.refs[p] > 0;
        let cover = self.cover_mut(track);
        if !available && cover.refs[p] == u32::from(!add) {
            if add {
                cover.missing += 1;
            } else {
                cover.missing -= 1;
            }
        }
        if add {
            cover.refs[p] += 1;
            cover.clusters += 1;
        } else {
            cover.refs[p] -= 1;
            cover.clusters -= 1;
        }
        let after = fixed || self.planned.refs[p] > 0 || self.published.refs[p] > 0;
        self.charged = self.charged + usize::from(after) - usize::from(before);
    }

    fn mark(&mut self, g: usize, track: Track) {
        debug_assert!(matches!(track, Track::Plan) || self.planned.active[g]);
        self.cover_mut(track).active[g] = true;
        self.proposal[g] = true;
        self.journal.push(g as u32);
    }

    /// Activates `seed`'s owner closure on `track` and returns the transaction's capacity denial;
    /// the caller rolls back any denial. Removals are applied first, after which additions only
    /// grow the cut and its page charge, so additions stop once the denial and reason are certain.
    pub fn propose(
        &mut self,
        seed: usize,
        track: Track,
        capacity: Capacity,
    ) -> Result<Option<DebtReason>> {
        self.journal.clear();
        self.deltas.clear();
        #[cfg(feature = "selection-profile")]
        {
            self.witness = None;
            self.stopped = false;
        }
        self.mark(seed, track);
        let mut closure_clusters = 0;
        let mut cursor = 0;
        while cursor < self.journal.len() {
            let g = self.journal[cursor] as usize;
            closure_clusters += self.mesh.groups[g].clusters.count as usize;
            for i in self.incoming.offsets[g]..self.incoming.offsets[g + 1] {
                self.visits += 1;
                let owner = self.mesh.clusters[self.incoming.links[i] as usize]
                    .owner
                    .index();
                if !self.cover(track).active[owner] {
                    self.mark(owner, track);
                }
            }
            cursor += 1;
        }
        self.remove_replaced(track);
        // Additions are closure clusters, and no cut exceeds the mesh. When even that bound fits
        // the output cap, only the geometry charge can deny this transaction.
        let geometry = (self.cover(track).clusters + closure_clusters)
            .min(self.mesh.clusters.len())
            <= capacity.clusters;
        if let Some(reason) = self.add_uncovered(track, Some(Stop { capacity, geometry }))? {
            #[cfg(feature = "selection-profile")]
            {
                self.stopped = true;
            }
            return Ok(Some(reason));
        }
        capacity.denial(self.cover(track).clusters, self.charged, true)
    }

    // All flags describe the final closure. Only its boundary changes the cut, and internal
    // coarse shards never enter page accounting, even temporarily. Delta order does not change
    // the final references, missing pages or charge.
    fn remove_replaced(&mut self, track: Track) {
        for i in 0..self.journal.len() {
            let g = self.journal[i] as usize;
            for link in self.incoming.offsets[g]..self.incoming.offsets[g + 1] {
                self.visits += 1;
                let c = &self.mesh.clusters[self.incoming.links[link] as usize];
                if self.cover(track).active[c.owner.index()] && !self.proposal[c.owner.index()] {
                    let page = c.page;
                    self.deltas.push((page, false));
                    self.page_delta(page, false, track);
                }
            }
        }
    }

    /// Returns a denial only when `stop` proves it before every addition is applied.
    fn add_uncovered(&mut self, track: Track, stop: Option<Stop>) -> Result<Option<DebtReason>> {
        for i in 0..self.journal.len() {
            let g = self.journal[i] as usize;
            let span = self.mesh.groups[g].clusters;
            for id in span.offset as usize..(span.offset + span.count) as usize {
                self.visits += 1;
                let c = &self.mesh.clusters[id];
                if c.refined
                    .is_none_or(|r| !self.cover(track).active[r.index()])
                {
                    let (page, charged) = (c.page, self.charged);
                    self.deltas.push((page, true));
                    self.page_delta(page, true, track);
                    #[cfg(feature = "selection-profile")]
                    if self.witness.is_none() && !self.available[page.index()] {
                        self.witness = Some(self.visits);
                    }
                    if let Some(Stop { capacity, geometry }) = stop
                        && (self.charged > charged || !geometry)
                        && let Some(reason) =
                            capacity.denial(self.cover(track).clusters, self.charged, geometry)?
                    {
                        return Ok(Some(reason));
                    }
                }
            }
        }
        Ok(None)
    }

    pub fn rollback(&mut self, track: Track) {
        for i in (0..self.deltas.len()).rev() {
            let (page, add) = self.deltas[i];
            self.page_delta(page, !add, track);
        }
        for i in 0..self.journal.len() {
            let g = self.journal[i];
            self.cover_mut(track).active[g as usize] = false;
            self.proposal[g as usize] = false;
        }
    }

    pub fn accept(&mut self, track: Track) {
        if matches!(track, Track::Plan) {
            self.gap.extend_from_slice(&self.journal);
        }
        for &g in &self.journal {
            self.proposal[g as usize] = false;
        }
    }

    /// The admitted plan is available and fits; synchronizing removes transition pages.
    /// Filter the gap because independent publication can already have activated some groups.
    pub fn publish_plan(&mut self) -> Result<()> {
        debug_assert_eq!(self.planned.missing, 0);
        self.journal.clear();
        self.deltas.clear();
        for i in 0..self.gap.len() {
            let g = self.gap[i] as usize;
            if !self.published.active[g] {
                self.mark(g, Track::Publication);
            }
        }
        self.remove_replaced(Track::Publication);
        self.add_uncovered(Track::Publication, None)?;
        self.accept(Track::Publication);
        self.gap.clear();
        Ok(())
    }
}
