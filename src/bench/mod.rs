//! The test bench (`lightbench`, feature `bench`, docs/bench.md): one headless process that
//! keeps a renderer per fixture on each path (with and without ray hardware), watches the
//! shader sources, rebuilds only the pass whose source changed (keeping the last working
//! pipelines when the new text fails), reruns every fixture and prints one line per fixture
//! and path, plus one JSON line for tools.
use crate::Error;
use crate::renderer::Renderer;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

mod city;
mod cause;
mod dump;
mod replay;
pub(crate) mod edits;
mod reload;
mod fixtures;
mod furnace;
mod glass;
pub(crate) mod kit;
mod reflect;
mod shadow_edges;
mod tr1_edge;

/// The two device paths every fixture runs on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BenchPath {
    NoRt,
    Rt,
}
impl BenchPath {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::NoRt => "nort",
            Self::Rt => "rt",
        }
    }
}

/// A fixture's reference image: radiance in cd/m^2, rows top first.
pub(crate) struct RefImage {
    pub size: [u32; 2],
    pub rgb: Vec<[f32; 3]>,
}

/// A bench fixture: a tiny scene, built once per path on its own renderer, and a scorer.
/// One file per fixture under src/bench/ and one line in `fixtures::fixtures` add it.
pub(crate) trait Fixture {
    /// Printed on every line; unique.
    fn name(&self) -> &str;
    /// Text that changes whenever the scene or its reference would: hashed into the
    /// reference cache's file name.
    fn scene_key(&self) -> String {
        self.name().to_owned()
    }
    /// Builds the scene on `r` (this fixture's renderer on `path`, kept for the process).
    fn load(&mut self, r: &mut Renderer, path: BenchPath) -> Result<(), Error>;
    /// Renders the fixture's frames and scores them: `(name, value)` pairs, printed in order.
    /// `err` (against the reference) and `energy` (against the exact answer) come first by
    /// convention.
    fn run(&mut self, r: &mut Renderer, path: BenchPath, reference: Option<&RefImage>) -> Result<Vec<(String, f64)>, Error>;
    /// Renders the reference once (the live path tracer, on the RT renderer when there is
    /// one); the bench caches it on disk. None: the fixture needs no stored reference.
    fn reference(&mut self, _r: &mut Renderer) -> Option<Result<RefImage, Error>> {
        None
    }
    /// Per-frame values from the last `run` (rays per frame, ...): the JSON line carries them
    /// whole (`series`), the text line their median and largest value.
    fn series(&mut self) -> Vec<(String, Vec<f64>)> {
        vec![]
    }
}

/// Shader text read from disk by the bench, by file name under shaders/: `program::gi_shader`
/// composes it instead of the built-in text.
static TEXT: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());
pub(crate) fn shader_text(name: &str) -> Option<String> {
    TEXT.lock().unwrap().get(name).cloned()
}

