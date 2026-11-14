//! Experimental ordered claim/gather admission. Unset switches keep the original module.
use super::*;
use std::sync::OnceLock;
pub(super) const HEADER_LIMIT: usize = 32;
pub(super) const DEFAULT_CELLS: u32 = 12_288;
pub(super) const FIRST_FLOOR: u32 = DEFAULT_CELLS - DEFAULT_CELLS / 5;
/// A soft gather target from the previous frame's measured cost, never a time ceiling.
pub(super) fn gather_target() -> f64 {
    static TARGET: OnceLock<f64> = OnceLock::new();
    *TARGET.get_or_init(|| {
        let target: f64 = std::env::var("RENDERLAB_GI_GATHER_MS")
            .unwrap_or_else(|_| "3".into())
            .parse()
            .expect("GI soft gather target");
        assert!(
            target.is_finite() && target > 0.0 && target <= f64::from(f32::MAX),
            "GI soft gather target must fit a positive finite f32"
        );
        target
    })
}

/// Measurement only: replay the clean run's per-frame budgets during blocking captures,
/// so PNG encoding/downclocking cannot change the time controller's picture workload.
pub(super) fn measurement_header(header: &mut [u32; HEADER_WORDS as usize]) {
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static FRAME: AtomicUsize = AtomicUsize::new(0);
    static REPLAY: OnceLock<Option<Vec<Vec<u32>>>> = OnceLock::new();
    static TRACE: OnceLock<Option<std::sync::Mutex<std::fs::File>>> = OnceLock::new();
    let frame = FRAME.fetch_add(1, Ordering::Relaxed);
    let replay = REPLAY.get_or_init(|| {
        std::env::var_os("RENDERLAB_GI_REFILL_REPLAY").map(|path| {
            std::fs::read_to_string(path)
                .expect("GI budget replay file")
                .lines()
                .enumerate()
                .map(|(index, line)| {
                    let row: Vec<u32> = line
                        .split(',')
                        .map(|s| s.parse().expect("GI replay integer"))
                        .collect();
                    assert!(
                        matches!(row.len(), 3 | 9),
                        "GI replay frame,claims,gathers[,first,costs,target,refresh,raw]"
                    );
                    assert_eq!(row[0] as usize, index, "GI replay frame order");
                    row[1..].to_vec()
                })
                .collect()
        })
    });
    if let Some(rows) = replay {
        let row = rows
            .get(frame)
            .expect("GI budget replay ended before the capture");
        let (claims, gathers) = (row[0], row[1]);
        assert!(
            gathers >= 64 && (claims == 0 || claims >= 64),
            "GI replay budget"
        );
        header[20] = gathers;
        header[32] = gathers;
        header[33] = claims;
        if row.len() == 8 {
            for (&index, &value) in [35, 36, 37, 38, 40, 41].iter().zip(&row[2..]) {
                header[index] = value;
            }
        }
    }
    let trace = TRACE.get_or_init(|| {
        std::env::var_os("RENDERLAB_GI_REFILL_TRACE").map(|path| {
            std::sync::Mutex::new(std::fs::File::create(path).expect("GI budget trace file"))
        })
    });
    if let Some(file) = trace {
        writeln!(
            file.lock().unwrap(),
            "{frame},{},{},{},{},{},{},{},{}",
            header[33],
            header[32],
            header[35],
            header[36],
            header[37],
            header[38],
            header[40],
            header[41]
        )
        .expect("GI budget trace write");
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Mode {
    /// Measurement control: the shipped 12k HOT ordering, without the soft gather target.
    Legacy,
    Today,
    Hold(u32),
    Fixed(u32),
    Milliseconds(f64),
}
pub(super) fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_GI_BOUNDED_REFILL").is_ok_and(|v| v == "1"))
}
pub(super) fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| {
        let Ok(value) = std::env::var("RENDERLAB_GI_REFILL_BUDGET") else {
            return Mode::Fixed(DEFAULT_CELLS);
        };
        if value == "legacy" {
            Mode::Legacy
        } else if value == "today" {
            Mode::Today
        } else if value == "hold" {
            Mode::Hold(
                std::env::var("RENDERLAB_GI_REFILL_HOLD")
                    .unwrap_or_else(|_| "8".into())
                    .parse()
                    .expect("GI refill hold frames"),
            )
        } else if value == "ms" {
            let ms: f64 = std::env::var("RENDERLAB_GI_REFILL_MS")
                .unwrap_or_else(|_| "3".into())
                .parse()
                .expect("GI refill milliseconds");
            assert!(
                ms.is_finite() && ms > 0.0,
                "GI refill milliseconds must be positive and finite"
            );
            Mode::Milliseconds(ms)
        } else {
            let cells = value
                .parse()
                .expect("GI refill budget: legacy, today, hold, ms or a cell count");
            assert!(cells >= 64, "GI refill cell budget must be at least 64");
            Mode::Fixed(cells)
        }
    })
}

