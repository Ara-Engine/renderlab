//! Explicit scheduled states, with mip/layer cells where views require them.
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, Barrier, Image, Resource,
    commands::{Command, Recorder},
    sync::{BarrierRange, ResourceBarrier},
    texture::ImageViewDesc,
};

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
#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    resource: Resource,
    cell: Option<(u32, u32)>,
}
impl Key {
    fn whole(resource: Resource) -> Self {
        Self {
            resource,
            cell: None,
        }
    }
}
/// Persistent scheduled states. Import actual states for external resources.
/// Submit lists in order; discard this tracker if a recorded list is abandoned.
#[derive(Default)]
pub struct Graph {
    states: Vec<(Key, Access)>,
    images: Vec<(Image, Subresources)>,
}
impl Graph {
    /// Remove retired resources and their registered subresource bounds.
    pub fn forget(&mut self, resource: Resource) {
        self.states.retain(|(k, _)| k.resource != resource);
        if let Resource::Image(image) = resource {
            self.images.retain(|(i, _)| *i != image);
        }
    }
    /// Import a whole resource state. Already registered images update every cell.
    pub fn import(&mut self, resource: Resource, access: Access) {
        let mut found = false;
        for (key, state) in &mut self.states {
            if key.resource == resource {
                *state = access;
                found = true;
            }
        }
        if !found {
            self.states.push((Key::whole(resource), access));
        }
    }
    /// Whole attachment loads require every registered cell to have a producer.
    pub fn require_initialized(&self, resource: Resource) -> Result<()> {
        let mut found = false;
        for (key, state) in &self.states {
            if key.resource == resource {
                found = true;
                ensure!(
                    *state != Access::Undefined,
                    "attachment load has no initialized producer"
                );
            }
        }
        ensure!(found, "attachment load has no initialized producer");
        Ok(())
    }
    fn before(&self, key: Key) -> Access {
        self.states
            .iter()
            .find(|(k, _)| *k == key)
            .or_else(|| {
                self.states
                    .iter()
                    .find(|(k, _)| *k == Key::whole(key.resource))
            })
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
    /// mips/layers can be read and written in one node; overlapping writes cannot.
    /// Invalid declarations leave both the tracker and recorder unchanged.
    pub fn node_uses(&mut self, rec: &mut Recorder, uses: &[ResourceUse]) -> Result<()> {
        let mut registered = Vec::new();
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
                if let Some((_, old)) = self
                    .images
                    .iter()
                    .chain(&registered)
                    .find(|(i, _)| *i == image)
                {
                    ensure!(*old == allocation, "image allocation bounds changed");
                } else {
                    registered.push((image, allocation));
                }
            }
        }
        let mut unique = Vec::<(Key, Access)>::new();
        for use_ in uses {
            let (resource, access, range) = match *use_ {
                ResourceUse::Whole(resource, access) => {
                    let range = if let Resource::Image(image) = resource {
                        self.images
                            .iter()
                            .chain(&registered)
                            .find(|(i, _)| *i == image)
                            .map(|(_, s)| (0, s.mips, 0, s.layers))
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
                if let Some((_, old)) = unique.iter().find(|(k, _)| *k == key) {
                    ensure!(
                        *old == access && !access.writes(),
                        "incompatible aliases in one node"
                    );
                } else {
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
            let before = self.before(Key::whole(resource));
            self.states.retain(|(k, _)| k.resource != resource);
            for mip in 0..shape.mips {
                for layer in 0..shape.layers {
                    self.states.push((
                        Key {
                            resource,
                            cell: Some((mip, layer)),
                        },
                        before,
                    ));
                }
            }
            self.images.push((image, shape));
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
            if let Some((_, state)) = self.states.iter_mut().find(|(k, _)| *k == key) {
                *state = after;
            } else {
                self.states.push((key, after));
            }
        }
        if !barriers.is_empty() {
            rec.push(Command::Barriers(barriers));
        }
        Ok(())
    }
}