/// A pass the bench can rebuild: the shader files it composes, its rebuild, and the fixtures
/// whose numbers read it (empty: every fixture). `edits`: its files' new text reaches modules
/// through `edits` (every module the process creates) instead of `shader_text`.
struct Pass {
    name: &'static str,
    files: &'static [&'static str],
    reload: fn(&mut Renderer) -> Result<(), Error>,
    fixtures: &'static [&'static str],
    edits: bool,
}
/// After a reload, only the pass's fixtures rebuild and run; the others rebuild before they next
/// run (`Rig::stale`). The prelude passes share one rebuild (src/bench/reload.rs): their kernels
/// all compose the traced heads, and a file of one (a reflection or TR1 view file) can compose
/// into another's kernels too.
const PASSES: &[Pass] = &[Pass {
    name: "gi",
    files: &[
        "gi_cache.wgsl",
        "gi_cells.wgsl",
        "gi_near.wgsl",
        "gi_emitters.wgsl",
        "gi_gather_software.wgsl",
        "gi_rays_software.wgsl",
        "gi_rays_hardware.wgsl",
        "gi_shade_entry.wgsl",
    ],
    reload: Renderer::bench_reload_gi,
    fixtures: &[],
    edits: false,
}, Pass {
    name: "vsm",
    files: &[
        "virtual_shadow.wgsl",
        "virtual_shadow_records.wgsl",
        "virtual_shadow_hybrid.wgsl",
        "virtual_shadow_lookup.wgsl",
        "virtual_shadow_disk.wgsl",
    ],
    reload: Renderer::bench_reload_vsm,
    fixtures: &[],
    edits: false,
}, Pass {
    name: "shading",
    files: &[
        "scene.wgsl",
        "program_api.wgsl",
        "program_traced.wgsl",
        "program_traced_entry.wgsl",
        "program_traced_list.wgsl",
        "program_traced_residency.wgsl",
        "program_view.wgsl",
        "program_bins.wgsl",
        "programs/standard.wgsl",
        "programs/toon.wgsl",
        "light_cluster_api.wgsl",
        "light_vis_pass.wgsl",
        "decals.wgsl",
        "environment_lobes.wgsl",
        "specular_filter.wgsl",
        "shadow_visibility_filter.wgsl",
        "gi_api.wgsl",
    ],
    reload: Renderer::bench_reload_prelude,
    fixtures: &["grey", "orange", "shadow-edges", "reflect"],
    edits: true,
}, Pass {
    name: "reflections",
    files: &[
        "reflections.wgsl",
        "reflections_composite.wgsl",
        "reflections_deferred.wgsl",
        "reflections_history.wgsl",
        "reflections_lobe.wgsl",
        "reflections_resolve.wgsl",
        "reflections_software.wgsl",
        "reflections_software_bvh4.wgsl",
        "reflections_trace.wgsl",
        "reflection_cache_hits.wgsl",
        "reflection_hit_bins.wgsl",
        "reflection_hit_sun.wgsl",
        "reflection_hits.wgsl",
    ],
    reload: Renderer::bench_reload_prelude,
    fixtures: &["reflect"],
    edits: true,
}, Pass {
    name: "transparency",
    files: &["transparency.wgsl", "transparency_shadow.wgsl", "id_glass.wgsl", "pane_pixels.wgsl", "transmission_reduce.wgsl"],
    reload: Renderer::bench_reload_prelude,
    fixtures: &["glass"],
    edits: true,
}, Pass {
    name: "gi_receiver",
    files: &["gi_receiver_pass.wgsl"],
    reload: Renderer::bench_reload_prelude,
    fixtures: &["walk", "grey"],
    edits: true,
}, Pass {
    name: "tr1",
    files: &["temporal_resolve.wgsl", "temporal_zones.wgsl", "temporal_coverage_cycle.wgsl", "temporal_coverage_pack.wgsl", "temporal_view.wgsl"],
    reload: Renderer::bench_reload_prelude,
    fixtures: &["tr1-edge"],
    edits: true,
}, Pass {
    name: "visibility",
    files: &[
        "primary_rays.wgsl",
        "primary_trace.wgsl",
        "primary_interactive.wgsl",
        "primary_edge_check.wgsl",
        "primary_edge_queue.wgsl",
        "visibility_buffer.wgsl",
        "visibility_buffer_attachment.wgsl",
        "visibility_buffer_attachment_hzb.wgsl",
        "visibility_buffer_cull.wgsl",
        "visibility_buffer_hzb.wgsl",
        "visibility_buffer_raster.wgsl",
        "visibility_buffer_reset.wgsl",
        "visibility_buffer_resolve.wgsl",
        "visibility_buffer_sw.wgsl",
    ],
    reload: Renderer::bench_reload_prelude,
    fixtures: &["shadow-edges", "tr1-edge"],
    edits: true,
}];

struct Options {
    once: bool,
    paths: Vec<BenchPath>,
    only: Option<Vec<String>>,
    shaders: PathBuf,
    hold: PathBuf,
    gpu_lock: PathBuf,
    refs: PathBuf,
    runs: PathBuf,
}
fn options() -> Result<Options, String> {
    let env = |k: &str, default: &str| PathBuf::from(std::env::var(k).unwrap_or_else(|_| default.to_owned()));
    let mut o = Options {
        once: false,
        // Without ray hardware by default: the walking-cost goal's path. `--paths nort,rt` adds RT.
        paths: vec![BenchPath::NoRt],
        only: None,
        shaders: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders"),
        hold: env("LIGHTBENCH_HOLD", "D:/Coding/renderlab-work/timing-hold"),
        gpu_lock: env("LIGHTBENCH_GPU_LOCK", "D:/Coding/renderlab-work/gpu-slot.lock"),
        refs: env("LIGHTBENCH_REFS", "D:/Coding/renderlab-work/bench-refs"),
        runs: env("LIGHTBENCH_RUNS", "D:/Coding/renderlab-work/bench-runs"),
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--once" => o.once = true,
            "--paths" => {
                o.paths = args
                    .next()
                    .ok_or("--paths rt,nort")?
                    .split(',')
                    .map(|p| match p {
                        "rt" => Ok(BenchPath::Rt),
                        "nort" => Ok(BenchPath::NoRt),
                        other => Err(format!("unknown path {other}")),
                    })
                    .collect::<Result<_, _>>()?
            }
            "--fixtures" => o.only = Some(args.next().ok_or("--fixtures a,b")?.split(',').map(str::to_owned).collect()),
            "--shaders" => o.shaders = PathBuf::from(args.next().ok_or("--shaders <dir>")?),
            other => return Err(format!("unknown argument {other}; see docs/bench.md")),
        }
    }
    Ok(o)
}