pub(super) fn source(mut source: String, enabled: bool) -> String {
    if !enabled {
        return source;
    }
    let edits = [
        (
            "const GI_KEEP:u32=GI_SPLIT|GI_SHADE|GI_BQUEUED|GI_SECONDARY|GI_TERTIARY|GI_RETRY|GI_GATHER_MASK;",
            "const GI_KEEP:u32=GI_SPLIT|GI_SHADE|GI_BQUEUED|GI_SECONDARY|GI_TERTIARY|GI_RETRY|GI_GATHER_MASK|2048u;",
        ),
        (
            "@compute @workgroup_size(64) fn gi_claim(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {",
            "fn gi_priority_claim_body(id:vec3<u32>,groups:vec3<u32>,visible:bool) {",
        ),
        (
            "for(var i=id.x;i<count;i+=groups.x*64u) { gi_claim_miss(i); }",
            "for(var i=id.x;i<count;i+=groups.x*64u) { let base=gi_frame.offsets.y+i*GI_MISS_WORDS; if atomicLoad(&gi_work[base+5u])!=GI_CREATED && (gi_miss_band(base)>=GI_BAND_PIXEL)==visible { gi_claim_miss(i); } }",
        ),
        ("let period=gi_claim_period();", "let period=1u;"),
        (
            "    let key=gi_miss_key(base);\n    let mask=gi_word(GI_H_MASK);",
            "    if !gi_priority_claim_admit(gi_miss_band(base)) { atomicStore(&gi_work[base+5u],GI_CREATED); atomicAdd(&gi_work[GI_W_CLAIM_DEFERRED],1u); return; }\n    let key=gi_miss_key(base);\n    let mask=gi_word(GI_H_MASK);",
        ),
        (
            "@compute @workgroup_size(64) fn gi_gather_batched(@builtin(workgroup_id) group:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>,@builtin(local_invocation_index) lane:u32) {",
            "fn gi_priority_gather_body(group:vec3<u32>,groups:vec3<u32>,lane:u32) {",
        ),
        (
            "            let state=gi_table[cold+GI_C_STATE];\n            let queued=",
            "            let state=gi_table[cold+GI_C_STATE];\n            // Snapshot before the first gather changes a zero-bounce cell's class.\n            if gi_priority_phase==0u { atomicStore(&gi_work[GI_W_CLAIMS+slot],gi_priority_class(slot)+1u); }\n            let priority_class=atomicLoad(&gi_work[GI_W_CLAIMS+slot])-1u;\n            if gi_priority_phase==2u { atomicStore(&gi_work[GI_W_CLAIMS+slot],0u); }\n            let queued=priority_class==gi_priority_phase && ",
        ),
        (
            "if queued && gi_gather_now(slot,period,count) {",
            "if queued && gi_priority_gather_admit() {",
        ),
    ];
    for (before, after) in edits {
        assert_eq!(source.matches(before).count(), 1, "priority seam: {before}");
        source = source.replacen(before, after, 1);
    }
    // Share the unchanged ray/reduction body with the compacted queue entry point.
    let start = source
        .find("        for(var j=0u;j<listed;j+=1u) {\n")
        .expect("gather list seam")
        + "        for(var j=0u;j<listed;j+=1u) {\n".len();
    let end = start
        + source[start..]
            .find("\n        }\n    }\n}")
            .expect("gather body end");
    let body = source[start..end].replacen(
        "        let slot=gi_wg_list[j];",
        "        let requests=(gi_frame.bounce.z&GI_RELIGHT_REQUESTS)!=0u;",
        1,
    );
    source.replace_range(
        start..end,
        "        gi_priority_gather_cell(workgroupUniformLoad(&gi_wg_list[j]),lane);",
    );
    source.push_str(&format!(
        "\nfn gi_priority_gather_cell(slot:u32,lane:u32) {{\n{body}\n}}\n"
    ));
    source.push_str(include_str!("../shaders/gi_refill_priority.wgsl"));
    source.push_str(include_str!("../shaders/gi_refill_compact.wgsl"));
    source
}

