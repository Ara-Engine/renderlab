//! P-STILL (docs/p-still.md): per-frame cache work counts, shared by the P-YARD and ARA1
//! adapters. Enabled by `RENDERLAB_P_STILL_COUNT`; each frame reads the counters back, so
//! count runs are for counts, and their spans include the GI profile's empty passes.
use renderlab::engine::Engine;
use serde_json::{Value, json};

pub struct Counts {
    rows: Vec<Value>,
    gi_frames: u64,
}

impl Counts {
    /// `None` unless counting was requested; turns the GI pass profile on.
    pub fn start(engine: &mut Engine) -> Option<Self> {
        std::env::var_os("RENDERLAB_P_STILL_COUNT")?;
        engine.renderer_mut().set_indirect_profiling(true);
        Some(Self { rows: vec![], gi_frames: 0 })
    }

    /// After a frame's GPU drain: whether the GI cache ran, its counters and pass times,
    /// virtual-shadow page work, the surface cache's queued walks and TR1's shadow budget.
    pub fn frame(&mut self, engine: &mut Engine, index: usize, state: &str) -> Result<(), renderlab::Error> {
        let r = engine.renderer_mut();
        let stats = r.indirect_stats();
        let ran = stats.frames != self.gi_frames;
        self.gi_frames = stats.frames;
        let c = r.indirect_counters()?.filter(|_| ran);
        let p = r.indirect_profile()?.filter(|_| ran);
        let v = r.virtual_shadow_stats();
        let walks = r.p_still_surface_walks();
        let rays = r.reconstruction_shadow_rays();
        self.rows.push(json!({"index":index,"state":state,"gi_ran":ran,
            "gi_idle_frames":stats.idle_frames,"gi_bounce_frames":stats.bounce_frames,
            "gi":c.map(|c| json!({"misses":c.misses,"traces":c.traces,"integrates":c.integrates,
                "created":c.created,"invalidated":c.invalidated,"refreshed":c.refreshed,"rays":c.rays,
                "shades":c.shades,"gathers":c.gathers,"gather_rays":c.gather_rays,
                "light_rays":c.light_rays,"bounce_queued":c.bounce_queued,"full":c.full,"lost":c.lost})),
            "gi_ms":p.map(|p| json!({"total":p.total(),"probe":p.probe,"claim":p.claim+p.resolve,
                "scan":p.scan,"trace":p.trace(),"integrate":p.integrate,"gather":p.gather,
                "light":p.light,"shade":p.shade,"filter":p.filter})),
            "vsm_page_work":v.page_work,"vsm_moved_rects":v.moved_rects,
            "vsm_invalidated_levels":v.invalidated_levels,"s4_frame":v.occluder_gather_frame,
            "surface_walks":walks,"shadow_rays":rays}));
        Ok(())
    }

    pub fn finish(self) -> Value {
        json!({"p_still":std::env::var("RENDERLAB_P_STILL").unwrap_or_default(),"frames":self.rows})
    }
}