/// A fixture on one path: its renderer (recreated after a fault) and cached reference.
struct Rig {
    path: BenchPath,
    device: (crate::pipeline_compilation::PipelineDevice, wgpu::Queue),
    fixture: Box<dyn Fixture>,
    r: Option<Renderer>,
    reference: Option<RefImage>,
    /// This rig's own pass log (each renderer truncates the file it opens) and how far it was read.
    pass_log: PathBuf,
    read_to: u64,
    /// Passes reloaded since this rig's renderer last rebuilt them: it rebuilds them before it
    /// next runs.
    stale: Vec<&'static Pass>,
}
impl Rig {
    /// Whether a reload of `pass` runs this rig's fixture.
    fn reads(&self, pass: &Pass) -> bool {
        pass.fixtures.is_empty() || pass.fixtures.contains(&self.fixture.name())
    }
    fn build(&mut self, refs: &Path) -> Result<(), Error> {
        // SAFETY: the bench's threads never read the environment; renderers read it as they are created.
        unsafe { std::env::set_var("RENDERLAB_PASS_LOG", &self.pass_log) };
        self.read_to = 0;
        let mut r = kit::renderer(&self.device, self.path)?;
        self.fixture.load(&mut r, self.path)?;
        if self.reference.is_none() {
            self.reference = reference(&mut *self.fixture, &mut r, refs)?;
        }
        self.r = Some(r);
        Ok(())
    }
}

/// The fixture's cached reference (`<refs>/<name>-<hash of scene_key>.lbref`), rendered and
/// saved on first use.
fn reference(fixture: &mut dyn Fixture, r: &mut Renderer, refs: &Path) -> Result<Option<RefImage>, Error> {
    let hash = format!("{:x}", Sha256::digest(fixture.scene_key().as_bytes()));
    let file = refs.join(format!("{}-{}.lbref", fixture.name(), &hash[..16]));
    if let Ok(bytes) = std::fs::read(&file) {
        if let Some(image) = decode_reference(&bytes) {
            return Ok(Some(image));
        }
    }
    let Some(image) = fixture.reference(r) else { return Ok(None) };
    let image = image?;
    let mut bytes = b"LBREF1".to_vec();
    bytes.extend(image.size[0].to_le_bytes());
    bytes.extend(image.size[1].to_le_bytes());
    bytes.extend(image.rgb.iter().flatten().flat_map(|v| v.to_le_bytes()));
    if let Err(error) = std::fs::create_dir_all(refs).and_then(|()| std::fs::write(&file, bytes)) {
        println!("lightbench: could not cache the reference at {}: {error}", file.display());
    }
    Ok(Some(image))
}
fn decode_reference(bytes: &[u8]) -> Option<RefImage> {
    let body = bytes.strip_prefix(b"LBREF1")?;
    let word = |i: usize| Some(u32::from_le_bytes(body.get(i * 4..i * 4 + 4)?.try_into().ok()?));
    let size = [word(0)?, word(1)?];
    let floats: Vec<f32> = body[8..].chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
    (floats.len() == (size[0] * size[1] * 3) as usize)
        .then(|| RefImage { size, rgb: floats.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect() })
}

