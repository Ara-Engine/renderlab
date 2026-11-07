use super::*;

// Explicit LE fields: never serialize Rust layout/padding.
pub fn encode(bvh: &Bvh, limits: Limits) -> Result<Vec<u8>> {
    bvh.validate(limits)?;
    let mut out = b"RLB\0".to_vec();
    out.extend(VERSION.to_le_bytes());
    for n in [
        bvh.nodes.len(),
        bvh.leaves.len(),
        bvh.levels.len(),
        bvh.level_leaves.len(),
        bvh.positions.len(),
        bvh.indices.len(),
    ] {
        out.extend(
            u32::try_from(n)
                .map_err(|_| Error("BVH encoded count overflow"))?
                .to_le_bytes(),
        );
    }
    for n in &bvh.nodes {
        for f in n.origin.into_iter().chain(n.step) {
            out.extend(f.to_le_bytes());
        }
        for c in n.children {
            out.extend(c.to_le_bytes());
        }
        for a in n.lower.into_iter().chain(n.upper) {
            out.extend(a);
        }
        for f in n.errors {
            out.extend(f.to_le_bytes());
        }
    }
    for l in &bvh.leaves {
        for v in [
            l.mesh,
            l.cluster,
            l.page,
            l.source_index_offset,
            l.indices.offset,
            l.indices.count,
            l.error.to_bits(),
            l.material,
        ] {
            out.extend(v.to_le_bytes());
        }
    }
    for l in &bvh.levels {
        for v in [
            l.mesh,
            l.root,
            l.requested_error.to_bits(),
            l.max_error.to_bits(),
            l.leaves.offset,
            l.leaves.count,
        ] {
            out.extend(v.to_le_bytes());
        }
    }
    for &v in &bvh.level_leaves {
        out.extend(v.to_le_bytes());
    }
    for p in &bvh.positions {
        for v in p {
            out.extend(v.to_le_bytes());
        }
    }
    for &v in &bvh.indices {
        out.extend(v.to_le_bytes());
    }
    if out.len() > limits.max_bytes {
        return Err(Error("BVH encoded byte limit"));
    }
    Ok(out)
}
struct Reader<'a>(&'a [u8]);
impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N]> {
        let (a, b) = self
            .0
            .split_at_checked(N)
            .ok_or(Error("truncated cluster BVH"))?;
        self.0 = b;
        Ok(a.try_into().unwrap())
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    fn float(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn floats<const N: usize>(&mut self) -> Result<[f32; N]> {
        let mut out = [0.; N];
        for f in &mut out {
            *f = self.float()?;
        }
        Ok(out)
    }
    fn span(&mut self) -> Result<Span> {
        Ok(Span {
            offset: self.u32()?,
            count: self.u32()?,
        })
    }
}
pub fn decode(bytes: &[u8], limits: Limits) -> Result<Bvh> {
    if bytes.len() > limits.max_bytes {
        return Err(Error("BVH file byte limit"));
    }
    let mut r = Reader(bytes);
    if r.take::<4>()? != *b"RLB\0" || r.u32()? != VERSION {
        return Err(Error("unknown cluster BVH schema"));
    }
    let mut counts = [0usize; 6];
    let mut length = 32usize;
    for (n, stride) in counts
        .iter_mut()
        .zip([NODE_BYTES, LEAF_BYTES, LEVEL_BYTES, 4, 16, 4])
    {
        *n = r.u32()? as usize;
        length = length
            .checked_add(n.checked_mul(stride).ok_or(Error("BVH count overflow"))?)
            .ok_or(Error("BVH length overflow"))?;
    }
    if length != bytes.len() || length > limits.max_bytes {
        return Err(Error("BVH extent/count limit"));
    }
    let [nn, nl, nt, nc, nv, ni] = counts;
    let mut b = Bvh {
        nodes: Vec::with_capacity(nn),
        leaves: Vec::with_capacity(nl),
        levels: Vec::with_capacity(nt),
        level_leaves: Vec::with_capacity(nc),
        positions: Vec::with_capacity(nv),
        indices: Vec::with_capacity(ni),
    };
    for _ in 0..nn {
        let mut n = Node {
            origin: r.floats()?,
            step: r.floats()?,
            ..Node::default()
        };
        for c in &mut n.children {
            *c = r.u32()?;
        }
        for a in &mut n.lower {
            *a = r.take()?;
        }
        for a in &mut n.upper {
            *a = r.take()?;
        }
        n.errors = r.floats()?;
        b.nodes.push(n);
    }
    for _ in 0..nl {
        b.leaves.push(Leaf {
            mesh: r.u32()?,
            cluster: r.u32()?,
            page: r.u32()?,
            source_index_offset: r.u32()?,
            indices: r.span()?,
            error: r.float()?,
            material: r.u32()?,
        });
    }
    for _ in 0..nt {
        b.levels.push(Level {
            mesh: r.u32()?,
            root: r.u32()?,
            requested_error: r.float()?,
            max_error: r.float()?,
            leaves: r.span()?,
        });
    }
    for _ in 0..nc {
        b.level_leaves.push(r.u32()?);
    }
    for _ in 0..nv {
        b.positions.push(r.floats()?);
    }
    for _ in 0..ni {
        b.indices.push(r.u32()?);
    }
    b.validate(limits)?;
    Ok(b)
}
