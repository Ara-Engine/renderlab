//! Whole-resource scheduling for the first leaf passes; one graphics queue.
use anyhow::{Result, ensure};
use ara_gpu::{Access, Barrier, Resource, commands::Recorder};

/// Persistent scheduled states. Import actual states when sharing external resources.
/// Submit recorded commands in order; discard this tracker if a recorded list is abandoned.
#[derive(Default)]
pub struct Graph {
    states: Vec<(Resource, Access)>,
}
impl Graph {
    /// Remove a retired resource's scheduled state after its last list is submitted.
    pub fn forget(&mut self, resource: Resource) {
        self.states.retain(|(r, _)| *r != resource);
    }
    /// Declare an external resource's state, or initialize a fresh resource as Undefined.
    pub fn import(&mut self, resource: Resource, access: Access) {
        if let Some((_, state)) = self.states.iter_mut().find(|(r, _)| *r == resource) {
            *state = access;
        } else {
            self.states.push((resource, access));
        }
    }
    /// Attachment loads require an earlier defined producer or explicit import.
    pub fn require_initialized(&self, resource: Resource) -> Result<()> {
        ensure!(
            self.states
                .iter()
                .any(|(r, a)| *r == resource && *a != Access::Undefined),
            "attachment load has no initialized producer"
        );
        Ok(())
    }
    /// Validate a node's declared uses and batch its RAW, WAR and WAW dependencies.
    /// Simultaneous aliases are accepted only for the same read role.
    pub fn node(&mut self, recorder: &mut Recorder, uses: &[(Resource, Access)]) -> Result<()> {
        let mut unique = Vec::with_capacity(uses.len());
        for &(resource, access) in uses {
            ensure!(
                access != Access::Undefined,
                "a node must declare a real resource use"
            );
            if let Some((_, previous)) = unique.iter().find(|(r, _)| *r == resource) {
                ensure!(
                    *previous == access && !access.writes(),
                    "incompatible aliases in one node"
                );
            } else {
                unique.push((resource, access));
            }
        }
        // Validate before mutating either the recorder or retained states.
        for &(resource, after) in &unique {
            let before = self
                .states
                .iter()
                .find(|(r, _)| *r == resource)
                .map(|(_, a)| *a)
                .unwrap_or(Access::Undefined);
            ensure!(
                before != Access::Undefined || after.writes(),
                "read of an undefined resource"
            );
        }
        let mut barriers = Vec::new();
        for (resource, after) in unique {
            let before = self
                .states
                .iter()
                .find(|(r, _)| *r == resource)
                .map(|(_, a)| *a)
                .unwrap_or(Access::Undefined);
            if before != after || before.writes() || after.writes() {
                barriers.push(Barrier {
                    resource,
                    before,
                    after,
                    handoff: None,
                });
            }
            self.import(resource, after);
        }
        recorder.barriers(barriers);
        Ok(())
    }
}
