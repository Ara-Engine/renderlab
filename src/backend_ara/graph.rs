//! Explicit scheduled states, with mip/layer cells where views require them.
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, Barrier, Image, Resource,
    commands::Recorder,
    sync::{BarrierRange, ResourceBarrier},
    texture::ImageViewDesc,
};
use std::collections::HashMap;

/// Allocated subresource counts; 3D depth slices belong to one array layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Subresources {
    /// Number of allocated mip levels.
    pub mips: u32,
    /// Number of allocated array layers.
    pub layers: u32,
}
/// One declared use, either whole or limited to a published image view.
#[derive(Clone, Copy)]
pub enum ResourceUse {
    /// Existing whole buffer/image declaration.
    Whole(Resource, Access),
    /// Exact mip/layer interval of an image allocation.
    Image {
        /// Native allocation identity.
        image: Image,
        /// Full allocation bounds, independent of the published view.
        allocation: Subresources,
        /// Published view bounds; dimensionality does not alter synchronization.
        view: ImageViewDesc,
        /// Access inside the node.
        access: Access,
    },
}
impl From<(Resource, Access)> for ResourceUse {
    fn from((resource, access): (Resource, Access)) -> Self {
        Self::Whole(resource, access)
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    resource: Resource,
    cell: Option<(u32, u32)>,
}
/// One resource's scheduled states: its whole state, or one per mip/layer cell once a
/// view registered its cells (a cell without its own state reads the whole one).
type States = Vec<(Option<(u32, u32)>, Access)>;
/// Persistent scheduled states. Import actual states for external resources.
/// Submit lists in order; discard this tracker if a recorded list is abandoned.
/// States are found by resource, so planning a node costs its own uses, not the
/// number of resources the frame has declared.
#[derive(Default)]
pub struct Graph {
    states: HashMap<Resource, States>,
    images: HashMap<Image, Subresources>,
}
impl Graph {
    /// Remove retired resources and their registered subresource bounds.
    pub fn forget(&mut self, resource: Resource) {
        self.states.remove(&resource);
        if let Resource::Image(image) = resource {
            self.images.remove(&image);
        }
    }
    /// Import a whole resource state. Already registered images update every cell.
    pub fn import(&mut self, resource: Resource, access: Access) {
        match self.states.get_mut(&resource) {
            Some(states) => states.iter_mut().for_each(|(_, state)| *state = access),
            None => {
                self.states.insert(resource, vec![(None, access)]);
            }
        }
    }
    /// Whole attachment loads require every registered cell to have a producer.
    pub fn require_initialized(&self, resource: Resource) -> Result<()> {
        let states = self
            .states
            .get(&resource)
            .filter(|states| !states.is_empty());
        ensure!(
            states.is_some_and(|states| states.iter().all(|(_, s)| *s != Access::Undefined)),
            "attachment load has no initialized producer"
        );
        Ok(())
    }
    fn before(&self, key: Key) -> Access {
        let Some(states) = self.states.get(&key.resource) else {
            return Access::Undefined;
        };
        states
            .iter()
            .find(|(cell, _)| *cell == key.cell)
            .or_else(|| states.iter().find(|(cell, _)| cell.is_none()))
            .map_or(Access::Undefined, |(_, a)| *a)
    }
    /// Whole-resource compatibility path. Once an image has view states, whole
    /// uses expand to all its cells, preserving the same layout accounting.
    pub fn node(&mut self, rec: &mut Recorder, uses: &[(Resource, Access)]) -> Result<()> {
        self.node_uses(
            rec,
            &uses
                .iter()
                .copied()
                .map(ResourceUse::from)
                .collect::<Vec<_>>(),
        )
    }
    /// Validate exact aliases and batch RAW/WAR/WAW dependencies. Disjoint image
    /// mips/layers can be read and written in one node. Identical shader-storage
    /// uses deduplicate; incompatible roles and overlapping attachment writes fail.
    /// Invalid declarations leave both the tracker and recorder unchanged.
    pub fn node_uses(&mut self, rec: &mut Recorder, uses: &[ResourceUse]) -> Result<()> {
        rec.barrier_ranges(self.plan(uses)?);
        Ok(())
    }
    /// Validate a node and advance scheduled states, returning its exact barriers.
    /// Record these dependencies before executing the node. Errors change no state.
    pub fn plan(&mut self, uses: &[ResourceUse]) -> Result<Vec<ResourceBarrier>> {
        let mut registered: Vec<(Image, Subresources)> = Vec::new();
        for use_ in uses {
            if let ResourceUse::Image {
                image,
                allocation,
                view,
                ..
            } = *use_
            {
                ensure!(
                    allocation.mips > 0 && allocation.layers > 0,
                    "empty image allocation"
                );
                ensure!(
                    view.mip_count > 0
                        && view.layer_count > 0
                        && view
                            .base_mip
                            .checked_add(view.mip_count)
                            .is_some_and(|n| n <= allocation.mips)
                        && view
                            .base_layer
                            .checked_add(view.layer_count)
                            .is_some_and(|n| n <= allocation.layers),
                    "image view exceeds allocation"
                );
                if let Some(old) = self
                    .images
                    .get(&image)
                    .or_else(|| registered.iter().find(|(i, _)| *i == image).map(|(_, s)| s))
                {
                    ensure!(*old == allocation, "image allocation bounds changed");
                } else {
                    registered.push((image, allocation));
                }
            }
        }
        // The node's distinct uses in declaration order, and where each key is.
        let mut unique = Vec::<(Key, Access)>::new();
        let mut at = HashMap::<Key, usize>::new();
        for use_ in uses {
            let (resource, access, range) = match *use_ {
                ResourceUse::Whole(resource, access) => {
                    let range = if let Resource::Image(image) = resource {
                        self.images
                            .get(&image)
                            .or_else(|| {
                                registered.iter().find(|(i, _)| *i == image).map(|(_, s)| s)
                            })
                            .map(|s| (0, s.mips, 0, s.layers))
                    } else {
                        None
                    };
                    (resource, access, range)
                }
                ResourceUse::Image {
                    image,
                    view,
                    access,
                    ..
                } => (
                    Resource::Image(image),
                    access,
                    Some((
                        view.base_mip,
                        view.mip_count,
                        view.base_layer,
                        view.layer_count,
                    )),
                ),
            };
            ensure!(
                access != Access::Undefined,
                "a node must declare a real resource use"
            );
            let mut add = |cell| -> Result<()> {
                let key = Key { resource, cell };
                if let Some((_, old)) = at.get(&key).map(|i| &mut unique[*i]) {
                    if matches!(resource, Resource::Buffer(_))
                        && matches!(
                            (*old, access),
                            (Access::ShaderRead, Access::Indirect)
                                | (Access::Indirect, Access::ShaderRead)
                        )
                    {
                        // Both native backends already include shader reads in
                        // Indirect. One GPU-produced args record can serve both.
                        *old = Access::Indirect;
                    } else {
                        ensure!(
                            *old == access && (!access.writes() || access == Access::Storage),
                            "incompatible aliases in one node"
                        );
                    }
                } else {
                    at.insert(key, unique.len());
                    unique.push((key, access));
                }
                Ok(())
            };
            if let Some((mip, count, layer, layers)) = range {
                for m in mip..mip + count {
                    for l in layer..layer + layers {
                        add(Some((m, l)))?;
                    }
                }
            } else {
                add(None)?;
            }
        }
        for &(key, after) in &unique {
            ensure!(
                self.before(key) != Access::Undefined || after.writes(),
                "read of an undefined resource"
            );
        }
        // Registration follows all validation, inheriting any prior whole state.
        for (image, shape) in registered {
            let resource = Resource::Image(image);
            let before = self.before(Key {
                resource,
                cell: None,
            });
            let cells = (0..shape.mips)
                .flat_map(|mip| (0..shape.layers).map(move |layer| (Some((mip, layer)), before)))
                .collect();
            self.states.insert(resource, cells);
            self.images.insert(image, shape);
        }
        let mut barriers = Vec::new();
        for (key, after) in unique {
            let before = self.before(key);
            if before != after || before.writes() || after.writes() {
                barriers.push(ResourceBarrier {
                    barrier: Barrier {
                        resource: key.resource,
                        before,
                        after,
                        handoff: None,
                    },
                    range: key.cell.map_or(BarrierRange::Whole, |(mip, layer)| {
                        BarrierRange::Image {
                            base_mip: mip,
                            mip_count: 1,
                            base_layer: layer,
                            layer_count: 1,
                        }
                    }),
                });
            }
            let states = self.states.entry(key.resource).or_default();
            match states.iter_mut().find(|(cell, _)| *cell == key.cell) {
                Some((_, state)) => *state = after,
                None => states.push((key.cell, after)),
            }
        }
        Ok(barriers)
    }
}
