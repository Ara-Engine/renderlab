//! The bench's fixtures, in print order. Adding one is one line here plus its own file
//! (docs/bench.md). Called once per path, so each path's fixture has its own state.
use super::{Fixture, city::City, dump::Dump, furnace::Furnace, glass::Glass, reflect::Reflect, shadow_edges::ShadowEdges, tr1_edge::Tr1Edge};

pub(crate) fn fixtures() -> Vec<Box<dyn Fixture>> {
    vec![
        Box::new(Furnace::walk()),
        Box::new(Furnace::walk_wrap()),
        Box::new(Furnace::grey()),
        Box::new(Furnace::orange()),
        Box::new(Furnace::slab()),
        Box::new(Furnace::slab_roof()),
        Box::new(ShadowEdges::default()),
        Box::new(Reflect::default()),
        Box::new(Glass::default()),
        Box::new(Tr1Edge::default()),
        Box::new(City::boxes()),
        Box::new(City::terrain()),
        Box::new(Dump::glint()),
        Box::new(Dump::glint_course()),
        Box::new(Dump::outpost()),
        Box::new(Dump::glint_play()),
    ]
}

/// Fixtures that run only when `--fixtures` names them: the real-size scenes take about a
/// second a round each (`glint`, a scene dump from disk, several), outside the default loop's
/// budget.
pub(crate) const NAMED_ONLY: &[&str] = &["city", "terrain", "glint", "glint-course", "outpost", "glint-play"];
