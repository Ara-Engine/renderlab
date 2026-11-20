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

mod fixtures;
mod furnace;
pub(crate) mod kit;

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
}

/// Shader text read from disk by the bench, by file name under shaders/: `program::gi_shader`
/// composes it instead of the built-in text.
static TEXT: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());
pub(crate) fn shader_text(name: &str) -> Option<String> {
    TEXT.lock().unwrap().get(name).cloned()
}

/// A pass the bench can rebuild: the shader files it composes and its rebuild.
struct Pass {
    name: &'static str,
    files: &'static [&'static str],
    reload: fn(&mut Renderer) -> Result<(), Error>,
}
/// Files whose text reaches the program shading kernels through compile-time constants
/// (program.rs) are not here yet: editing them prints that the bench cannot reload them.
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
}];

struct Options {
    once: bool,
    paths: Vec<BenchPath>,
    only: Option<Vec<String>>,
    shaders: PathBuf,
    hold: PathBuf,
    refs: PathBuf,
    runs: PathBuf,
}
fn options() -> Result<Options, String> {
    let env = |k: &str, default: &str| PathBuf::from(std::env::var(k).unwrap_or_else(|_| default.to_owned()));
    let mut o = Options {
        once: false,
        paths: vec![BenchPath::NoRt, BenchPath::Rt],
        only: None,
        shaders: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders"),
        hold: env("LIGHTBENCH_HOLD", "D:/Coding/renderlab-work/timing-hold"),
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
    device: (wgpu::Device, wgpu::Queue),
    fixture: Box<dyn Fixture>,
    r: Option<Renderer>,
    reference: Option<RefImage>,
}
impl Rig {
    fn build(&mut self, refs: &Path) -> Result<(), Error> {
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

/// Waits while a timing-hold exists (someone's timing row needs the machine quiet), saying so.
fn wait_quiet(hold: &Path) {
    if !hold.exists() {
        return;
    }
    let owner = std::fs::read_to_string(hold.join("owner.json")).unwrap_or_default();
    println!("lightbench: idle, a timing-hold is held: {}", owner.trim());
    while hold.exists() {
        std::thread::sleep(Duration::from_millis(500));
    }
    println!("lightbench: the timing-hold is gone, resuming");
}

/// Each pass's median GPU milliseconds over the pass-log lines from `offset` on (frames that
/// compiled or were dropped left out), and the end of the log.
fn pass_medians(log: &Path, offset: u64) -> (BTreeMap<String, f64>, u64) {
    let Ok(text) = std::fs::read(log) else { return (BTreeMap::new(), offset) };
    let end = text.len() as u64;
    let text = String::from_utf8_lossy(&text[(offset.min(end)) as usize..]).into_owned();
    let mut samples: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for line in text.lines() {
        if line.contains("\"dropped\":true") || !line.contains("\"compiles\":0,") && line.contains("\"compiles\":") {
            continue;
        }
        let mut add = |key: &str, value: f64| samples.entry(key.to_owned()).or_default().push(value);
        if let Some(span) = field(line, "\"span\":") {
            add("span", span);
        }
        let Some(start) = line.find("\"passes\":{") else { continue };
        let body = &line[start + 10..];
        let Some(close) = body.find('}') else { continue };
        for item in body[..close].split(',') {
            if let Some((k, v)) = item.split_once(':') {
                if let Ok(v) = v.trim().parse::<f64>() {
                    add(k.trim().trim_matches('"'), v);
                }
            }
        }
    }
    let medians = samples
        .into_iter()
        .map(|(k, mut v)| {
            v.sort_by(f64::total_cmp);
            (k, v[v.len() / 2])
        })
        .collect();
    (medians, end)
}
fn field(line: &str, key: &str) -> Option<f64> {
    let rest = &line[line.find(key)? + key.len()..];
    rest[..rest.find([',', '}'])?].trim().parse().ok()
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

/// Runs every rig once and prints its lines. Returns the seconds the fixtures took.
fn run_round(rigs: &mut [Rig], o: &Options, log: &Path, round: u32) -> f64 {
    wait_quiet(&o.hold);
    let started = Instant::now();
    let mut offset = std::fs::metadata(log).map_or(0, |m| m.len());
    for rig in rigs.iter_mut() {
        if rig.r.is_none() {
            if let Err(error) = rig.build(&o.refs) {
                println!("bench {} {} error: could not build the fixture: {error}", rig.fixture.name(), rig.path.name());
                continue;
            }
        }
        let r = rig.r.as_mut().unwrap();
        let t = Instant::now();
        let result = rig.fixture.run(r, rig.path, rig.reference.as_ref());
        let _ = kit::drain(r);
        let seconds = t.elapsed().as_secs_f64();
        let (passes, end) = pass_medians(log, offset);
        offset = end;
        let name = rig.fixture.name().to_owned();
        match result {
            Ok(metrics) => {
                let shown: Vec<String> = metrics.iter().map(|(k, v)| format!("{k}={v:.4}")).collect();
                let pass_text: Vec<String> = passes.iter().map(|(k, v)| format!("{k}={v:.2}")).collect();
                println!("bench {name} {} {} | ms {} | {seconds:.2} s", rig.path.name(), shown.join(" "), pass_text.join(" "));
                let passes: Vec<(String, f64)> = passes.into_iter().collect();
                println!(
                    "{{\"bench\":\"lightbench\",\"round\":{round},\"fixture\":{},\"path\":\"{}\",\"metrics\":{},\"pass_ms\":{},\"run_s\":{seconds:.3}}}",
                    json_string(&name),
                    rig.path.name(),
                    json_map(&metrics),
                    json_map(&passes)
                );
            }
            Err(error) => {
                println!("bench {name} {} error: {error}", rig.path.name());
                if matches!(error, Error::Fault(_)) {
                    // A faulted renderer is rebuilt on the next round.
                    rig.r = None;
                }
            }
        }
    }
    started.elapsed().as_secs_f64()
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
    // Per-pass GPU times come from the renderers' own pass log (src/pass_timeline.rs), which
    // each renderer opens when it is created.
    let log = o.runs.join(format!("passes-{}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&log);
    // SAFETY: set before any thread starts; nothing else reads the environment concurrently.
    unsafe { std::env::set_var("RENDERLAB_PASS_LOG", &log) };
    // The passes' sources as they are on disk now, not as this binary was built.
    {
        let mut text = TEXT.lock().unwrap();
        for file in PASSES.iter().flat_map(|p| p.files) {
            if let Ok(source) = std::fs::read_to_string(o.shaders.join(file)) {
                text.insert((*file).to_owned(), source);
            }
        }
    }
    wait_quiet(&o.hold);
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
            if o.only.as_ref().is_some_and(|only| !only.iter().any(|n| n == fixture.name())) {
                continue;
            }
            rigs.push(Rig { path, device: device.clone(), fixture, r: None, reference: None });
        }
    }
    for rig in &mut rigs {
        if let Err(error) = rig.build(&o.refs) {
            println!("bench {} {} error: could not build the fixture: {error}", rig.fixture.name(), rig.path.name());
        }
    }
    println!("lightbench: {} fixtures on {} paths built in {:.1} s; watching {}", rigs.len() / o.paths.len().max(1), o.paths.len(), started.elapsed().as_secs_f64(), o.shaders.display());
    let mut round = 0;
    let seconds = run_round(&mut rigs, &o, &log, round);
    println!("lightbench: round {round}: fixtures {seconds:.2} s");
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
        let mut reload: Vec<&Pass> = vec![];
        let mut previous: Vec<(String, Option<String>)> = vec![];
        for (file, _) in &changed {
            let Some(pass) = PASSES.iter().find(|p| p.files.contains(&file.as_str())) else {
                println!("lightbench: {file} changed; no bench pass reloads it yet (docs/bench.md)");
                continue;
            };
            let Ok(source) = std::fs::read_to_string(o.shaders.join(file)) else { continue };
            let old = TEXT.lock().unwrap().insert(file.clone(), source);
            previous.push((file.clone(), old));
            if !reload.iter().any(|p| p.name == pass.name) {
                reload.push(pass);
            }
        }
        if reload.is_empty() {
            continue;
        }
        wait_quiet(&o.hold);
        round += 1;
        let t = Instant::now();
        for pass in &reload {
            let mut failed = None;
            for rig in rigs.iter_mut() {
                if let Some(r) = rig.r.as_mut() {
                    if let Err(error) = (pass.reload)(r) {
                        failed = Some((rig.path, error));
                        break;
                    }
                }
            }
            if let Some((path, error)) = failed {
                println!("lightbench: {} on {}: shader error; keeping the last working pipelines:\n{error}", pass.name, path.name());
                let mut text = TEXT.lock().unwrap();
                for (file, old) in previous.iter().filter(|(f, _)| pass.files.contains(&f.as_str())) {
                    match old {
                        Some(old) => text.insert(file.clone(), old.clone()),
                        None => text.remove(file),
                    };
                }
                drop(text);
                // Renderers rebuilt before the failing one go back to the working text.
                for rig in rigs.iter_mut() {
                    if let Some(r) = rig.r.as_mut() {
                        let _ = (pass.reload)(r);
                    }
                }
            }
        }
        let reload_s = t.elapsed().as_secs_f64();
        let fixtures_s = run_round(&mut rigs, &o, &log, round);
        let total = SystemTime::now().duration_since(saved).map_or(0.0, |d| d.as_secs_f64());
        let names: Vec<&str> = reload.iter().map(|p| p.name).collect();
        println!("lightbench: round {round}: save to numbers {total:.2} s (reload {} {reload_s:.2} s, fixtures {fixtures_s:.2} s)", names.join(","));
    }
}
