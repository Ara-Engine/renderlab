//! The shading stage's parts in the pass log (`RENDERLAB_PASS_LOG_DETAIL`,
//! src/pass_timeline.rs). Shading is one dispatch, so timestamps cannot split it. Under
//! detail, a traced frame first replays that dispatch from modules that leave
//! terms out (shaders/program_api.wgsl's `RL_SHADE_TERMS`), each marked as its own dotted
//! part, and then shades as always: the picture and the `shading` time are the frame's own.
//! A term costs `shading` minus `shading.without_<term>`. The terms overlap (reflection hits
//! run the light loop and its shadows), so those costs need not sum to `shading`;
//! `shading.base` keeps none of them. Mixed-program frames replay their tile lists (the
//! binning pass included, as in `shading`); view-mode frames log no parts.
use super::program_gpu::{module, scope_error};
use super::*;
use crate::program::{self, Stage};

/// The line of shaders/program_api.wgsl that each part's module rewrites.
const ALL_TERMS: &str = "const RL_SHADE_TERMS:u32=0xffffffffu;";
/// Each part's pass-log name and the RL_TERM_* bits it keeps.
pub(crate) const PARTS: [(&str, u32); 7] = [
    ("shading.without_lights", !1),
    ("shading.without_shadows", !2),
    ("shading.without_transmission", !4),
    ("shading.without_reflections", !8),
    ("shading.without_textures", !16),
    ("shading.without_brdf", !32),
    ("shading.base", 0),
];

/// The parts' pipelines for one frame's traced shading: by part, then by the frame's
/// program (one for a single-program frame, else its list programs in slot order). Kept
/// while the composed modules (programs, visibility head, streaming stage) stay the same.
pub(super) struct ShadeDetail {
    programs: Vec<u64>,
    sources: Vec<String>,
    streamed: bool,
    shading: usize,
    pipelines: Vec<Vec<Owned<wgpu::ComputePipeline>>>,
}

/// Whether the pass log keeps parts (`RENDERLAB_PASS_LOG_DETAIL`), read once: the Ara
/// backend, which has no log of its own, names its part dispatches by it.
pub(crate) fn wanted() -> bool {
    static WANTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *WANTED.get_or_init(|| std::env::var_os("RENDERLAB_PASS_LOG_DETAIL").is_some())
}

/// `source` with only the `terms` bits of RL_SHADE_TERMS kept.
pub(crate) fn without_terms(source: &str, terms: u32) -> String {
    debug_assert_eq!(source.matches(ALL_TERMS).count(), 1);
    source.replacen(ALL_TERMS, &format!("const RL_SHADE_TERMS:u32={terms}u;"), 1)
}

impl Renderer {
    /// Builds the parts for a frame shading `programs` (its distinct programs in slot order;
    /// several shade by tile list) with `Shading` index `shading`, streamed or not, when the
    /// pass log keeps detail and they are missing or stale; drops them otherwise.
    pub(super) fn ensure_shade_detail(
        &mut self,
        programs: &[u64],
        streamed: bool,
        shading: usize,
    ) -> Result<(), Error> {
        let detail = self.pass_timeline.as_ref().is_some_and(|t| t.detail());
        let Some((_, heads)) = self.primary_sources.as_ref().filter(|_| detail) else {
            self.shade_detail = None;
            return Ok(());
        };
        let standard = [self.standard.id()];
        let programs = if programs.is_empty() {
            &standard[..]
        } else {
            programs
        };
        let list = programs.len() > 1;
        let stage = if streamed {
            Stage::TracedStreamed
        } else {
            Stage::Traced
        };
        let full: Vec<_> = programs
            .iter()
            .map(|&id| {
                let source = &self.program_slot(id).source;
                let parameters = program::declares_parameters(source);
                if list {
                    program::compose_list(&heads[shading], source, stage, parameters)
                } else {
                    program::compose(&heads[shading], source, stage, parameters)
                }
            })
            .collect();
        if self.shade_detail.as_ref().is_some_and(|d| {
            d.programs == programs
                && d.streamed == streamed
                && d.shading == shading
                && d.sources.iter().eq(full.iter().map(|c| &c.source))
        }) {
            return Ok(());
        }
        let abi = self.program_abi();
        let primary = abi.primary.unwrap();
        let mut pipelines = vec![];
        for &(_, terms) in &PARTS {
            let mut part = vec![];
            for (&id, full) in programs.iter().zip(&full) {
                let label = &self.program_slot(id).label;
                let composed = [program::Composed {
                    source: without_terms(&full.source, terms),
                    program: full.program.clone(),
                }];
                let scopes = OperationScopes::new(&self.device);
                let module = module(&abi, label, &composed[0]);
                part.push(if list {
                    primary.list_pipeline(
                        abi.device, abi.ledger, &module, shading, streamed, abi.cache,
                    )
                } else if streamed {
                    primary.streamed_pipeline(abi.device, abi.ledger, &module, shading, abi.cache)
                } else {
                    primary.shade_pipeline(abi.device, abi.ledger, &module, shading, abi.cache)
                });
                if let Some(error) = scopes.finish() {
                    return Err(match scope_error(label, error, &composed) {
                        Error::Fault(Fault(message)) => self.fail(message),
                        other => other,
                    });
                }
            }
            pipelines.push(part);
        }
        self.pipelines.created += (PARTS.len() * programs.len()) as u64;
        self.shade_detail = Some(ShadeDetail {
            programs: programs.to_vec(),
            sources: full.into_iter().map(|c| c.source).collect(),
            streamed,
            shading,
            pipelines,
        });
        Ok(())
    }

