//! Estimator bisect switches for the example scenes (`--diag KEY=VALUE`; P-FLAT,
//! docs/p-flat.md), shared by `game_scene` and `reflection_matched`.
use renderlab::{
    IndirectLookup, IndirectSettings, Reconstruction, ReconstructionShadowBudget, SoftShadows,
    VirtualLookup, VirtualShadowSettings,
    engine::{Engine, Light, Quality, RenderPath, RenderScale, Settings, ViewLook},
};

/// Bisect switches (`--diag KEY=VALUE`; P-FLAT, docs/p-flat.md). Settings-level keys edit the
/// engine `Settings`; the rest replace one estimator's renderer settings after the engine has
/// applied its defaults, starting from the values the engine would have set. Diagnostic only:
/// the switches turn estimators off or raise their sample counts, and none is a shipping setting.
#[derive(Default)]
pub struct Diag {
    quality: Option<Quality>,
    aerial_perspective: Option<bool>,
    height_fog: Option<bool>,
    reflection_max_roughness: Option<f32>,
    indirect: Option<IndirectSettings>,
    reconstruction: Option<Reconstruction>,
    virtual_shadows: Option<VirtualShadowSettings>,
    soft_shadows: Option<SoftShadows>,
    shadow_filter: Option<bool>,
    shadow_budget: Option<ReconstructionShadowBudget>,
    /// Bit i keeps local light i (0 arch point, 1 crate spot, 2 fire point, 3+ lanterns).
    light_mask: Option<u32>,
    light_radius_scale: Option<f32>,
}
impl Diag {
    pub fn parse(keys: &[(String, String)]) -> Result<Self, Box<dyn std::error::Error>> {
        let mut d = Self::default();
        let on = |v: &str| v != "0";
        for (key, v) in keys {
            let indirect = d.indirect.get_or_insert_with(IndirectSettings::default);
            match key.as_str() {
                "quality" => {
                    d.quality = Some(match v.as_str() {
                        "realtime" => Quality::Realtime,
                        "reference" => Quality::Reference,
                        other => {
                            return Err(format!("quality realtime|reference, not {other}").into());
                        }
                    })
                }
                "aerial" => d.aerial_perspective = Some(on(v)),
                "height-fog" => d.height_fog = Some(on(v)),
                "reflect-max-roughness" => d.reflection_max_roughness = Some(v.parse()?),
                "gi-filter" => indirect.filter = v.parse()?,
                "gi-filter-edge" => indirect.filter_edge = v.parse()?,
                "gi-rotate" => indirect.rotate_directions = on(v),
                "gi-directions" => indirect.directions = v.parse()?,
                "gi-cell-pixels" => indirect.cell_pixels = v.parse()?,
                "gi-bounce" => indirect.bounce = on(v),
                "gi-lookup" => {
                    indirect.lookup = match v.as_str() {
                        "nearest" => IndirectLookup::Nearest,
                        _ => IndirectLookup::Trilinear,
                    }
                }
                "gi-level-blend" => indirect.level_blend = v.parse()?,
                "gi-gather-cells" => indirect.gather_cells = v.parse()?,
                "gi-gather-stride" => indirect.gather_stride = v.parse()?,
                "gi-light-rays" => indirect.near_light_rays = v.parse()?,
                "gi-pressure" => indirect.pressure_eviction = on(v),
                "gi-hit-cells" => indirect.hit_cone_cells = v.parse()?,
                key if key.starts_with("tr-") => {
                    let r = d.reconstruction.get_or_insert_with(Reconstruction::default);
                    match key {
                        "tr-window" => r.window = v.parse()?,
                        "tr-sharp-window" => r.sharp_window = v.parse()?,
                        "tr-blur-budget" => r.blur_budget = v.parse()?,
                        "tr-motion-box" => r.motion_box = on(v),
                        "tr-view-budget" => r.view_budget = v.parse()?,
                        "tr-light-budget" => r.light_budget = v.parse()?,
                        "tr-rotate" => r.rotate_directions = on(v),
                        "tr-catmull-rom" => r.catmull_rom = on(v),
                        other => return Err(format!("unknown diag key {other}").into()),
                    }
                }
                key if key.starts_with("vsm-") => {
                    let s = d
                        .virtual_shadows
                        .get_or_insert_with(VirtualShadowSettings::default);
                    match key {
                        "vsm-lookup" => {
                            s.lookup = match v.as_str() {
                                "exact" => VirtualLookup::Exact,
                                "map" => VirtualLookup::Map,
                                "hybrid" => VirtualLookup::Hybrid,
                                "disk" => VirtualLookup::Disk,
                                _ => VirtualLookup::Surface,
                            }
                        }
                        "vsm-walk-cap" => s.walk_cap = v.parse()?,
                        "vsm-refresh" => s.refresh = v.parse()?,
                        "vsm-directions" => s.directions = v.parse()?,
                        "vsm-march-samples" => s.march_samples = v.parse()?,
                        "vsm-caster-error" => s.caster_error_texels = v.parse()?,
                        other => return Err(format!("unknown diag key {other}").into()),
                    }
                }
                "soft-shadows" => {
                    d.soft_shadows = Some(match v.as_str() {
                        "exact" => SoftShadows::Exact,
                        _ => SoftShadows::Interactive,
                    })
                }
                "shadow-filter" => d.shadow_filter = Some(on(v)),
                "shadow-budget" => {
                    let n: Vec<u32> = v.split(',').map(str::parse).collect::<Result<_, _>>()?;
                    let [cold, reset, active, held, settle] = n[..] else {
                        return Err("shadow-budget COLD,RESET,ACTIVE,HELD,SETTLE".into());
                    };
                    d.shadow_budget = Some(ReconstructionShadowBudget {
                        cold_rays: cold,
                        reset_rays: reset,
                        active_rays: active,
                        held_rays: held,
                        settle_frames: settle,
                    });
                }
                "lights" => d.light_mask = Some(v.parse()?),
                "light-radius" => d.light_radius_scale = Some(v.parse()?),
                other => return Err(format!("unknown diag key {other}").into()),
            }
        }
        if d.indirect == Some(IndirectSettings::default()) {
            d.indirect = None;
        }
        Ok(d)
    }
    pub fn settings(&self, mut s: Settings) -> Settings {
        if let Some(aerial) = self.aerial_perspective {
            s.aerial_perspective = aerial;
        }
        if self.height_fog == Some(false) {
            s.height_fog = None;
        }
        s
    }
    pub fn look(&self, mut look: ViewLook) -> ViewLook {
        if let Some(quality) = self.quality {
            look.quality = quality;
        }
        if let Some(r) = self.reflection_max_roughness {
            look.reflections.max_roughness = r;
        }
        look
    }
    pub fn apply(
        &self,
        engine: &mut Engine,
        path: RenderPath,
        scale: RenderScale,
        quality: Quality,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let settings = engine.settings();
        let r = engine.renderer_mut();
        if let Some(indirect) = self.indirect.filter(|_| settings.global_illumination) {
            r.set_indirect(indirect)?;
        }
        if let Some(rec) = self
            .reconstruction
            .filter(|_| settings.temporal_reconstruction)
        {
            r.set_reconstruction(Reconstruction { scale, ..rec })?;
        }
        if let Some(budget) = self.shadow_budget {
            r.set_reconstruction_shadow_budget(budget)?;
        }
        match path {
            RenderPath::HardwareRt => {
                if let Some(mode) = self.soft_shadows {
                    r.set_soft_shadows(mode);
                }
                if let Some(filter) = self.shadow_filter {
                    r.set_interactive_shadow_filter(filter)?;
                }
            }
            RenderPath::NonRt => {
                if let Some(mut vsm) = self.virtual_shadows {
                    if quality == Quality::Reference {
                        vsm.lookup = VirtualLookup::Exact;
                    }
                    r.set_virtual_shadow_settings(vsm)?;
                }
            }
        }
        Ok(())
    }
    /// The frame's local lights after the mask and source-radius scale.
    pub fn lights(&self, lights: Vec<Light>) -> Vec<Light> {
        let mask = self.light_mask.unwrap_or(u32::MAX);
        let scale = self.light_radius_scale.unwrap_or(1.0);
        lights
            .into_iter()
            .enumerate()
            .filter(|(i, _)| *i >= 32 || mask & (1 << i) != 0)
            .map(|(_, mut light)| {
                if let Light::Point { source_radius, .. } | Light::Spot { source_radius, .. } =
                    &mut light
                {
                    *source_radius *= scale;
                }
                light
            })
            .collect()
    }
}
