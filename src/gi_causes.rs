//! The lighting cache's cause counters (the bench's cause line, src/bench/cause.rs; agreed with
//! gicut and shade, 21 Nov): cells queued for cell shading by why, once a cell, and the scan's
//! re-traces of untraced or partial cells, in work words 210-215 (`IndirectCounters::shade_causes`,
//! `trace_partial`). They are same-address atomics on the hot queue paths (Outpost queues about
//! 90,000 shades a walking frame), so default frames carry none: the lines are composed into the
//! kernels' text only when the cause log is asked for, by `RENDERLAB_GI_CAUSES=1` (the bench sets
//! it) or `RENDERLAB_GI_LOG` (the engine's counter log). Otherwise the kernels are main's text.

/// Whether this process composes the cause counters into the cache kernels.
pub(crate) fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_GI_CAUSES").is_ok_and(|v| v == "1") || std::env::var_os("RENDERLAB_GI_LOG").is_some())
}

/// The counters' constants and the counting queue call, before gi_queue_shade.
const DECLARE: (&str, &str) = (
    "fn gi_queue_shade(slot:u32) {",
    "// Cause counters (src/gi_causes.rs; composed only for the cause log).
const GI_W_SHADE_CAUSE:u32=210u;
const GI_SHADE_AFTER_TRACE:u32=0u;
const GI_SHADE_SKY:u32=1u;
const GI_SHADE_RELIGHT:u32=2u;
const GI_SHADE_MOVER:u32=3u;
const GI_SHADE_AFTER_GATHER:u32=4u;
const GI_W_TRACE_PARTIAL:u32=215u;
// Counts the cell once (before gi_queue_shade sets GI_SHADE), then queues it.
fn gi_queue_shade_cause(slot:u32, cause:u32) {
    if (gi_table[gi_cold(slot)+GI_C_STATE]&GI_SHADE)==0u { atomicAdd(&gi_work[GI_W_SHADE_CAUSE+cause],1u); }
    gi_queue_shade(slot);
}
fn gi_queue_shade(slot:u32) {",
);

/// gi_cache.wgsl's seams: (main's text, the counting text).
const CACHE: [(&str, &str); 7] = [
    DECLARE,
    (
        "            if gi_bounce_on() {
                gi_queue_shade(slot);",
        "            if gi_bounce_on() {
                gi_queue_shade_cause(slot,GI_SHADE_AFTER_TRACE);",
    ),
    (
        "            if gi_bounce_on() && gi_filter_on(GI_FILTER_SKY) { gi_queue_shade(slot); }",
        "            if gi_bounce_on() && gi_filter_on(GI_FILTER_SKY) { gi_queue_shade_cause(slot,GI_SHADE_SKY); }",
    ),
    (
        "                gi_table[cold+GI_C_STATE]=gi_table[cold+GI_C_STATE]&~GI_LIGHTS_KNOWN;
                gi_queue_shade(slot);
                gi_schedule(slot,select(1u,gi_word(GI_H_GATHERS),all));",
        "                gi_table[cold+GI_C_STATE]=gi_table[cold+GI_C_STATE]&~GI_LIGHTS_KNOWN;
                gi_queue_shade_cause(slot,GI_SHADE_RELIGHT);
                gi_schedule(slot,select(1u,gi_word(GI_H_GATHERS),all));",
    ),
    (
        "            gi_table[cold+GI_C_STATE]=gi_table[cold+GI_C_STATE]&~GI_LIGHTS_KNOWN;
            gi_queue_reshade(slot);
        }
    }",
        "            gi_table[cold+GI_C_STATE]=gi_table[cold+GI_C_STATE]&~GI_LIGHTS_KNOWN;
            if (gi_table[cold+GI_C_STATE]&GI_SHADE)==0u { atomicAdd(&gi_work[GI_W_SHADE_CAUSE+GI_SHADE_MOVER],1u); }
            gi_queue_reshade(slot);
        }
    }",
    ),
    (
        "            gi_table[cold+GI_C_STATE]=(state&~(GI_GATHER_MASK|GI_BQUEUED|GI_RETRY))|retry;
            gi_queue_shade(slot);
            gi_gather_done(slot,left);",
        "            gi_table[cold+GI_C_STATE]=(state&~(GI_GATHER_MASK|GI_BQUEUED|GI_RETRY))|retry;
            gi_queue_shade_cause(slot,GI_SHADE_AFTER_GATHER);
            gi_gather_done(slot,left);",
    ),
    (
        "    var trace=(state&GI_TRACED)==0u || ((flags&GI_SCAN_PARTIAL)!=0u && (state&GI_PARTIAL)!=0u);\n",
        "    var trace=(state&GI_TRACED)==0u || ((flags&GI_SCAN_PARTIAL)!=0u && (state&GI_PARTIAL)!=0u);
    if trace { atomicAdd(&gi_work[GI_W_TRACE_PARTIAL],1u); }\n",
    ),
];
/// gi_gather_software.wgsl's seam (composed after gi_cache.wgsl, so its declarations hold).
const GATHER: [(&str, &str); 1] = [CACHE[5]];

/// `text` (shader file `name`) with the cause counters when they are enabled; None for other
/// files or when they are off.
pub(crate) fn compose(name: &str, text: &str) -> Option<String> {
    enabled().then(|| with_counters(name, text)).flatten()
}

fn with_counters(name: &str, text: &str) -> Option<String> {
    let seams: &[(&str, &str)] = match name {
        "gi_cache.wgsl" => &CACHE,
        "gi_gather_software.wgsl" => &GATHER,
        _ => return None,
    };
    let mut out = text.to_owned();
    for (from, to) in seams {
        assert_eq!(out.matches(from).count(), 1, "gi_causes seam moved in {name}: {}", from.lines().next().unwrap_or(""));
        out = out.replacen(from, to, 1);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    const CACHE: &str = include_str!("../shaders/gi_cache.wgsl");
    const GATHER: &str = include_str!("../shaders/gi_gather_software.wgsl");

    #[test]
    fn default_kernels_carry_no_cause_counter() {
        // Tests run without RENDERLAB_GI_CAUSES and RENDERLAB_GI_LOG: the composed text is the file's.
        assert!(!super::enabled());
        for (name, text) in [("gi_cache.wgsl", CACHE), ("gi_gather_software.wgsl", GATHER)] {
            assert_eq!(crate::program::gi_shader(name, text), text, "{name}");
            assert!(!text.contains("GI_W_SHADE_CAUSE") && !text.contains("GI_W_TRACE_PARTIAL") && !text.contains("gi_queue_shade_cause"), "{name}");
        }
    }

    #[test]
    fn every_seam_composes_once_and_counts_every_shade_site() {
        let cache = super::with_counters("gi_cache.wgsl", CACHE).unwrap();
        let gather = super::with_counters("gi_gather_software.wgsl", GATHER).unwrap();
        // Every gi_queue_shade call outside its definition counts a cause.
        for text in [&cache, &gather] {
            let plain = text.matches("gi_queue_shade(slot);").count();
            assert_eq!(plain, usize::from(text == &cache), "the one plain call is the counting wrapper's own");
        }
        assert_eq!(cache.matches("gi_queue_shade_cause(slot,").count(), 5);
        assert_eq!(gather.matches("gi_queue_shade_cause(slot,").count(), 1);
        assert_eq!(cache.matches("atomicAdd(&gi_work[GI_W_TRACE_PARTIAL],1u)").count(), 1);
    }
}
