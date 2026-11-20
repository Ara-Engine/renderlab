//! The bench's fixtures, in print order. Adding one is one line here plus its own file
//! (docs/bench.md). Called once per path, so each path's fixture has its own state.
use super::{Fixture, furnace::Furnace};

pub(crate) fn fixtures() -> Vec<Box<dyn Fixture>> {
    vec![
        Box::new(Furnace::grey()),
        Box::new(Furnace::orange()),
        Box::new(Furnace::slab()),
        Box::new(Furnace::slab_roof()),
    ]
}
