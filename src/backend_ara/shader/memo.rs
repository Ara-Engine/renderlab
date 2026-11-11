//! Reflection results kept for the life of the process.
//!
//! Reflecting a program parses and validates its whole WGSL, and a frame asks for the
//! same programs every time it is built. Each result is a pure function of the pass and
//! the source text, so it is computed once and looked up by the pass and the source's
//! digest. Failures are not kept. Entries are never dropped: one per distinct program
//! source a process has reflected.
use super::Pass;
use anyhow::Result;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

type Table<T> = Mutex<HashMap<(Pass, [u8; 32]), T>>;
/// One memo table: results by pass and source digest.
pub(super) struct Memo<T>(OnceLock<Table<T>>);
impl<T: Clone> Memo<T> {
    pub(super) const fn new() -> Self {
        Self(OnceLock::new())
    }
    /// The kept result for `pass` and `source`, or `make`'s, kept if it succeeds.
    pub(super) fn get(
        &self,
        pass: Pass,
        source: &str,
        make: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let key = (pass, Sha256::digest(source.as_bytes()).into());
        let table = self.0.get_or_init(Default::default);
        if let Some(kept) = table.lock().expect("memo lock").get(&key) {
            return Ok(kept.clone());
        }
        let made = make()?;
        table.lock().expect("memo lock").insert(key, made.clone());
        Ok(made)
    }
}
