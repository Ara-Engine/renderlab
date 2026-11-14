//! Experimental ordered claim/gather admission. Unset switches keep the original module.
use super::*;
use std::sync::OnceLock;
pub(super) const HEADER_LIMIT: usize = 32;
pub(super) const DEFAULT_CELLS: u32 = 12_288;

/// Measurement only: replay the clean run's per-frame budgets during blocking captures,
/// so PNG encoding/downclocking cannot change the time controller's picture workload.
pub(super) fn measurement_header(header: &mut [u32; HEADER_WORDS as usize]) {
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static FRAME: AtomicUsize = AtomicUsize::new(0);
    static REPLAY: OnceLock<Option<Vec<[u32; 2]>>> = OnceLock::new();
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
                    assert_eq!(row.len(), 3, "GI replay frame,claims,gathers");
                    assert_eq!(row[0] as usize, index, "GI replay frame order");
                    [row[1], row[2]]
                })
                .collect()
        })
    });
    if let Some(rows) = replay {
        let [claims, gathers] = *rows
            .get(frame)
            .expect("GI budget replay ended before the capture");
        assert!(
            gathers >= 64 && (claims == 0 || claims >= 64),
            "GI replay budget"
        );
        header[20] = gathers;
        header[32] = gathers;
        header[33] = claims;
    }
    let trace = TRACE.get_or_init(|| {
        std::env::var_os("RENDERLAB_GI_REFILL_TRACE").map(|path| {
            std::sync::Mutex::new(std::fs::File::create(path).expect("GI budget trace file"))
        })
    });
    if let Some(file) = trace {
        writeln!(
            file.lock().unwrap(),
            "{frame},{},{}",
            header[33],
            header[32]
        )
        .expect("GI budget trace write");
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Mode {
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
        let Ok(value) = std::env::var("RENDERLAB_GI_REFILL_BUDGET") else { return Mode::Fixed(DEFAULT_CELLS); };
        if value == "today" {
            Mode::Today
        } else if value == "hold" {
            Mode::Hold(std::env::var("RENDERLAB_GI_REFILL_HOLD").unwrap_or_else(|_| "8".into()).parse().expect("GI refill hold frames"))
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
                .expect("GI refill budget: today, hold, ms or a cell count");
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
    let start = source.find("        for(var j=0u;j<listed;j+=1u) {\n").expect("gather list seam")
        + "        for(var j=0u;j<listed;j+=1u) {\n".len();
    let end = start + source[start..].find("\n        }\n    }\n}").expect("gather body end");
    let body = source[start..end].replacen("        let slot=gi_wg_list[j];", "        let requests=(gi_frame.bounce.z&GI_RELIGHT_REQUESTS)!=0u;", 1);
    source.replace_range(start..end, "        gi_priority_gather_cell(workgroupUniformLoad(&gi_wg_list[j]),lane);");
    source.push_str(&format!("\nfn gi_priority_gather_cell(slot:u32,lane:u32) {{\n{body}\n}}\n"));
    source.push_str(include_str!("../shaders/gi_refill_priority.wgsl"));
    source.push_str(include_str!("../shaders/gi_refill_compact.wgsl"));
    source
}

pub(super) struct Pipelines {
    pub compact: Owned<wgpu::ComputePipeline>,
    pub select: Owned<wgpu::ComputePipeline>,
    pub defer: Owned<wgpu::ComputePipeline>,
    pub gather_compacted: Owned<wgpu::ComputePipeline>,
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
            gather_compacted: compute("gi_priority_gather_compacted"),
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
}
impl Default for Controller {
    fn default() -> Self {
        Self {
            claims: 3000,
            gathers: 3000,
            claim_ms: 0.001,
            gather_ms: 0.001,
            last: 0,
        }
    }
}
impl Controller {
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
            *budget = wanted.clamp((*budget - *budget / 4).max(3000), *budget + (*budget / 8).max(1));
        }
    }
    pub fn header(&self, header: &mut [u32; HEADER_WORDS as usize], mode: Mode) {
        let (claims, gathers) = match mode {
            Mode::Today | Mode::Hold(_) => (header[22], GATHER_CELLS),
            Mode::Fixed(cells) => (cells, cells),
            Mode::Milliseconds(_) => (self.claims, self.gathers),
        };
        if matches!(mode, Mode::Hold(_)) { header[31] = 1; }
        header[20] = gathers;
        header[22] = 0; // Ordered resolve admission replaces the old hash-period claim gate.
        header[32] = gathers;
        header[33] = claims;
        header[34] = 1;
        header[28] = header[28].max(1); // Gather hits keep the previous stamp; only pixels mark this frame.
    }
}

pub(super) fn compact_available(cap: u32) -> bool { cap <= (RAY_ITEMS - 8) / 3 }

#[cfg(test)]
mod tests {
    use super::*;
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
