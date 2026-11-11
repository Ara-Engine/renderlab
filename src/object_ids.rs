//! Entity-ID output's device-free half (docs/design-id-output.md): the embedder's ids per
//! primitive range of each object, the per-placement tables a frame's ID pass reads, and the
//! readback's request and event types. Both GPU layers share this; the wgpu pass is
//! src/id_output.rs.
use crate::{Error, FrameId, engine::ObjectId, engine::ViewId};
use std::collections::HashMap;

/// No surface: sky or background.
pub const ID_NONE: u32 = 0;
/// A surface with no id: an object (or a primitive range of one) the embedder did not tag.
pub const ID_UNTAGGED: u32 = 0xFFFF_FFFF;
/// Largest `request_ids` rectangle, per side.
pub const MAX_ID_RECT: u32 = 512;
/// Id readbacks in flight per engine; a request beyond them is refused.
pub const ID_SLOTS: usize = 4;
/// Rendered frames a readback may take before it is reported as `IdsLost`.
pub const ID_TICKS: u32 = 8;
/// Undelivered id events kept between polls; older ones are dropped and counted.
pub const KEPT_ID_EVENTS: usize = 64;

/// A view's optional outputs; everything is off by default and costs nothing when off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    /// Per-pixel ids (`IdPixel`) of every rendered frame, readable with `request_ids` and,
    /// on the wgpu engine, as a texture (`Engine::view_ids`).
    pub ids: bool,
}

/// Which of a pixel's two ids a handoff writes (`Engine::render_view_ids_to`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum IdLayer {
    /// The nearest opaque surface: what outlines and occlusion read every frame.
    #[default]
    Opaque,
    /// The frontmost surface, glass included: what a click picks.
    Front,
}

/// A rectangle of a view's output, in pixels from the top-left corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IdRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
impl IdRect {
    /// One pixel, as a click picks.
    pub fn pixel(x: u32, y: u32) -> Self {
        Self {
            x,
            y,
            width: 1,
            height: 1,
        }
    }
    /// Nonempty, at most `MAX_ID_RECT` a side, and inside a `width` x `height` view.
    pub fn check(self, width: u32, height: u32) -> Result<(), Error> {
        if self.width == 0 || self.height == 0 {
            return Err(Error::InvalidInput("id rectangle must be nonempty"));
        }
        if self.width > MAX_ID_RECT || self.height > MAX_ID_RECT {
            return Err(Error::InvalidInput(
                "id rectangle exceeds MAX_ID_RECT; read a large region in parts",
            ));
        }
        if u64::from(self.x) + u64::from(self.width) > u64::from(width)
            || u64::from(self.y) + u64::from(self.height) > u64::from(height)
        {
            return Err(Error::InvalidInput("id rectangle must lie inside the view"));
        }
        Ok(())
    }
}

/// Names one `request_ids` call; its `IdReadback` or `IdsLost` carries the same value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IdRequest(pub(crate) u64);

/// The two ids of one pixel: `ID_NONE` for no surface, `ID_UNTAGGED` for a surface without
/// an id, else the embedder's id.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct IdPixel {
    /// The nearest opaque surface (cut-out geometry is opaque, so its holes show through).
    pub opaque: u32,
    /// The nearest transparent pane covering at least a tenth of the pixel in front of the
    /// opaque surface, else the opaque surface. Particles are effects and never ids.
    pub front: u32,
}

/// A finished `request_ids`: the rectangle of the view's frame `frame`, rows top first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdReadback {
    pub request: IdRequest,
    pub view: ViewId,
    pub frame: FrameId,
    pub rect: IdRect,
    pub ids: Vec<IdPixel>,
}
impl IdReadback {
    /// The pixel at (x, y) inside the rectangle's own coordinates.
    pub fn at(&self, x: u32, y: u32) -> IdPixel {
        self.ids[(y * self.rect.width + x) as usize]
    }
}

