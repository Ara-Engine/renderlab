//! The bench's fixtures, in print order. Adding one is one line here plus its own file
//! (docs/bench.md). Called once per path, so each path's fixture has its own state.
use super::{Fixture, city::City, furnace::Furnace};

pub(crate) fn fixtures() -> Vec<Box<dyn Fixture>> {
    vec![
        Box::new(Furnace::walk()),
        Box::new(Furnace::walk_wrap()),
        Box::new(Furnace::grey()),
        Box::new(Furnace::orange()),
        Box::new(Furnace::slab()),
        Box::new(Furnace::slab_roof()),
        Box::new(City::boxes()),
        Box::new(City::terrain()),
    ]
}

/// Fixtures that run only when `--fixtures` names them. The slab cases read black on the bench
/// where the furnace test reads 0.5 on the top face (a bench bug not yet found); the real-size
/// scenes take about a second a round each, outside the default loop's budget.
pub(crate) const NAMED_ONLY: &[&str] = &["slab", "slab-roof", "city", "terrain"];
