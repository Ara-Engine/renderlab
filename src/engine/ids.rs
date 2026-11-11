//! Typed generational handles and the slot storage behind them.
use crate::Error;
use std::{fmt, hash, marker::PhantomData};

/// A copyable handle to one engine-owned resource of kind `T`. A handle stays valid until
/// its resource is removed; after that, every call that takes it returns
/// `Error::InvalidInput` instead of reaching a reused slot. Handles are only meaningful to
/// the [`Engine`](super::Engine) that issued them.
pub struct Id<T> {
    index: u32,
    generation: u32,
    kind: PhantomData<fn() -> T>,
}
impl<T> Id<T> {
    /// The slot index, stable for the handle's lifetime: a dense key for engine-side
    /// tables. Indices are reused after removal; the full handle is not.
    pub fn index(self) -> u32 {
        self.index
    }
}
impl<T> Clone for Id<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Id<T> {}
impl<T> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        (self.index, self.generation) == (other.index, other.generation)
    }
}
impl<T> Eq for Id<T> {}
impl<T> hash::Hash for Id<T> {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        (self.index, self.generation).hash(state);
    }
}
impl<T> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = std::any::type_name::<T>()
            .rsplit("::")
            .next()
            .unwrap_or("?");
        write!(f, "{name}#{}v{}", self.index, self.generation)
    }
}

/// Kind markers for [`Id`]; they carry no data.
pub mod kind {
    pub enum Mesh {}
    pub enum Texture {}
    pub enum Program {}
    pub enum Material {}
    pub enum Environment {}
    pub enum Object {}
    pub enum Emitter {}
    /// A view: one camera's targets and histories (docs/design-views.md).
    pub enum View {}
    /// The engine's internal mesh-material draw groups.
    pub(crate) enum Batch {}
}
pub type MeshId = Id<kind::Mesh>;
pub type TextureId = Id<kind::Texture>;
pub type ProgramId = Id<kind::Program>;
pub type MaterialId = Id<kind::Material>;
pub type EnvironmentId = Id<kind::Environment>;
pub type ObjectId = Id<kind::Object>;
pub type EmitterId = Id<kind::Emitter>;
pub type ViewId = Id<kind::View>;

/// Dense storage with a free list; each reuse of a slot bumps its generation.
pub(crate) struct Slots<K, V> {
    entries: Vec<(u32, Option<V>)>,
    free: Vec<u32>,
    what: &'static str,
    kind: PhantomData<fn() -> K>,
}
impl<K, V> Slots<K, V> {
    /// `what` is the error message for stale handles, e.g. "unknown or removed mesh".
    pub fn new(what: &'static str) -> Self {
        Self {
            entries: Vec::new(),
            free: Vec::new(),
            what,
            kind: PhantomData,
        }
    }
    pub fn insert(&mut self, value: V) -> Id<K> {
        let index = match self.free.pop() {
            Some(index) => {
                let entry = &mut self.entries[index as usize];
                entry.0 = entry.0.wrapping_add(1);
                entry.1 = Some(value);
                index
            }
            None => {
                self.entries.push((0, Some(value)));
                (self.entries.len() - 1) as u32
            }
        };
        Id {
            index,
            generation: self.entries[index as usize].0,
            kind: PhantomData,
        }
    }
    pub fn get(&self, id: Id<K>) -> Result<&V, Error> {
        match self.entries.get(id.index as usize) {
            Some((generation, Some(value))) if *generation == id.generation => Ok(value),
            _ => Err(Error::InvalidInput(self.what)),
        }
    }
    pub fn get_mut(&mut self, id: Id<K>) -> Result<&mut V, Error> {
        match self.entries.get_mut(id.index as usize) {
            Some((generation, Some(value))) if *generation == id.generation => Ok(value),
            _ => Err(Error::InvalidInput(self.what)),
        }
    }
    pub fn remove(&mut self, id: Id<K>) -> Result<V, Error> {
        self.get(id)?;
        let value = self.entries[id.index as usize].1.take().unwrap();
        self.free.push(id.index);
        Ok(value)
    }
    pub fn len(&self) -> usize {
        self.entries.len() - self.free.len()
    }
    /// Live handles in slot order.
    pub fn ids(&self) -> impl Iterator<Item = Id<K>> + '_ {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, (_, value))| value.is_some())
            .map(|(index, (generation, _))| Id {
                index: index as u32,
                generation: *generation,
                kind: PhantomData,
            })
    }
    /// Live values in slot order.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.entries
            .iter_mut()
            .filter_map(|(_, value)| value.as_mut())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_handles_never_reach_reused_slots() {
        let mut slots = Slots::<kind::Mesh, &str>::new("unknown or removed mesh");
        let a = slots.insert("a");
        let b = slots.insert("b");
        assert_eq!(slots.remove(a).unwrap(), "a");
        let c = slots.insert("c");
        assert_eq!(c.index(), a.index(), "the freed slot is reused");
        assert!(slots.get(a).is_err(), "the stale handle is rejected");
        assert!(slots.remove(a).is_err());
        assert_eq!(*slots.get(c).unwrap(), "c");
        assert_eq!(*slots.get(b).unwrap(), "b");
        assert_eq!(slots.len(), 2);
        assert_ne!(a, c);
    }
}
