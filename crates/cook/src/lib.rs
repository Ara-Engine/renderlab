//! Bounded static GLB preparation and content-addressed publication.
mod cache;
pub mod environment;
pub mod environment_fixture;
pub mod environment_hdr;
pub mod fixtures;
mod import;
mod mip;
pub mod normal_fixture;
pub mod normal_map;
pub mod tangent;
pub mod virtual_cache;
pub mod virtual_cook;
pub mod virtual_fixtures;
pub mod virtual_proof;

pub use cache::{CookReport, CookResult, LoadedAsset, cook_file, hash_hex, load_manifest};
pub use import::{CookLimits, MissingNormals, Settings, prepare};
use std::fmt;

#[derive(Debug)]
pub struct Error(pub String);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self(e.to_string())
    }
}
impl From<renderlab_asset::Error> for Error {
    fn from(e: renderlab_asset::Error) -> Self {
        Self(e.to_string())
    }
}
pub type Result<T> = std::result::Result<T, Error>;
fn ensure(condition: bool, message: impl Into<String>) -> Result<()> {
    if !condition {
        return Err(Error(message.into()));
    }
    Ok(())
}