pub(super) struct Pipelines {
    pub compact: Owned<wgpu::ComputePipeline>,
    pub select: Owned<wgpu::ComputePipeline>,
    pub defer: Owned<wgpu::ComputePipeline>,
    pub gather_first: Owned<wgpu::ComputePipeline>,
    pub gather_refresh: Owned<wgpu::ComputePipeline>,
    pub hold: Owned<wgpu::ComputePipeline>,
    pub gather_clamp: Owned<wgpu::ComputePipeline>,
    pub claim_reset: Owned<wgpu::ComputePipeline>,
    pub claim_background: Owned<wgpu::ComputePipeline>,
    pub gather_later: Owned<wgpu::ComputePipeline>,
    pub gather_background: Owned<wgpu::ComputePipeline>,
}
impl Pipelines {
    pub fn new(mut compute: impl FnMut(&str) -> Owned<wgpu::ComputePipeline>) -> Self {
        Self {
            compact: compute("gi_priority_compact"),
            select: compute("gi_priority_select"),
            defer: compute("gi_priority_defer"),
            gather_first: compute("gi_priority_gather_first"),
            gather_refresh: compute("gi_priority_gather_refresh"),
            hold: compute("gi_priority_hold_queue"),
            gather_clamp: compute("gi_priority_gather_clamp"),
            claim_reset: compute("gi_priority_claim_reset"),
            claim_background: compute("gi_priority_claim_background"),
            gather_later: compute("gi_priority_gather_later"),
            gather_background: compute("gi_priority_gather_background"),
        }
    }
}

/// Delayed feedback is a time target, not a GPU preemption deadline. Each update drops
/// at most 25% or grows 12.5%, with a 3000-cell floor and 20% target headroom.
pub(super) struct Controller {
    pub claims: u32,
    pub gathers: u32,
    claim_ms: f64,
    gather_ms: f64,
    last: u64,
    first_cost: f64,
    refresh_cost: f64,
    first_limit: u32,
    refresh_limit: u32,
    last_split: u64,
}
impl Default for Controller {
    fn default() -> Self {
        Self {
            claims: 3000,
            gathers: 3000,
            claim_ms: 0.001,
            gather_ms: 0.001,
            last: 0,
            first_cost: 0.0005,
            refresh_cost: 0.0005,
            first_limit: DEFAULT_CELLS,
            refresh_limit: DEFAULT_CELLS,
            last_split: 0,
        }
    }
}
impl Controller {
    /// A soft gather target from the previous frame's measured cost. The GPU raises
    /// first fill's floor to this frame's inflow before allowing refresh to use the rest.
    pub fn observe_split(
        &mut self,
        frame: u64,
        target: f64,
        first: u32,
        refresh: u32,
        first_ms: f64,
        refresh_ms: f64,
    ) {
        if frame <= self.last_split {
            return;
        }
        self.last_split = frame;
        for (count, ms, cost, limit, floor) in [
            (
                first,
                first_ms,
                &mut self.first_cost,
                &mut self.first_limit,
                FIRST_FLOOR,
            ),
            (
                refresh,
                refresh_ms,
                &mut self.refresh_cost,
                &mut self.refresh_limit,
                0,
            ),
        ] {
            if count < 64 || !ms.is_finite() || ms <= 0.0 {
                continue;
            }
            let measured = ms / f64::from(count);
            *cost = measured.max(*cost * 0.9 + measured * 0.1);
            let wanted = (target * 0.8 / *cost)
                .floor()
                .clamp(f64::from(floor), f64::from(GATHER_CELLS)) as u32;
            *limit = wanted.clamp(
                (*limit - *limit / 4).max(floor),
                *limit + (*limit / 8).max(1),
            );
        }
    }
    pub fn observe(
        &mut self,
        frame: u64,
        target: f64,
        claims: u32,
        gathers: u32,
        claim_ms: f64,
        gather_ms: f64,
    ) {
        if frame <= self.last {
            return;
        }
        self.last = frame;
        for (count, ms, estimate, budget, share) in [
            (claims, claim_ms, &mut self.claim_ms, &mut self.claims, 0.4),
            (
                gathers,
                gather_ms,
                &mut self.gather_ms,
                &mut self.gathers,
                0.6,
            ),
        ] {
            if count < 64 || !ms.is_finite() || ms <= 0.0 {
                continue;
            }
            let cost = ms / f64::from(count);
            *estimate = cost.max(*estimate * 0.9 + cost * 0.1);
            let wanted = (target * 0.8 * share / *estimate)
                .floor()
                .clamp(3000.0, f64::from(GATHER_CELLS)) as u32;
            *budget = wanted.clamp(
                (*budget - *budget / 4).max(3000),
                *budget + (*budget / 8).max(1),
            );
        }
    }
    pub fn header(&self, header: &mut [u32; HEADER_WORDS as usize], mode: Mode) {
        let (claims, gathers) = match mode {
            Mode::Legacy => (DEFAULT_CELLS, DEFAULT_CELLS),
            Mode::Today | Mode::Hold(_) => (header[22], GATHER_CELLS),
            Mode::Fixed(cells) => (cells, cells),
            // The follow-up budgets gather work, not update: keep creation at 12k.
            Mode::Milliseconds(_) => (DEFAULT_CELLS, self.gathers),
        };
        if matches!(mode, Mode::Hold(_)) {
            header[31] = 1;
        }
        header[20] = gathers;
        header[22] = 0; // Ordered resolve admission replaces the old hash-period claim gate.
        header[32] = gathers;
        header[33] = claims;
        header[34] = 1;
        header[35] = self.first_limit.max(FIRST_FLOOR);
        header[36] = (self.first_cost as f32).to_bits();
        header[37] = (self.refresh_cost as f32).to_bits();
        header[38] = (gather_target() as f32).to_bits();
        header[40] = self.refresh_limit;
        header[41] = u32::from(mode != Mode::Legacy);
        header[28] = header[28].max(1); // Gather hits keep the previous stamp; only pixels mark this frame.
    }
}

