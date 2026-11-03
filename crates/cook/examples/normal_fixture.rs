use renderlab_cook::{Error, normal_fixture};
use std::path::PathBuf;

fn main() -> Result<(), Error> {
    let directory = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| {
            Error(
                "usage: cargo run -p renderlab-cook --example normal_fixture -- <output-directory>"
                    .into(),
            )
        })?;
    normal_fixture::generate(&directory)
}