/// Why the machine must be quiet now, if it must: a gate's lease (the timing-hold's or the GPU
/// lock's owner file names a `gate` tool: the clip gate, the play gate, the scoreboard), a timing
/// lease (`--timing`, whose owner file carries no `exclusive` field) or an exclusive one. Other
/// ordinary leases (`"exclusive":false`, no `gate`: one-off scripts) leave the bench running
/// beside them (docs/bench.md). Main thread, 21 Nov: a bench beside a gate read slowed every pass
/// 2.5x (a real-size fixture) or about 10% (a small one), above the gates' 0.3 ms line.
fn quiet_wanted(hold: &Path, gpu_lock: &Path) -> Option<String> {
    let owner = |dir: &Path| -> Option<serde_json::Value> {
        dir.exists().then(|| std::fs::read_to_string(dir.join("owner.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default())
    };
    let named = |v: &serde_json::Value| format!("{} {}", v["owner"].as_str().unwrap_or("?"), v["label"].as_str().unwrap_or("")).trim().to_owned();
    for v in [owner(hold), owner(gpu_lock)].into_iter().flatten() {
        if let Some(gate) = v["gate"].as_str() {
            return Some(format!("{}'s gate ({gate}: {})", v["owner"].as_str().unwrap_or("?"), v["label"].as_str().unwrap_or("")));
        }
    }
    let hold = owner(hold)?;
    // A hold without its owner file yet (between the lease's mkdir and its write) counts as quiet.
    (hold["exclusive"] != serde_json::Value::Bool(false)).then(|| format!("a timing lease: {}", named(&hold)))
}
/// Waits while a gate, a timing lease or an exclusive hold wants the machine quiet, saying so.
fn wait_quiet(o: &Options) {
    let Some(why) = quiet_wanted(&o.hold, &o.gpu_lock) else { return };
    println!("lightbench: idle: {why}");
    let started = Instant::now();
    while quiet_wanted(&o.hold, &o.gpu_lock).is_some() {
        std::thread::sleep(Duration::from_millis(500));
    }
    println!("lightbench: resuming after {:.0} s idle", started.elapsed().as_secs_f64());
}
/// The leases running now (the timing-hold's and the GPU lock's owners): a round that overlaps
/// one is contended, and its milliseconds are not clean.
fn leases(o: &Options) -> Vec<String> {
    [&o.hold, &o.gpu_lock]
        .into_iter()
        .filter_map(|d| std::fs::read_to_string(d.join("owner.json")).ok())
        .map(|text| {
            let get = |key: &str| text.split(&format!("\"{key}\":\"")).nth(1).and_then(|r| r.split('"').next()).unwrap_or("").to_owned();
            format!("{} {}", get("owner"), get("label")).trim().to_owned()
        })
        .collect()
}

/// The pass log's frames from `offset` on (the last `last` of them when given): each pass's
/// median GPU milliseconds over the frames that did not compile, with `span` (the frame) and
/// `cache` (every `gi_*` stage: the lighting cache's whole cost); each frame's cache
/// milliseconds in order (None where it compiled or was dropped); and the end of the log.
struct PassStats {
    medians: BTreeMap<String, f64>,
    cache_frames: Vec<Option<f64>>,
}
fn pass_stats(log: &Path, offset: u64, last: Option<usize>, after: usize) -> (PassStats, u64) {
    let Ok(text) = std::fs::read(log) else {
        return (PassStats { medians: BTreeMap::new(), cache_frames: vec![] }, offset);
    };
    let end = text.len() as u64;
    let text = String::from_utf8_lossy(&text[(offset.min(end)) as usize..]).into_owned();
    let mut frames: Vec<Option<BTreeMap<String, f64>>> = vec![];
    for line in text.lines().filter(|l| l.starts_with('{')) {
        let compiled = !line.contains("\"compiles\":0,") && line.contains("\"compiles\":");
        if line.contains("\"dropped\":true") || compiled {
            frames.push(None);
            continue;
        }
        let mut stages = BTreeMap::new();
        if let Some(span) = field(line, "\"span\":") {
            stages.insert("span".to_owned(), span);
        }
        if let Some(start) = line.find("\"passes\":{") {
            let body = &line[start + 10..];
            for item in body[..body.find('}').unwrap_or(0)].split(',') {
                if let Some((k, v)) = item.split_once(':') {
                    if let Ok(v) = v.trim().parse::<f64>() {
                        stages.insert(k.trim().trim_matches('"').to_owned(), v);
                    }
                }
            }
        }
        let cache: f64 = stages.iter().filter(|(k, _)| k.starts_with("gi_")).map(|(_, v)| v).sum();
        stages.insert("cache".to_owned(), cache);
        frames.push(Some(stages));
    }
    // The window: the `last` frames before the final `after` ones.
    frames.truncate(frames.len().saturating_sub(after));
    if let Some(last) = last {
        frames.drain(..frames.len().saturating_sub(last));
    }
    let mut samples: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for stages in frames.iter().flatten() {
        for (k, v) in stages {
            samples.entry(k.clone()).or_default().push(*v);
        }
    }
    let medians = samples
        .into_iter()
        .map(|(k, mut v)| {
            v.sort_by(f64::total_cmp);
            (k, v[v.len() / 2])
        })
        .collect();
    let cache_frames = frames.iter().map(|f| f.as_ref().map(|s| s["cache"])).collect();
    (PassStats { medians, cache_frames }, end)
}
fn field(line: &str, key: &str) -> Option<f64> {
    let rest = &line[line.find(key)? + key.len()..];
    rest[..rest.find([',', '}'])?].trim().parse().ok()
}

/// The pipeline compiles logged from `offset` on (the renderers' pipeline log): their count,
/// seconds and the longest, and the end of the log.
fn compiles(log: &Path, offset: u64) -> (String, u64) {
    let Ok(text) = std::fs::read(log) else { return (String::new(), offset) };
    let end = text.len() as u64;
    let text = String::from_utf8_lossy(&text[(offset.min(end)) as usize..]).into_owned();
    let (mut n, mut total, mut longest) = (0, 0.0, (0.0, String::new()));
    for line in text.lines().filter(|l| l.contains("\"event\":\"compile\"") && !l.contains("\"kind\":\"module\"")) {
        let ms = field(line, "\"ms\":").unwrap_or(0.0);
        n += 1;
        total += ms;
        if ms > longest.0 {
            let label = line.split("\"label\":\"").nth(1).and_then(|r| r.split('"').next()).unwrap_or("");
            longest = (ms, label.to_owned());
        }
    }
    (format!("{n} pipelines in {:.2} s, longest {} {:.2} s", total / 1e3, longest.1, longest.0 / 1e3), end)
}

fn json_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}
fn json_map(items: &[(String, f64)]) -> String {
    let body: Vec<String> = items
        .iter()
        .map(|(k, v)| format!("{}:{}", json_string(k), if v.is_finite() { format!("{v:.6}") } else { "null".into() }))
        .collect();
    format!("{{{}}}", body.join(","))
}

/// The GPU's graphics clock through a round: nvidia-smi sampling every 200 ms (NVIDIA only;
/// nothing elsewhere), its median and lowest MHz, so a round the GPU spent in a slow state marks
/// itself as `contended` does (gicut, 21 Nov: a 9.17 ms span with every pass about 3x, the clock
/// dropping to 585-885 MHz).
struct Clock(Option<std::process::Child>);
impl Clock {
    fn start() -> Self {
        Self(
            std::process::Command::new("nvidia-smi")
                .args(["--query-gpu=clocks.gr", "--format=csv,noheader,nounits", "-lms", "200"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .ok(),
        )
    }
    fn finish(self) -> Option<(f64, f64)> {
        let mut child = self.0?;
        let _ = child.kill();
        let out = child.wait_with_output().ok()?;
        let mut mhz: Vec<f64> = String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.trim().parse().ok()).collect();
        mhz.sort_by(f64::total_cmp);
        Some((*mhz.get(mhz.len() / 2)?, mhz[0]))
    }
}

/// Runs the rigs `run` picks once and prints their lines, first rebuilding the passes they are
/// stale in, then the round's GPU clock. Returns the seconds the fixtures took.
fn run_round(rigs: &mut [Rig], o: &Options, round: u32, run: impl Fn(&Rig) -> bool) -> f64 {
    wait_quiet(o);
    let clock = Clock::start();
    let started = Instant::now();
    for rig in rigs.iter_mut().filter(|rig| run(rig)) {
        if rig.r.is_none() {
            if let Err(error) = rig.build(&o.refs) {
                println!("bench {} {} error: could not build the fixture: {error}", rig.fixture.name(), rig.path.name());
                continue;
            }
            rig.stale.clear();
        }
        for pass in std::mem::take(&mut rig.stale) {
            if let Err(error) = (pass.reload)(rig.r.as_mut().unwrap()) {
                println!("bench {} {} error: rebuilding {}: {error}", rig.fixture.name(), rig.path.name(), pass.name);
            }
        }
        let r = rig.r.as_mut().unwrap();
        let t = Instant::now();
        let mut busy = leases(o);
        let result = rig.fixture.run(r, rig.path, rig.reference.as_ref());
        let _ = kit::drain(r);
        busy.extend(leases(o));
        busy.sort();
        busy.dedup();
        let contended = if busy.is_empty() { String::new() } else { format!(" | contended: {}", busy.join("; ")) };
        let seconds = t.elapsed().as_secs_f64();
        let name = rig.fixture.name().to_owned();
        match result {
            Ok(mut metrics) => {
                // `frames_measured`: the pass times cover only the fixture's last frames.
                let mut take = |name: &str| metrics.iter().position(|(k, _)| k == name).map(|i| metrics.remove(i).1 as usize);
                // `frames_after`: frames after that window (a hold after a walk) are left out too.
                let last = take("frames_measured");
                let after = take("frames_after").unwrap_or(0);
                let (stats, end) = pass_stats(&rig.pass_log, rig.read_to, last, after);
                rig.read_to = end;
                let cache: Vec<f64> = stats.cache_frames.iter().flatten().copied().collect();
                let cache_max = cache.iter().copied().fold(0.0, f64::max);
                let series = rig.fixture.series();
                let mut shown: Vec<String> = metrics.iter().map(|(k, v)| format!("{k}={v:.4}")).collect();
                for (k, values) in &series {
                    let mut sorted = values.clone();
                    sorted.sort_by(f64::total_cmp);
                    if let (Some(median), Some(max)) = (sorted.get(sorted.len() / 2), sorted.last()) {
                        shown.push(format!("{k}/frame={median:.0} (max {max:.0})"));
                    }
                }
                let pass_text: Vec<String> =
                    stats.medians.iter().filter(|(k, _)| *k != "cache" && *k != "span").map(|(k, v)| format!("{k}={v:.2}")).collect();
                // The cache's cost by the walking-cost budget's three parts: new-cell tracing (update,
                // claim, probe), gather and integrate; the rest is cell shading and the receivers.
                let m = |k: &str| stats.medians.get(k).copied().unwrap_or(0.0);
                let trace = m("gi_update") + m("gi_claim") + m("gi_probe");
                let split = format!(
                    "trace {trace:.2} gather {:.2} integrate {:.2} other {:.2}",
                    m("gi_gather"),
                    m("gi_integrate"),
                    m("cache") - trace - m("gi_gather") - m("gi_integrate")
                );
                println!(
                    "bench {name} {} {} | cache ms/frame median {:.2} max {cache_max:.2} over {} frames ({split}), span {:.2} | ms {} | {seconds:.2} s{contended}",
                    rig.path.name(),
                    shown.join(" "),
                    m("cache"),
                    cache.len(),
                    m("span"),
                    pass_text.join(" ")
                );
                if let Some(causes) = cause::line(&series, m) {
                    println!("bench {name} {} cause | {causes}", rig.path.name());
                }
                let passes: Vec<(String, f64)> = stats.medians.into_iter().collect();
                let cache_series: Vec<String> =
                    stats.cache_frames.iter().map(|v| v.map_or("null".into(), |v| format!("{v:.4}"))).collect();
                println!(
                    "{{\"bench\":\"lightbench\",\"round\":{round},\"fixture\":{},\"path\":\"{}\",\"metrics\":{},\"pass_ms\":{},\"cache_ms_frames\":[{}],\"series\":{{{}}},\"contended\":{},\"run_s\":{seconds:.3}}}",
                    json_string(&name),
                    rig.path.name(),
                    json_map(&metrics),
                    json_map(&passes),
                    cache_series.join(","),
                    series
                        .iter()
                        .map(|(k, v)| format!("{}:[{}]", json_string(k), v.iter().map(|x| format!("{x}")).collect::<Vec<_>>().join(",")))
                        .collect::<Vec<_>>()
                        .join(","),
                    if busy.is_empty() { "null".to_owned() } else { json_string(&busy.join("; ")) }
                );
            }
            Err(error) => {
                rig.read_to = std::fs::metadata(&rig.pass_log).map_or(0, |m| m.len());
                println!("bench {name} {} error: {error}", rig.path.name());
                if matches!(error, Error::Fault(_)) {
                    // A faulted renderer is rebuilt on the next round.
                    rig.r = None;
                }
            }
        }
    }
    let seconds = started.elapsed().as_secs_f64();
    if let Some((median, lowest)) = clock.finish() {
        println!("lightbench: round {round} GPU clock median {median:.0} MHz, lowest {lowest:.0}");
    }
    seconds
}

/// The watched shader files (shaders/*.wgsl and shaders/programs/*.wgsl) and their times.
fn scan(shaders: &Path) -> BTreeMap<String, SystemTime> {
    let mut out = BTreeMap::new();
    for (dir, prefix) in [(shaders.to_path_buf(), ""), (shaders.join("programs"), "programs/")] {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".wgsl") {
                if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                    out.insert(format!("{prefix}{name}"), modified);
                }
            }
        }
    }
    out
}

