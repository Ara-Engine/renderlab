//! A play session's motion replayed on its scene dump, one play step a frame: the camera, every
//! placement that moved and every skinned mesh posed as the game posed it. The files come from a
//! dump armed with `motion` (src/engine/scene_dump.rs; u-sunrt/bench/motion_dump.py drives the
//! play gate's own session): `motion.bin` (a delta record per renderer frame), `skins.rla` and
//! `skins.bin` (each skinned mesh's rest primitive and influences) and `steps.json` (the renderer
//! frame each play step's first frame landed on, its `still` steps then its `walk` steps).
//!
//! Skinned meshes are drawn as posable instances (`Renderer::try_upload_skinned_mesh`) in place
//! of the dump's frozen copies, so the renderer deforms them and the lighting cache refits and
//! patches their software geometry every pose, as in the game. A skinned mesh the session never
//! posed again keeps its bind pose.
use super::kit;
use crate::asset::skin::Influence;
use crate::renderer::Renderer;
use crate::{Error, Instances, Mat4, SkinnedMeshInstance, Vec3};
use std::collections::HashMap;
use std::path::Path;

struct Record {
    frame: u64,
    eye: Vec3,
    target: Vec3,
    models: Vec<(u32, Mat4)>,
    palettes: Vec<(u32, Vec<Mat4>)>,
}

/// A skinned mesh drawn posed: its source mesh in scene.rla (for its material), the posable
/// instance and its placements' instances.
pub(super) struct Skin {
    pub scene_mesh: usize,
    pub posed: SkinnedMeshInstance,
    pub instances: Instances,
}

pub(super) struct Replay {
    records: Vec<Record>,
    /// The renderer frame each bench frame replays up to: the still steps, then the walk steps.
    steps: Vec<u64>,
    /// How many of `steps` are the still window's.
    pub still: usize,
    next: usize,
    /// Each scene.rla placement's (batch, slot) pairs in `Prepared`'s order.
    batches: Vec<Vec<(usize, usize)>>,
    /// Placements of skinned meshes: (skin, slot in its instances).
    skinned: HashMap<u32, (usize, usize)>,
    pub skins: Vec<Skin>,
    /// The batches the posed skins replace (not drawn).
    pub skip: Vec<bool>,
    /// The dump's own models, restored at the start of each run.
    initial: Vec<Mat4>,
    touched: Vec<u32>,
}

/// A little-endian reader over a motion file.
struct Bytes<'a>(&'a [u8]);
impl Bytes<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (head, tail) = self.0.split_at_checked(N)?;
        self.0 = tail;
        head.try_into().ok()
    }
    fn u32(&mut self) -> Option<u32> {
        self.take().map(u32::from_le_bytes)
    }
    fn f32(&mut self) -> Option<f32> {
        self.take().map(f32::from_le_bytes)
    }
    fn mat4(&mut self) -> Option<Mat4> {
        let mut m = [0.0; 16];
        for v in &mut m {
            *v = self.f32()?;
        }
        Some(Mat4::from_cols_array(&m))
    }
}

fn records(bytes: &[u8]) -> Option<Vec<Record>> {
    let mut b = Bytes(bytes);
    let mut out = vec![];
    while !b.0.is_empty() {
        let frame = u64::from_le_bytes(b.take()?);
        let eye = Vec3::new(b.f32()?, b.f32()?, b.f32()?);
        let target = Vec3::new(b.f32()?, b.f32()?, b.f32()?);
        let models = (0..b.u32()?).map(|_| Some((b.u32()?, b.mat4()?))).collect::<Option<_>>()?;
        let palettes = (0..b.u32()?)
            .map(|_| {
                let skin = b.u32()?;
                Some((skin, (0..b.u32()?).map(|_| b.mat4()).collect::<Option<_>>()?))
            })
            .collect::<Option<_>>()?;
        out.push(Record { frame, eye, target, models, palettes });
    }
    Some(out)
}

