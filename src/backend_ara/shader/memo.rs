//! Reflection results kept for the life of the process.
//!
//! Reflecting a program parses and validates its whole WGSL, and a frame asks for the
//! same programs every time it is built. Each result is a pure function of its key, so
//! it is computed once: by pass alone where the pass fixes its source ([`Memo<Pass, _>`]),
//! by pass and the source's digest for caller-authored programs. Failures are not kept.
//! Entries are never dropped: one per pass, and one per distinct program source a
//! process has reflected.
use anyhow::Result;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    hash::Hash,
    sync::{Mutex, OnceLock},
};

/// One memo table.
pub(super) struct Memo<K, T>(OnceLock<Mutex<HashMap<K, T>>>);
impl<K: Eq + Hash, T: Clone> Memo<K, T> {
    pub(super) const fn new() -> Self {
        Self(OnceLock::new())
    }
    /// The kept result for `key`, or `make`'s, kept if it succeeds.
    pub(super) fn get(&self, key: K, make: impl FnOnce() -> Result<T>) -> Result<T> {
        let table = self.0.get_or_init(Default::default);
        if let Some(kept) = table.lock().expect("memo lock").get(&key) {
            return Ok(kept.clone());
        }
        let made = make()?;
        table.lock().expect("memo lock").insert(key, made.clone());
        Ok(made)
    }
}

/// A source text's identity in these tables and in the pipeline cache's keys.
pub(crate) fn digest(source: &str) -> [u8; 32] {
    Sha256::digest(source.as_bytes()).into()
}
