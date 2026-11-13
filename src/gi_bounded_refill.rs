//! Opt-in, persistent software refill budgets, shared by both renderer backends.
use super::*;
pub(super) const HEADER_LIMIT: usize = 32;

pub(super) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_GI_BOUNDED_REFILL").is_ok_and(|v| v == "1"))
}

pub(super) fn settings(mut s: IndirectSettings, enabled: bool) -> IndirectSettings {
    if enabled {
        s.cut_refill = true;
        if s.cut_gather_ms == 0.0 {
            s.cut_gather_ms = 3.0;
        }
        s.parent_standin = true;
    }
    s
}

pub(super) fn gather_source(source: String, enabled: bool) -> String {
    if !enabled {
        return source;
    }
    let needle = "if queued && gi_gather_now(slot,period,count) {";
    assert_eq!(source.matches(needle).count(), 1);
    let mut source = source.replacen(
        needle,
        "if queued && gi_gather_now(slot,period,count) && gi_bounded_gather_admit() {",
        1,
    );
    source.push_str(include_str!("../shaders/gi_bounded_refill.wgsl"));
    source
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(software: bool) -> PlanConfig {
        PlanConfig {
            settings: IndirectSettings::default(),
            slots: 1 << 20,
            misses: 1 << 16,
            cold_words: 4,
            raw_slots: 1 << 20,
            software,
            field: false,
            profiling: false,
            near_bins: 0,
            eye: Vec3::ZERO,
            fov_y: 1.0,
            size: [1920, 1080],
        }
    }

    #[test]
    fn bounded_refill_survives_the_old_cut_windows_without_coarse_or_partial_sky() {
        for software in [false, true] {
            let c = config(software);
            let mut ordinary = Planner::default();
            let mut bounded = Planner::default();
            ordinary.camera_cut();
            bounded.camera_cut();
            for frame in 0..=128 {
                let off = ordinary.begin_bounded(&c, true, true, true, false);
                let on = bounded.begin_bounded(&c, true, true, true, true);
                if software {
                    assert_eq!(on.header[20], 3000, "gather budget at {frame}");
                    assert_eq!(on.header[32], 3000, "hard ceiling at {frame}");
                    assert_eq!(on.header[22], CLAIM_CELLS_SOFTWARE, "claims at {frame}");
                    assert_eq!(on.header[4], off.header[4], "no coarse read at {frame}");
                    assert_eq!(on.header[14], 0, "full sky directions at {frame}");
                } else {
                    assert_eq!(on.header, off.header, "hardware unchanged at {frame}");
                }
                ordinary.advance(&off, false);
                bounded.advance(&on, false);
            }
        }
        let s = IndirectSettings::default();
        assert_eq!(settings(s, false), s);
        assert_eq!(
            gather_source("original module".to_owned(), false),
            "original module"
        );
        let mut c = config(true);
        c.settings.gather_cells = 1000;
        c.settings.cut_gather_ms = 2.0;
        let bounded = Planner::default().begin_bounded(&c, true, true, true, true);
        assert_eq!(
            bounded.header[20], 1000,
            "keep a smaller explicit cell budget"
        );
        assert_eq!(bounded.header[32], 1000);
    }

    #[test]
    fn bounded_software_kernels_validate_at_both_direction_counts() {
        assert!(HEADER_LIMIT < HEADER_WORDS as usize);
        assert!(47 < COUNTER_WORDS);
        for directions in [64, 128] {
            assert_eq!(
                kernel_source_bounded(false, directions, false),
                kernel_source_bounded(false, directions, true),
                "the hardware module stays byte-identical"
            );
            let source = kernel_source_bounded(true, directions, true);
            let module = naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }
}