/// A `request_ids` whose copy did not map within `ID_TICKS` rendered frames; reported once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdsLost {
    pub request: IdRequest,
    pub view: ViewId,
}

/// One object's ids: sorted, disjoint primitive ranges `(first, count, id)`.
type Ranges = Vec<(u32, u32, u32)>;

/// The embedder's ids per object, kept by the engine (scene-wide, shared by every view).
#[derive(Default)]
pub(crate) struct ObjectIds {
    ids: HashMap<ObjectId, Ranges>,
}
impl ObjectIds {
    /// Sets an object's primitive ranges (empty clears them). Ranges must be nonempty,
    /// disjoint and carry no reserved id; they may come in any order. The caller checks
    /// that the object is live.
    pub fn set(&mut self, object: ObjectId, ranges: &[(u32, u32, u32)]) -> Result<(), Error> {
        let mut sorted = ranges.to_vec();
        sorted.sort_unstable();
        for (i, &(first, count, id)) in sorted.iter().enumerate() {
            if count == 0 || id == ID_NONE || id == ID_UNTAGGED {
                return Err(Error::InvalidInput(
                    "id ranges must be nonempty and ids neither 0 nor 0xFFFF_FFFF",
                ));
            }
            if let Some(&(next, ..)) = sorted.get(i + 1)
                && u64::from(first) + u64::from(count) > u64::from(next)
            {
                return Err(Error::InvalidInput("id ranges overlap"));
            }
        }
        if sorted.is_empty() {
            self.ids.remove(&object);
        } else {
            self.ids.insert(object, sorted);
        }
        Ok(())
    }
    pub fn forget(&mut self, object: ObjectId) {
        self.ids.remove(&object);
    }
    /// The frame's tables from its batches in draw order, each batch's objects in
    /// instance-slot order (`State::batches`): one entry per placement of the opaque
    /// batches, then one per instance of the transparent batches, as the renderer's stable
    /// split orders the draws.
    pub fn tables<'a>(
        &self,
        batches: impl Iterator<Item = (bool, &'a [ObjectId])>,
        tables: &mut TagTables,
    ) {
        tables.opaque.clear();
        tables.transparent.clear();
        tables.ranges.clear();
        for (transparent, objects) in batches {
            for object in objects {
                let ranges = self.ids.get(object).map_or(&[][..], |r| &r[..]);
                let entry = [tables.ranges.len() as u32, ranges.len() as u32];
                tables
                    .ranges
                    .extend(ranges.iter().map(|&(first, count, id)| [first, count, id, 0]));
                if transparent {
                    tables.transparent.push(entry);
                } else {
                    tables.opaque.push(entry);
                }
            }
        }
    }
}

/// One frame's id tables: per opaque placement and per transparent instance, its range
/// list `[offset, count]` in `ranges`, whose entries are `[first, count, id, 0]`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagTables {
    pub opaque: Vec<[u32; 2]>,
    pub transparent: Vec<[u32; 2]>,
    pub ranges: Vec<[u32; 4]>,
}
impl TagTables {
    /// The id of `primitive` of table entry `entry`, as the ID pass looks it up: the
    /// binary search over the entry's ranges, `ID_UNTAGGED` outside them. `None` is a
    /// primitive the pass does not know (raster glass): the first range's id.
    pub fn lookup(&self, entry: [u32; 2], primitive: Option<u32>) -> u32 {
        let ranges = &self.ranges[entry[0] as usize..(entry[0] + entry[1]) as usize];
        let Some(primitive) = primitive else {
            return ranges.first().map_or(ID_UNTAGGED, |r| r[2]);
        };
        let i = ranges.partition_point(|r| r[0] <= primitive);
        match i.checked_sub(1).map(|i| ranges[i]) {
            Some([first, count, id, _]) if primitive - first < count => id,
            _ => ID_UNTAGGED,
        }
    }
}