    /// Replays this frame's shading dispatch once per part, each after its mark, when parts
    /// were built for exactly this frame's shading (`ensure_shade_detail`). The frame's own
    /// dispatch follows and rewrites every pixel, so the picture is unchanged.
    pub(super) fn encode_shade_detail(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        shading: primary_ray_gpu::Shading,
        mark: &mut dyn FnMut(&mut wgpu::CommandEncoder, &'static str),
    ) {
        let state = self.view.primary.as_ref().unwrap();
        let mixed = state.mixed();
        let Some(detail) = self.shade_detail.as_ref().filter(|d| {
            d.streamed == state.requests
                && d.shading == shading as usize
                && (!mixed && d.programs == [state.program]
                    || mixed && d.programs == state.programs)
        }) else {
            return;
        };
        for (&(name, _), part) in PARTS.iter().zip(&detail.pipelines) {
            mark(encoder, name);
            if mixed {
                self.encode_program_lists_with(encoder, shading, None, |k| &*part[k]);
            } else {
                self.encode_primary_shade(
                    encoder,
                    &part[0],
                    primary_ray_gpu::SHADE_TILE,
                    shading,
                    None,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every part rewrites the one switch line of the shared API, which every traced
    /// module carries exactly once.
    #[test]
    fn each_part_rewrites_the_switch_line() {
        let api = include_str!("../shaders/program_api.wgsl");
        assert_eq!(api.matches(ALL_TERMS).count(), 1);
        let base = without_terms(api, 0);
        assert!(base.contains("const RL_SHADE_TERMS:u32=0u;"));
        assert!(!base.contains(ALL_TERMS));
    }

    /// Every part's module of the standard program validates, on both paths' heads and
    /// with and without streaming: the switches fold to valid code whichever bits are off.
    #[test]
    fn every_part_of_the_standard_program_validates() {
        const SCENE: &str = include_str!("../shaders/scene.wgsl");
        let rt = primary_ray_gpu::shading_heads(SCENE).unwrap();
        let (_, non_rt) = primary_ray_gpu::non_rt_sources(SCENE).unwrap();
        let parameters = program::declares_parameters(crate::STANDARD_PROGRAM);
        for head in rt.iter().chain(&non_rt) {
            for stage in [Stage::Traced, Stage::TracedStreamed] {
                let full = program::compose(head, crate::STANDARD_PROGRAM, stage, parameters);
                for (name, terms) in PARTS {
                    let source = without_terms(&full.source, terms);
                    let ir = naga::front::wgsl::parse_str(&source)
                        .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(&source)));
                    naga::valid::Validator::new(
                        naga::valid::ValidationFlags::all(),
                        naga::valid::Capabilities::all(),
                    )
                    .validate(&ir)
                    .unwrap_or_else(|e| panic!("{name}: {e:?}"));
                }
            }
        }
    }
}
