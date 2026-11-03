use super::*;

pub(super) struct Cover {
    pub active: Vec<bool>,
    pub refs: Vec<u32>,
    pub clusters: usize,
}

/// One call owns all indices and journals. No mutable selection state survives the call.
pub(super) struct State<'a> {
    mesh: &'a VirtualMesh,
    available: &'a [bool],
    offsets: Vec<usize>,
    incoming: Vec<u32>,
    fixed: Vec<bool>,
    pub planned: Cover,
    pub published: Cover,
    journal: Vec<u32>,
    pending: Vec<u32>,
    proposal: Vec<bool>,
    deltas: Vec<(PageId, bool)>,
    pub charged: usize,
    missing: usize,
    pub visits: usize,
}

impl<'a> State<'a> {
    pub fn new(
        mesh: &'a VirtualMesh,
        available: &'a [bool],
        active: Vec<bool>,
        pinned: &[PageId],
        protected: &[PageId],
    ) -> Self {
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
        let mut incoming = vec![0; offsets[n]];
        let mut cursor = offsets[..n].to_vec();
        for (id, c) in mesh.clusters.iter().enumerate() {
            if let Some(r) = c.refined {
                incoming[cursor[r.index()]] = id as u32;
                cursor[r.index()] += 1;
            }
        }
        drop(cursor);
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
            offsets,
            incoming,
            fixed,
            published: Cover {
                active: active.clone(),
                refs: refs.clone(),
                clusters,
            },
            planned: Cover {
                active,
                refs,
                clusters,
            },
            journal: Vec::with_capacity(n),
            pending: Vec::with_capacity(n),
            proposal: vec![false; n],
            deltas: Vec::with_capacity(mesh.clusters.len()),
            charged,
            missing: 0,
            // Two CSR passes and the initial cover scan. Proposal/publication count each
            // closure/boundary record read below. Rollback replays pages without cluster reads.
            visits: mesh.clusters.len() * 3,
        }
    }

    fn page_delta(&mut self, page: PageId, add: bool, planning: bool) {
        let p = page.index();
        let before = self.fixed[p] || self.planned.refs[p] > 0 || self.published.refs[p] > 0;
        let cover = if planning {
            &mut self.planned
        } else {
            &mut self.published
        };
        if planning && !self.available[p] && cover.refs[p] == u32::from(!add) {
            if add {
                self.missing += 1;
            } else {
                self.missing -= 1;
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

    fn mark(&mut self, g: usize) {
        self.planned.active[g] = true;
        self.proposal[g] = true;
        self.journal.push(g as u32);
    }

    pub fn propose(&mut self, seed: usize) {
        self.journal.clear();
        self.deltas.clear();
        self.mark(seed);
        let mut cursor = 0;
        while cursor < self.journal.len() {
            let g = self.journal[cursor] as usize;
            for i in self.offsets[g]..self.offsets[g + 1] {
                self.visits += 1;
                let owner = self.mesh.clusters[self.incoming[i] as usize].owner.index();
                if !self.planned.active[owner] {
                    self.mark(owner);
                }
            }
            cursor += 1;
        }
        // All flags now describe the final closure. Only its boundary changes the cut.
        // Internal coarse shards never enter page accounting, even temporarily.
        for i in 0..self.journal.len() {
            let g = self.journal[i] as usize;
            let span = self.mesh.groups[g].clusters;
            for id in span.offset as usize..(span.offset + span.count) as usize {
                self.visits += 1;
                let c = &self.mesh.clusters[id];
                if c.refined.is_none_or(|r| !self.planned.active[r.index()]) {
                    let page = c.page;
                    self.deltas.push((page, true));
                    self.page_delta(page, true, true);
                }
            }
            for link in self.offsets[g]..self.offsets[g + 1] {
                self.visits += 1;
                let c = &self.mesh.clusters[self.incoming[link] as usize];
                if self.planned.active[c.owner.index()] && !self.proposal[c.owner.index()] {
                    let page = c.page;
                    self.deltas.push((page, false));
                    self.page_delta(page, false, true);
                }
            }
        }
    }

    pub fn rollback(&mut self) {
        for i in (0..self.deltas.len()).rev() {
            let (page, add) = self.deltas[i];
            self.page_delta(page, !add, true);
        }
        for &g in &self.journal {
            self.planned.active[g as usize] = false;
            self.proposal[g as usize] = false;
        }
    }

    pub fn accept(&mut self) {
        self.pending.extend_from_slice(&self.journal);
        for &g in &self.journal {
            self.proposal[g as usize] = false;
        }
        if self.missing != 0 {
            return;
        }
        for i in 0..self.pending.len() {
            let g = self.pending[i] as usize;
            let span = self.mesh.groups[g].clusters;
            for id in span.offset as usize..(span.offset + span.count) as usize {
                self.visits += 1;
                let c = &self.mesh.clusters[id];
                if c.refined.is_none_or(|r| !self.planned.active[r.index()]) {
                    self.page_delta(c.page, true, false);
                }
            }
            for link in self.offsets[g]..self.offsets[g + 1] {
                self.visits += 1;
                let c = &self.mesh.clusters[self.incoming[link] as usize];
                if self.published.active[c.owner.index()] {
                    self.page_delta(c.page, false, false);
                }
            }
        }
        for &g in &self.pending {
            self.published.active[g as usize] = true;
        }
        self.pending.clear();
    }
}