/// The bench's entry point (src/bin/lightbench.rs). Returns the process exit code.
pub fn main() -> i32 {
    // The cause line's counters (src/gi_causes.rs), composed into the cache kernels before the first renderer;
    // LIGHTBENCH_CAUSES=0 runs the default kernels.
    if std::env::var("LIGHTBENCH_CAUSES").as_deref() != Ok("0") && std::env::var_os("RENDERLAB_GI_CAUSES").is_none() {
        // SAFETY: set before the bench starts any thread or renderer.
        unsafe { std::env::set_var("RENDERLAB_GI_CAUSES", "1") };
    }
    let o = match options() {
        Ok(o) => o,
        Err(error) => {
            eprintln!("lightbench: {error}");
            return 2;
        }
    };
    if let Err(error) = std::fs::create_dir_all(&o.runs) {
        eprintln!("lightbench: {}: {error}", o.runs.display());
        return 1;
    }
    // Per-pass GPU times come from the renderers' own pass logs (src/pass_timeline.rs), one
    // per rig, which each renderer opens when it is created.
    let pid = std::process::id();
    // Each compile's label and time, for the summaries after start-up and each reload.
    let compile_log = o.runs.join(format!("pipelines-{pid}.jsonl"));
    let _ = std::fs::remove_file(&compile_log);
    // SAFETY: set before any thread starts; nothing else reads the environment concurrently.
    unsafe { std::env::set_var("RENDERLAB_PIPELINE_LOG", &compile_log) };
    // Frames keep their draws' programs, which the fixtures' live references trace (kit.rs).
    // SAFETY: as above.
    unsafe { std::env::set_var("RENDERLAB_LIVE_REFERENCE", "1") };
    // The passes' sources as they are on disk now, not as this binary was built.
    {
        let mut text = TEXT.lock().unwrap();
        for file in PASSES.iter().filter(|p| !p.edits).flat_map(|p| p.files) {
            if let Ok(source) = std::fs::read_to_string(o.shaders.join(file)) {
                text.insert((*file).to_owned(), source);
            }
        }
    }
    // The edits passes' files as the bench starts: their hunks are taken against this text,
    // which the binary's own (`include_str!`) is assumed to match.
    let mut base: BTreeMap<String, String> = BTreeMap::new();
    for file in PASSES.iter().filter(|p| p.edits).flat_map(|p| p.files) {
        if let Ok(source) = std::fs::read_to_string(o.shaders.join(file)) {
            base.insert((*file).to_owned(), source);
        }
    }
    wait_quiet(&o);
    let started = Instant::now();
    let mut rigs = vec![];
    for &path in &o.paths {
        let device = match kit::device(path) {
            Ok(device) => device,
            Err(error) => {
                println!("lightbench: no {} device: {error}", path.name());
                continue;
            }
        };
        for fixture in fixtures::fixtures() {
            let named = o.only.as_ref().map(|only| only.iter().any(|n| n == fixture.name()));
            if named == Some(false) || named.is_none() && fixtures::NAMED_ONLY.contains(&fixture.name()) {
                continue;
            }
            let pass_log = o.runs.join(format!("passes-{pid}-{}-{}.jsonl", fixture.name(), path.name()));
            rigs.push(Rig { path, device: device.clone(), fixture, r: None, reference: None, pass_log, read_to: 0, stale: vec![] });
        }
    }
    for rig in &mut rigs {
        if let Err(error) = rig.build(&o.refs) {
            println!("bench {} {} error: could not build the fixture: {error}", rig.fixture.name(), rig.path.name());
        }
    }
    let (built, mut compiled_to) = compiles(&compile_log, 0);
    println!("lightbench: {} fixtures on {} paths built in {:.1} s ({built}); watching {}", rigs.len() / o.paths.len().max(1), o.paths.len(), started.elapsed().as_secs_f64(), o.shaders.display());
    let mut round = 0;
    let seconds = run_round(&mut rigs, &o, round, |_| true);
    println!("lightbench: round {round}: fixtures {seconds:.2} s");
    // The paths' pipeline caches on disk (each facade is shared, so once per path).
    let save = |rigs: &[Rig]| rigs.iter().for_each(|rig| rig.device.0.save_cache());
    save(&rigs);
    if o.once {
        return 0;
    }
    let mut seen = scan(&o.shaders);
    loop {
        std::thread::sleep(Duration::from_millis(50));
        let now = scan(&o.shaders);
        let changed: Vec<(String, SystemTime)> =
            now.iter().filter(|(k, t)| seen.get(*k) != Some(t)).map(|(k, t)| (k.clone(), *t)).collect();
        if changed.is_empty() {
            continue;
        }
        seen = now;
        let saved = changed.iter().map(|(_, t)| *t).min().unwrap();
        // Editors write a file in more than one step: read it once it has stopped changing.
        std::thread::sleep(Duration::from_millis(20));
        let mut reload: Vec<&'static Pass> = vec![];
        let mut previous: Vec<(String, Option<String>)> = vec![];
        let mut edited: Vec<String> = vec![];
        for (file, _) in &changed {
            let Some(pass) = PASSES.iter().find(|p| p.files.contains(&file.as_str())) else {
                println!("lightbench: {file} changed; no bench pass reloads it yet (docs/bench.md)");
                continue;
            };
            let Ok(source) = std::fs::read_to_string(o.shaders.join(file)) else { continue };
            if pass.edits {
                if edits::set(file, base.get(file).map_or("", String::as_str), &source) {
                    edited.push(file.clone());
                }
            } else {
                let old = TEXT.lock().unwrap().insert(file.clone(), source);
                previous.push((file.clone(), old));
            }
            if !reload.iter().any(|p| p.name == pass.name) {
                reload.push(pass);
            }
        }
        if reload.is_empty() {
            continue;
        }
        wait_quiet(&o);
        round += 1;
        let t = Instant::now();
        edits::reset_counts();
        for &pass in &reload {
            for rig in rigs.iter_mut().filter(|rig| rig.r.is_some()) {
                if !rig.reads(pass) {
                    if !rig.stale.iter().any(|p| p.name == pass.name) {
                        rig.stale.push(pass);
                    }
                } else {
                    rig.stale.retain(|p| p.name != pass.name);
                }
            }
            // The rigs' renderers rebuild at once, each on its own thread (prepare's compile
            // slots add up), so the kernels of both paths compile together.
            let failed = std::thread::scope(|s| {
                let jobs: Vec<_> = rigs
                    .iter_mut()
                    .filter(|rig| rig.reads(pass))
                    .filter_map(|rig| {
                        let path = rig.path.name();
                        rig.r.as_mut().map(|r| s.spawn(move || (path, (pass.reload)(r))))
                    })
                    .collect();
                jobs.into_iter().map(|j| j.join().unwrap()).find_map(|(path, result)| result.err().map(|e| (path, e)))
            });
            // An edits pass's module that fails naga was created from the last working text.
            let failed = failed.or_else(|| if pass.edits { edits::take_error().map(|e| ("naga", Error::Shader(e))) } else { None });
            if let Some((path, error)) = failed {
                println!("lightbench: {} on {path}: shader error; keeping the last working pipelines:
{error}", pass.name);
                if pass.edits {
                    edits::settle(false);
                } else {
                    let mut text = TEXT.lock().unwrap();
                    for (file, old) in previous.iter().filter(|(f, _)| pass.files.contains(&f.as_str())) {
                        match old {
                            Some(old) => text.insert(file.clone(), old.clone()),
                            None => text.remove(file),
                        };
                    }
                }
                // Renderers rebuilt before the failing one go back to the working text.
                for rig in rigs.iter_mut().filter(|rig| rig.reads(pass)) {
                    if let Some(r) = rig.r.as_mut() {
                        let _ = (pass.reload)(r);
                    }
                }
            } else if pass.edits {
                edits::settle(true);
            }
        }
        let reload_s = t.elapsed().as_secs_f64();
        let (summary, end) = compiles(&compile_log, compiled_to);
        compiled_to = end;
        println!("lightbench: reloaded in {reload_s:.2} s: {summary}");
        let fixtures_s = run_round(&mut rigs, &o, round, |rig| reload.iter().any(|p| rig.reads(p)));
        // Counted after the round: some kernels compile on first use in the fixtures' frames.
        let reached: Vec<String> = edited.iter().map(|f| format!("{f} reached {} modules", edits::reached(f))).collect();
        if !reached.is_empty() {
            println!("lightbench: {}", reached.join(", "));
        }
        for file in edited.iter().filter(|f| edits::reached(f) == 0) {
            println!("lightbench: warning: the edit to {file} reached no module: its numbers are the old kernels' (docs/bench.md)");
        }
        // A module the fixtures' frames compiled (built on first use) failed: its pipelines used
        // the last working text; the rigs rebuild that text before they next run.
        if let Some(error) = edits::take_error() {
            println!("lightbench: shader error in a module built during the round; its pipelines kept the last working text:
{error}");
            edits::settle(false);
            for rig in rigs.iter_mut() {
                let stale: Vec<&'static Pass> = reload.iter().copied().filter(|p| p.edits && rig.reads(p)).collect();
                rig.stale.extend(stale);
            }
        }
        let total = SystemTime::now().duration_since(saved).map_or(0.0, |d| d.as_secs_f64());
        let names: Vec<&str> = reload.iter().map(|p| p.name).collect();
        println!("lightbench: round {round}: save to numbers {total:.2} s (reload {} {reload_s:.2} s, fixtures {fixtures_s:.2} s)", names.join(","));
        save(&rigs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lease directory with `owner` as its owner file (none when `owner` is None).
    fn lease(root: &Path, name: &str, owner: Option<&str>) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(text) = owner {
            std::fs::write(dir.join("owner.json"), text).unwrap();
        }
        dir
    }

    #[test]
    fn the_bench_idles_under_gates_timing_and_exclusive_leases_only() {
        let root = std::env::temp_dir().join(format!("lightbench-quiet-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let free = (root.join("no-hold"), root.join("no-lock"));
        assert_eq!(quiet_wanted(&free.0, &free.1), None, "nothing held");
        let script = lease(&root, "script", Some(r#"{"owner":"shade","label":"timing-hold furnace","exclusive":false}"#));
        assert_eq!(quiet_wanted(&script, &free.1), None, "a one-off script's ordinary lease");
        let gate = lease(&root, "gate", Some(r#"{"owner":"gicut","label":"timing-hold play b","exclusive":false,"gate":"play"}"#));
        assert_eq!(quiet_wanted(&gate, &free.1).as_deref(), Some("gicut's gate (play: timing-hold play b)"));
        let lock = lease(&root, "lock", Some(r#"{"owner":"main","label":"smoke b","gate":"smoke"}"#));
        assert!(quiet_wanted(&script, &lock).is_some(), "a gate's GPU lock beside an ordinary hold");
        let timing = lease(&root, "timing", Some(r#"{"owner":"sunrt","label":"timing-hold n8"}"#));
        assert!(quiet_wanted(&timing, &free.1).is_some(), "a timing lease");
        let exclusive = lease(&root, "exclusive", Some(r#"{"owner":"clipmap","label":"scoreboard","exclusive":true}"#));
        assert!(quiet_wanted(&exclusive, &free.1).is_some(), "an exclusive lease");
        let unwritten = lease(&root, "unwritten", None);
        assert!(quiet_wanted(&unwritten, &free.1).is_some(), "a hold between its mkdir and its owner file");
        let _ = std::fs::remove_dir_all(&root);
    }
}