impl Replay {
    /// The replay of the dump in `dir`, or None when it has no motion recording. `source` is
    /// scene.rla, before `Prepared` takes it.
    pub(super) fn load(dir: &Path, source: &crate::asset::Asset, r: &mut Renderer) -> Result<Option<Self>, Error> {
        let Ok(motion) = std::fs::read(dir.join("motion.bin")) else { return Ok(None) };
        let bad = |what: &'static str| Error::InvalidInput(what);
        let records = records(&motion).ok_or(bad("bench replay: motion.bin"))?;
        let steps: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("steps.json")).map_err(|_| bad("bench replay: steps.json"))?).map_err(|_| bad("bench replay: steps.json"))?;
        let list = |k: &str| steps[k].as_array().into_iter().flatten().filter_map(|v| v.as_u64()).collect::<Vec<_>>();
        let (still, walk) = (list("still"), list("walk"));
        if walk.is_empty() {
            return Err(bad("bench replay: steps.json has no walk steps"));
        }
        // Prepared's order: a batch per primitive of each placed mesh, a slot per placement of that mesh.
        let mut first_batch = vec![0usize; source.meshes.len()];
        let mut placed = vec![0usize; source.meshes.len()];
        for p in &source.placements {
            placed[p.mesh as usize] += 1;
        }
        let mut next_batch = 0;
        for (m, mesh) in source.meshes.iter().enumerate() {
            first_batch[m] = next_batch;
            if placed[m] > 0 {
                next_batch += mesh.primitives.len();
            }
        }
        let mut slot_of = vec![0usize; source.meshes.len()];
        let batches: Vec<Vec<(usize, usize)>> = source
            .placements
            .iter()
            .map(|p| {
                let m = p.mesh as usize;
                let slot = slot_of[m];
                slot_of[m] += 1;
                (0..source.meshes[m].primitives.len()).map(|k| (first_batch[m] + k, slot)).collect()
            })
            .collect();
        let initial = source.placements.iter().map(|p| Mat4::from_cols_array_2d(&p.transform)).collect();
        // The skinned meshes, posable, in place of their frozen batches.
        let mut skins = vec![];
        let mut skinned = HashMap::new();
        let mut skip = vec![false; next_batch];
        if let (Ok(rest), Ok(table)) = (std::fs::read(dir.join("skins.rla")), std::fs::read(dir.join("skins.bin"))) {
            let rest = crate::asset::decode(&rest, crate::asset::Limits::unlimited()).map_err(|_| bad("bench replay: skins.rla"))?;
            let mut b = Bytes(&table);
            let count = b.u32().ok_or(bad("bench replay: skins.bin"))?;
            for k in 0..count as usize {
                let (scene_mesh, joints, vertices) = (|| Some((b.u32()? as usize, b.u32()?, b.u32()?)))().ok_or(bad("bench replay: skins.bin"))?;
                let influences: Vec<Influence> = (0..vertices)
                    .map(|_| {
                        let mut joints = [0u16; 4];
                        for j in &mut joints {
                            *j = u16::from_le_bytes(b.take()?);
                        }
                        let mut weights = [0f32; 4];
                        for w in &mut weights {
                            *w = b.f32()?;
                        }
                        Some(Influence { joints, weights })
                    })
                    .collect::<Option<_>>()
                    .ok_or(bad("bench replay: skins.bin influences"))?;
                let primitive = rest.meshes.get(k).and_then(|m| m.primitives.first()).ok_or(bad("bench replay: skins.rla mesh"))?;
                let posed = kit::admitted(r, "bench skinned mesh", |r| r.try_upload_skinned_mesh(primitive, &influences, joints))?;
                let placements: Vec<u32> = (0..source.placements.len() as u32).filter(|&i| source.placements[i as usize].mesh as usize == scene_mesh).collect();
                let models: Vec<Mat4> = placements.iter().map(|&i| Mat4::from_cols_array_2d(&source.placements[i as usize].transform)).collect();
                if models.is_empty() {
                    continue;
                }
                let instances = kit::admitted(r, "bench skinned instances", |r| r.try_upload_instances(&models))?;
                for (slot, &p) in placements.iter().enumerate() {
                    skinned.insert(p, (skins.len(), slot));
                    for &(batch, _) in &batches[p as usize] {
                        skip[batch] = true;
                    }
                }
                skins.push(Skin { scene_mesh, posed, instances });
            }
        }
        Ok(Some(Self { records, still: still.len(), steps: still.into_iter().chain(walk).collect(), next: 0, batches, skinned, skins, skip, initial, touched: vec![] }))
    }

    /// Bench frames: the still steps, then the walk steps.
    pub(super) fn frames(&self) -> usize {
        self.steps.len()
    }

    /// Back to the dump's own state, for a new run.
    pub(super) fn rewind(&mut self, asset: &mut super::dump::Prepared) -> Result<(), Error> {
        for p in std::mem::take(&mut self.touched) {
            self.place(asset, p, self.initial[p as usize])?;
        }
        self.next = 0;
        Ok(())
    }

    fn place(&mut self, asset: &mut super::dump::Prepared, p: u32, model: Mat4) -> Result<(), Error> {
        if let Some(&(skin, slot)) = self.skinned.get(&p) {
            return self.skins[skin].instances.set(slot, model);
        }
        for &(batch, slot) in &self.batches[p as usize] {
            asset.set_model(batch, slot, model)?;
        }
        Ok(())
    }

    /// Applies every record up to bench frame `k`'s renderer frame; returns the camera then (eye,
    /// target), or None before the first record.
    pub(super) fn advance(&mut self, k: usize, asset: &mut super::dump::Prepared) -> Result<Option<(Vec3, Vec3)>, Error> {
        let until = self.steps[k];
        let mut camera = None;
        while self.next < self.records.len() && self.records[self.next].frame <= until {
            let record = std::mem::replace(&mut self.records[self.next], Record { frame: 0, eye: Vec3::ZERO, target: Vec3::ZERO, models: vec![], palettes: vec![] });
            for &(p, model) in &record.models {
                self.place(asset, p, model)?;
                self.touched.push(p);
            }
            for (skin, palette) in &record.palettes {
                if let Some(s) = self.skins.get_mut(*skin as usize) {
                    s.posed.set_joint_palette(palette)?;
                }
            }
            camera = Some((record.eye, record.target));
            self.records[self.next] = record;
            self.next += 1;
        }
        Ok(camera)
    }
}
