//! Independently versioned animated assets. RLS v1 embeds an ordinary RLA payload;
//! static v1-v6 caches and decoders keep their existing meaning.
use crate::{Asset, Error, Limits, Result};

pub const SCHEMA_VERSION: u32 = 1;
pub type Matrix = [[f32; 4]; 4];

#[derive(Clone, Debug, PartialEq)]
pub struct Influence {
    pub joints: [u16; 4],
    pub weights: [f32; 4],
}
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub parent: Option<u32>,
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}
#[derive(Clone, Debug, PartialEq)]
pub struct Skin {
    /// Node indices, in the glTF joint-index domain.
    pub joints: Vec<u32>,
    pub inverse_bind: Vec<Matrix>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub mesh: u32,
    pub node: u32,
    pub skin: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Property {
    Translation,
    Rotation,
    Scale,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    Step,
    Linear,
    CubicSpline,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub node: u32,
    pub property: Property,
    pub interpolation: Interpolation,
    pub times: Vec<f32>,
    /// Translation/scale use xyz, rotation xyzw. Cubic keys are in/value/out.
    pub values: Vec<[f32; 4]>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub channels: Vec<Channel>,
}
impl Clip {
    pub fn duration(&self) -> f32 {
        self.channels
            .iter()
            .filter_map(|c| c.times.last())
            .copied()
            .fold(0., f32::max)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct SkinnedAsset {
    pub geometry: Asset,
    pub nodes: Vec<Node>,
    pub skins: Vec<Skin>,
    pub bindings: Vec<Binding>,
    /// Mesh, primitive, vertex; None marks an ordinary primitive.
    pub influences: Vec<Vec<Option<Vec<Influence>>>>,
    pub clips: Vec<Clip>,
}

impl SkinnedAsset {
    pub fn validate(&self, limits: Limits) -> Result<()> {
        let count = self.geometry.validate(limits)?;
        if self.nodes.len() > limits.max_placements
            || self.skins.len() > limits.max_placements
            || self.bindings.len() > limits.max_placements
            || self.clips.len() > limits.max_primitives
            || self.influences.len() != self.geometry.meshes.len()
        {
            return Err(Error("skin counts exceed limits or geometry"));
        }
        let mut bytes = count.decoded_bytes;
        let mut add = |n: usize, stride: usize| -> Result<()> {
            bytes = bytes
                .checked_add(n.checked_mul(stride).ok_or(Error("skin size overflow"))?)
                .ok_or(Error("skin size overflow"))?;
            if bytes > limits.max_decoded_bytes {
                return Err(Error("skin decoded byte limit"));
            }
            Ok(())
        };
        add(self.nodes.len(), 44)?;
        for (i, n) in self.nodes.iter().enumerate() {
            if n.parent
                .is_some_and(|p| p as usize >= self.nodes.len() || p as usize == i)
                || !n
                    .translation
                    .iter()
                    .chain(&n.rotation)
                    .chain(&n.scale)
                    .all(|v| v.is_finite())
                || n.scale.contains(&0.)
                || (n.rotation.iter().map(|v| v * v).sum::<f32>() - 1.).abs() > 1e-3
            {
                return Err(Error("invalid skin node transform or parent"));
            }
        }
        // Linear-time hierarchy validation, including nodes outside the selected scene.
        let mut marks = vec![0u8; self.nodes.len()];
        for root in 0..self.nodes.len() {
            let mut path = vec![];
            let mut at = Some(root);
            while let Some(i) = at {
                if marks[i] == 2 {
                    break;
                }
                if marks[i] == 1 {
                    return Err(Error("skin hierarchy cycle"));
                }
                marks[i] = 1;
                path.push(i);
                at = self.nodes[i].parent.map(|p| p as usize);
            }
            for i in path {
                marks[i] = 2;
            }
        }
        for s in &self.skins {
            add(s.joints.len(), 68)?;
            if s.joints.is_empty()
                || s.joints.len() > u16::MAX as usize + 1
                || s.joints.len() != s.inverse_bind.len()
                || s.joints.iter().any(|&j| j as usize >= self.nodes.len())
                || s.inverse_bind
                    .iter()
                    .flatten()
                    .flatten()
                    .any(|v| !v.is_finite())
            {
                return Err(Error("invalid joints or inverse bind matrices"));
            }
        }
        for (mesh, streams) in self.geometry.meshes.iter().zip(&self.influences) {
            if streams.len() != mesh.primitives.len() {
                return Err(Error("skin primitive count mismatch"));
            }
            for (p, stream) in mesh.primitives.iter().zip(streams) {
                if let Some(stream) = stream {
                    add(stream.len(), 24)?;
                    if stream.len() != p.vertices.len() {
                        return Err(Error("skin vertex count mismatch"));
                    }
                    for v in stream {
                        if v.weights.iter().any(|w| !w.is_finite() || *w < 0.)
                            || (v.weights.iter().sum::<f32>() - 1.).abs() > 1e-4
                        {
                            return Err(Error("skin weights must be normalized and nonnegative"));
                        }
                    }
                }
            }
        }
        add(self.bindings.len(), 12)?;
        for b in &self.bindings {
            let skin = self
                .skins
                .get(b.skin as usize)
                .ok_or(Error("skin binding index"))?;
            if b.node as usize >= self.nodes.len() || b.mesh as usize >= self.geometry.meshes.len()
            {
                return Err(Error("skin mesh/node index"));
            }
            for stream in &self.influences[b.mesh as usize] {
                let stream = stream
                    .as_ref()
                    .ok_or(Error("skinned mesh missing joint attributes"))?;
                if stream
                    .iter()
                    .any(|v| v.joints.iter().any(|&j| j as usize >= skin.joints.len()))
                {
                    return Err(Error("vertex joint outside skin palette"));
                }
            }
        }
        for clip in &self.clips {
            if clip.channels.len() > limits.max_primitives {
                return Err(Error("animation channel limit"));
            }
            let mut targets = std::collections::HashSet::new();
            for c in &clip.channels {
                add(c.times.len(), 4)?;
                add(c.values.len(), 16)?;
                let multiplier = if c.interpolation == Interpolation::CubicSpline {
                    3
                } else {
                    1
                };
                if c.node as usize >= self.nodes.len()
                    || c.times.is_empty()
                    || c.times.len() > limits.max_indices
                    || !targets.insert((c.node, c.property as u32))
                    || c.times.iter().any(|t| !t.is_finite() || *t < 0.)
                    || c.times.windows(2).any(|w| w[0] >= w[1])
                    || c.times.len().checked_mul(multiplier) != Some(c.values.len())
                    || c.values.iter().flatten().any(|v| !v.is_finite())
                {
                    return Err(Error("invalid animation channel"));
                }
                if c.property == Property::Rotation {
                    for q in c
                        .values
                        .iter()
                        .skip(usize::from(multiplier == 3))
                        .step_by(multiplier)
                    {
                        if (q.iter().map(|v| v * v).sum::<f32>() - 1.).abs() > 1e-3 {
                            return Err(Error("animation rotation must be unit length"));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

struct Writer(Vec<u8>);
impl Writer {
    fn word(&mut self, x: u32) {
        self.0.extend(x.to_le_bytes());
    }
    fn count(&mut self, x: usize) -> Result<()> {
        self.word(x.try_into().map_err(|_| Error("skin count overflow"))?);
        Ok(())
    }
    fn floats(&mut self, x: impl IntoIterator<Item = f32>) {
        for v in x {
            self.word(v.to_bits());
        }
    }
}
pub fn encode(asset: &SkinnedAsset, limits: Limits) -> Result<Vec<u8>> {
    asset.validate(limits)?;
    let geometry = crate::encode(&asset.geometry, limits)?;
    let mut w = Writer(b"RLS\0".to_vec());
    w.word(SCHEMA_VERSION);
    w.count(geometry.len())?;
    w.0.extend(geometry);
    w.count(asset.nodes.len())?;
    for n in &asset.nodes {
        w.word(n.parent.unwrap_or(u32::MAX));
        w.floats(n.translation);
        w.floats(n.rotation);
        w.floats(n.scale);
    }
    w.count(asset.skins.len())?;
    for s in &asset.skins {
        w.count(s.joints.len())?;
        for (&j, m) in s.joints.iter().zip(&s.inverse_bind) {
            w.word(j);
            w.floats(m.iter().flatten().copied());
        }
    }
    w.count(asset.bindings.len())?;
    for b in &asset.bindings {
        w.word(b.mesh);
        w.word(b.node);
        w.word(b.skin);
    }
    for m in &asset.influences {
        for p in m {
            w.word(u32::from(p.is_some()));
            if let Some(p) = p {
                w.count(p.len())?;
                for v in p {
                    for j in v.joints {
                        w.word(j.into());
                    }
                    w.floats(v.weights);
                }
            }
        }
    }
    w.count(asset.clips.len())?;
    for c in &asset.clips {
        w.count(c.channels.len())?;
        for c in &c.channels {
            w.word(c.node);
            w.word(c.property as u32);
            w.word(c.interpolation as u32);
            w.count(c.times.len())?;
            w.floats(c.times.iter().copied());
            w.floats(c.values.iter().flatten().copied());
        }
    }
    if w.0.len() > limits.max_file_bytes {
        return Err(Error("skin file byte limit"));
    }
    Ok(w.0)
}
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let x = self.0.get(..n).ok_or(Error("truncated skin payload"))?;
        self.0 = &self.0[n..];
        Ok(x)
    }
    fn word(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn count(&mut self, limit: usize, stride: usize) -> Result<usize> {
        let n = self.word()? as usize;
        if n > limit || n.checked_mul(stride).is_none_or(|s| s > self.0.len()) {
            return Err(Error("skin count/span limit"));
        }
        Ok(n)
    }
    fn floats<const N: usize>(&mut self) -> Result<[f32; N]> {
        let mut x = [0.; N];
        for v in &mut x {
            *v = f32::from_bits(self.word()?);
        }
        Ok(x)
    }
}
pub fn decode(bytes: &[u8], limits: Limits) -> Result<SkinnedAsset> {
    if bytes.len() > limits.max_file_bytes || bytes.len() > limits.max_decoded_bytes {
        return Err(Error("skin file byte limit"));
    }
    let mut r = Reader(bytes);
    if r.take(4)? != b"RLS\0" || r.word()? != SCHEMA_VERSION {
        return Err(Error("unsupported skin schema"));
    }
    let n = r.count(limits.max_file_bytes, 1)?;
    let geometry = crate::decode(r.take(n)?, limits)?;
    let n = r.count(limits.max_placements, 44)?;
    let nodes = (0..n)
        .map(|_| {
            let p = r.word()?;
            Ok(Node {
                parent: (p != u32::MAX).then_some(p),
                translation: r.floats()?,
                rotation: r.floats()?,
                scale: r.floats()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let n = r.count(limits.max_placements, 4)?;
    let mut skins = vec![];
    for _ in 0..n {
        let n = r.count(65536, 68)?;
        let mut s = Skin {
            joints: vec![],
            inverse_bind: vec![],
        };
        for _ in 0..n {
            s.joints.push(r.word()?);
            let mut m = [[0.; 4]; 4];
            for c in &mut m {
                *c = r.floats()?;
            }
            s.inverse_bind.push(m);
        }
        skins.push(s);
    }
    let n = r.count(limits.max_placements, 12)?;
    let bindings = (0..n)
        .map(|_| {
            Ok(Binding {
                mesh: r.word()?,
                node: r.word()?,
                skin: r.word()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut influences = vec![];
    for mesh in &geometry.meshes {
        let mut streams = vec![];
        for p in &mesh.primitives {
            streams.push(match r.word()? {
                0 => None,
                1 => {
                    let n = r.count(p.vertices.len(), 32)?;
                    let mut v = vec![];
                    for _ in 0..n {
                        let mut joints = [0; 4];
                        for j in &mut joints {
                            *j = r
                                .word()?
                                .try_into()
                                .map_err(|_| Error("skin joint width"))?;
                        }
                        v.push(Influence {
                            joints,
                            weights: r.floats()?,
                        });
                    }
                    Some(v)
                }
                _ => return Err(Error("skin stream flag")),
            });
        }
        influences.push(streams);
    }
    let n = r.count(limits.max_primitives, 4)?;
    let mut clips = vec![];
    for _ in 0..n {
        let n = r.count(limits.max_primitives, 16)?;
        let mut channels = vec![];
        for _ in 0..n {
            let node = r.word()?;
            let property = match r.word()? {
                0 => Property::Translation,
                1 => Property::Rotation,
                2 => Property::Scale,
                _ => return Err(Error("animation property")),
            };
            let interpolation = match r.word()? {
                0 => Interpolation::Step,
                1 => Interpolation::Linear,
                2 => Interpolation::CubicSpline,
                _ => return Err(Error("animation interpolation")),
            };
            let stride = if interpolation == Interpolation::CubicSpline {
                52
            } else {
                20
            };
            let n = r.count(limits.max_indices, stride)?;
            let times = (0..n)
                .map(|_| Ok(f32::from_bits(r.word()?)))
                .collect::<Result<Vec<_>>>()?;
            let values = (0..n * if interpolation == Interpolation::CubicSpline {
                3
            } else {
                1
            })
                .map(|_| r.floats())
                .collect::<Result<Vec<_>>>()?;
            channels.push(Channel {
                node,
                property,
                interpolation,
                times,
                values,
            });
        }
        clips.push(Clip { channels });
    }
    if !r.0.is_empty() {
        return Err(Error("trailing skin bytes"));
    }
    let a = SkinnedAsset {
        geometry,
        nodes,
        skins,
        bindings,
        influences,
        clips,
    };
    a.validate(limits)?;
    Ok(a)
}