pub(super) fn compact_available(cap: u32, slots: u32) -> bool {
    u64::from(slots) + 2 * u64::from(cap) + 16 <= u64::from(RAY_ITEMS)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn soft_gather_feedback_keeps_the_first_fill_floor_and_bounds_growth() {
        let mut c = Controller::default();
        c.observe_split(2, 3.0, 12_000, 12_000, 120.0, 120.0);
        assert_eq!(c.first_limit, FIRST_FLOOR);
        assert_eq!(c.refresh_limit, DEFAULT_CELLS - DEFAULT_CELLS / 4);
        let old = (c.first_limit, c.refresh_limit, c.first_cost, c.refresh_cost);
        c.observe_split(1, 3.0, 12_000, 12_000, 0.001, 0.001);
        assert_eq!(
            (c.first_limit, c.refresh_limit, c.first_cost, c.refresh_cost),
            old
        );
        let mut header = [0; HEADER_WORDS as usize];
        c.header(&mut header, Mode::Fixed(DEFAULT_CELLS));
        assert_eq!(header[33], DEFAULT_CELLS);
        assert!(header[35] >= FIRST_FLOOR);
        assert_eq!(header[41], 1);
        c.header(&mut header, Mode::Legacy);
        assert_eq!(
            (header[32], header[33], header[41]),
            (DEFAULT_CELLS, DEFAULT_CELLS, 0)
        );
        assert!(compact_available(DEFAULT_CELLS, 1 << 20));
        assert!(!compact_available(u32::MAX, u32::MAX));
    }
    #[test]
    fn delayed_feedback_cuts_cost_spikes_and_ignores_old_samples() {
        let mut c = Controller::default();
        c.observe(2, 3.0, 1200, 1800, 12.0, 18.0);
        assert_eq!((c.claims, c.gathers), (3000, 3000));
        c.observe(1, 3.0, 1200, 1800, 0.01, 0.01);
        assert_eq!((c.claims, c.gathers), (3000, 3000));
        c.claims = 12000;
        c.gathers = 12000;
        c.observe(3, 3.0, 12000, 12000, 120.0, 120.0);
        assert_eq!((c.claims, c.gathers), (9000, 9000));
        let mut h = [0; HEADER_WORDS as usize];
        c.header(&mut h, Mode::Fixed(DEFAULT_CELLS));
        assert_eq!((h[32], h[33]), (12288, 12288));
        c.header(&mut h, Mode::Fixed(8192));
        assert_eq!((h[32], h[33]), (8192, 8192));
        c.header(&mut h, Mode::Today);
        assert_eq!(h[32], 32768);
    }
    #[test]
    fn ordered_modules_validate_without_changing_disabled_modules() {
        let original = kernel_source_base(true, 64);
        assert_eq!(source(original.clone(), false), original);
        for directions in [64, 128] {
            let original = kernel_source_base(true, directions);
            let module = source(original, true);
            let parsed = naga::front::wgsl::parse_str(&module)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&module)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&parsed)
            .unwrap();
        }
    }
}
