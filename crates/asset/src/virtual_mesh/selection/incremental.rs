use super::*;

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

/// CSR of the clusters whose `refined` names each group, in ascending cluster order.
/// Immutable after construction; a prepared package retains one per mesh.
#[derive(Debug)]
pub(super) struct Incoming {
    offsets: Vec<usize>,
    links: Vec<u32>,
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
        for (id, c) in mesh.clusters.iter().enumerate() {
            if let Some(r) = c.refined {
                links[cursor[r.index()]] = id as u32;
                cursor[r.index()] += 1;
            }
        }
        Self { offsets, links }
    }

    pub fn bytes(&self) -> usize {
        self.offsets.capacity() * std::mem::size_of::<usize>()
            + self.links.capacity() * std::mem::size_of::<u32>()
    }
}

/// One call owns its covers and journals. No mutable selection state survives the call.
pub(super) struct State<'a> {
    mesh: &'a VirtualMesh,
    available: &'a [bool],
    incoming: &'a Incoming,
    fixed: Vec<bool>,
    pub planned: Cover,
    pub published: Cover,
    journal: Vec<u32>,
    gap: Vec<u32>,
    proposal: Vec<bool>,
    deltas: Vec<(PageId, bool)>,
    pub charged: usize,
    pub visits: usize,
}

impl<'a> State<'a> {
    pub fn new(
        mesh: &'a VirtualMesh,
        incoming: &'a Incoming,
        available: &'a [bool],
        active: Vec<bool>,
        pinned: &[PageId],
        protected: &[PageId],
    ) -> Self {
        let n = mesh.groups.len();
        let mut refs = vec![0; available.len()];
        let mut clusters = 0;
        for c in &mesh.clusters {
            if active[c.owner.index()] && c.refined.is_none_or(|r| !active[r.index()]) {
                refs[c.page.index()] += 1;
                clusters += 1;
            }
        }
        let mut fixed = vec![false; available.len()];
        for p in pinned.iter().chain(protected) {
            fixed[p.index()] = true;
        }
        let charged = fixed
            .iter()
            .zip(&refs)
            .filter(|(f, r)| **f || **r > 0)
            .count();
        Self {
            mesh,
            available,
            incoming,
            fixed,
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
            gap: Vec::with_capacity(n),
            proposal: vec![false; n],
            deltas: Vec::with_capacity(mesh.clusters.len()),
            charged,
            // The initial cover scan. Proposal/publication count each closure/boundary record
            // read below. Rollback replays pages without cluster reads.
            visits: mesh.clusters.len(),
        }
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
        let p = page.index();
        let before = self.fixed[p] || self.planned.refs[p] > 0 || self.published.refs[p] > 0;
        let available = self.available[p];
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
        let after = self.fixed[p] || self.planned.refs[p] > 0 || self.published.refs[p] > 0;
        self.charged = self.charged + usize::from(after) - usize::from(before);
    }

    fn mark(&mut self, g: usize, track: Track) {
        debug_assert!(matches!(track, Track::Plan) || self.planned.active[g]);
        self.cover_mut(track).active[g] = true;
        self.proposal[g] = true;
        self.journal.push(g as u32);
    }

    pub fn propose(&mut self, seed: usize, track: Track) {
        self.journal.clear();
        self.deltas.clear();
        self.mark(seed, track);
        let mut cursor = 0;
        while cursor < self.journal.len() {
            let g = self.journal[cursor] as usize;
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
        self.apply_boundary(track);
    }

    fn apply_boundary(&mut self, track: Track) {
        // All flags describe the final closure. Only its boundary changes the cut.
        // Internal coarse shards never enter page accounting, even temporarily.
        for i in 0..self.journal.len() {
            let g = self.journal[i] as usize;
            let span = self.mesh.groups[g].clusters;
            for id in span.offset as usize..(span.offset + span.count) as usize {
                self.visits += 1;
                let c = &self.mesh.clusters[id];
                if c.refined
                    .is_none_or(|r| !self.cover(track).active[r.index()])
                {
                    let page = c.page;
                    self.deltas.push((page, true));
                    self.page_delta(page, true, track);
                }
            }
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
    pub fn publish_plan(&mut self) {
        debug_assert_eq!(self.planned.missing, 0);
        self.journal.clear();
        self.deltas.clear();
        for i in 0..self.gap.len() {
            let g = self.gap[i] as usize;
            if !self.published.active[g] {
                self.mark(g, Track::Publication);
            }
        }
        self.apply_boundary(Track::Publication);
        self.accept(Track::Publication);
        self.gap.clear();
    }
}