/// What an engine keeps for ids: the scene's object ids, the frame's tables, and (until
/// views exist, docs/design-views.md) the handle of its one view and that view's outputs.
pub(crate) struct EngineIds {
    pub objects: ObjectIds,
    pub tables: TagTables,
    pub view: ViewId,
    pub outputs: Outputs,
    pub mode: crate::ViewMode,
}
impl Default for EngineIds {
    fn default() -> Self {
        let mut views =
            crate::engine::ids::Slots::<crate::engine::kind::View, ()>::new("unknown view");
        Self {
            objects: ObjectIds::default(),
            tables: TagTables::default(),
            view: views.insert(()),
            outputs: Outputs::default(),
            mode: crate::ViewMode::Lit,
        }
    }
}
impl EngineIds {
    /// The one view until `create_view` exists; any other handle is stale.
    pub fn check(&self, view: ViewId) -> Result<(), Error> {
        if view == self.view {
            Ok(())
        } else {
            Err(Error::InvalidInput("unknown view"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::kind;

    fn objects(n: usize) -> Vec<ObjectId> {
        let mut slots = crate::engine::ids::Slots::<kind::Object, ()>::new("object");
        (0..n).map(|_| slots.insert(())).collect()
    }

    #[test]
    fn tables_follow_draw_order_and_look_up_primitive_ranges() {
        let o = objects(4);
        let mut ids = ObjectIds::default();
        ids.set(o[0], &[(0, u32::MAX, 10)]).unwrap();
        // A merged object: two entities and a gap, given out of order.
        ids.set(o[1], &[(8, 4, 21), (0, 4, 20)]).unwrap();
        ids.set(o[2], &[(0, 1, 30)]).unwrap();
        let mut tables = TagTables::default();
        // Batches: opaque [0, 1], transparent [2], opaque [3] (untagged).
        ids.tables(
            [(false, &o[0..2]), (true, &o[2..3]), (false, &o[3..4])].into_iter(),
            &mut tables,
        );
        assert_eq!(tables.opaque.len(), 3);
        assert_eq!(tables.transparent.len(), 1);
        let [a, b, untagged] = [tables.opaque[0], tables.opaque[1], tables.opaque[2]];
        assert_eq!(tables.lookup(a, Some(123_456)), 10);
        let merged: Vec<u32> = (0..13).map(|p| tables.lookup(b, Some(p))).collect();
        assert_eq!(merged[..4], [20; 4]);
        assert_eq!(merged[4..8], [ID_UNTAGGED; 4]);
        assert_eq!(merged[8..12], [21; 4]);
        assert_eq!(merged[12], ID_UNTAGGED);
        assert_eq!(tables.lookup(b, None), 20);
        assert_eq!(tables.lookup(untagged, Some(0)), ID_UNTAGGED);
        assert_eq!(tables.lookup(tables.transparent[0], None), 30);
    }

    #[test]
    fn ranges_reject_reserved_ids_overlaps_and_empty_ranges() {
        let o = objects(1)[0];
        let mut ids = ObjectIds::default();
        assert!(ids.set(o, &[(0, 4, ID_NONE)]).is_err());
        assert!(ids.set(o, &[(0, 4, ID_UNTAGGED)]).is_err());
        assert!(ids.set(o, &[(0, 0, 5)]).is_err());
        assert!(ids.set(o, &[(4, 4, 5), (0, 5, 6)]).is_err());
        ids.set(o, &[(4, 4, 5), (0, 4, 6)]).unwrap();
        ids.set(o, &[]).unwrap();
        assert!(ids.ids.is_empty());
    }

    #[test]
    fn rectangles_are_bounded_and_inside_the_view() {
        assert!(IdRect::pixel(9, 9).check(10, 10).is_ok());
        assert!(IdRect::pixel(10, 9).check(10, 10).is_err());
        assert!(IdRect { x: 0, y: 0, width: 0, height: 1 }.check(10, 10).is_err());
        let big = IdRect { x: 0, y: 0, width: MAX_ID_RECT + 1, height: 1 };
        assert!(big.check(4096, 4096).is_err());
    }
}
