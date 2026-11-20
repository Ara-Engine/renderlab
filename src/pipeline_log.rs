//! Opt-in CPU pipeline/resource events. No GPU readback or timing query is issued.
use crate::{FrameReport, Renderer, ResourceKind};
use std::{fs::File, io::Write, sync::Mutex, time::Duration};

pub(crate) struct PipelineLog(Mutex<File>);

impl PipelineLog {
    pub fn from_env() -> Option<Self> {
        let path = std::env::var_os("RENDERLAB_PIPELINE_LOG")?;
        match std::fs::OpenOptions::new().create(true).append(true).open(path) {
            Ok(file) => Some(Self(Mutex::new(file))),
            Err(error) => {
                eprintln!("pipeline_log: {error}");
                None
            }
        }
    }

    /// A finished compile: its kind, label and time, the wall clock when it ended
    /// (`end_unix_ms`, to lay compiles against an embedder's own timeline) and the thread
    /// that ran it (its name, or its id when it has none).
    pub fn compile(&self, kind: &str, label: &str, duration: Duration) {
        let thread = std::thread::current();
        let thread = thread.name().map_or_else(|| format!("{:?}", thread.id()), str::to_owned);
        self.line(&format!(
            "{{\"event\":\"compile\",\"kind\":{},\"label\":{},\"ms\":{:.6},\"end_unix_ms\":{:.1},\"thread\":{}}}",
            json_string(kind), json_string(label), duration.as_secs_f64() * 1e3, unix_ms(), json_string(&thread),
        ));
    }

    /// A frame-end marker associates preceding compile events with this submission.
    /// The first marker also includes pipelines compiled while loading the engine/scene.
    /// `late` is the count of pipelines compiled since the last prepare.
    pub fn frame(&self, renderer: &Renderer, report: &FrameReport, late: u64) {
        let stats = renderer.stats();
        let owned = &stats.ownership;
        let hw = renderer.hardware_shadow_stats();
        self.line(&format!(
            "{{\"event\":\"frame\",\"frame\":{},\"render_created\":{},\"compute_created\":{},\"shader_created\":{},\"blas_created\":{},\"tlas_created\":{},\"blas_builds\":{},\"blas_updates\":{},\"tlas_builds\":{},\"target_bytes\":{},\"shadow_bytes\":{},\"visible_instances\":{},\"compiles_after_prepare\":{},\"unix_ms\":{:.1}}}",
            report.frame.0,
            owned.get(ResourceKind::RenderPipeline).created,
            owned.get(ResourceKind::ComputePipeline).created,
            owned.get(ResourceKind::ShaderModule).created,
            owned.get(ResourceKind::Blas).created,
            owned.get(ResourceKind::Tlas).created,
            hw.map_or(0, |s| s.blas_builds),
            hw.map_or(0, |s| s.blas_updates),
            hw.map_or(0, |s| s.tlas_builds),
            owned.target_bytes.live,
            owned.shadow_bytes.live,
            report.scene.visible_instances,
            late,
            unix_ms(),
        ));
    }

    fn line(&self, line: &str) {
        // Diagnostic I/O must never reject a frame.
        let _ = writeln!(self.0.lock().unwrap(), "{line}");
    }
}

/// Milliseconds since the Unix epoch: the log's clock, shared with an embedder's own stamps.
fn unix_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64() * 1e3)
}

/// JSON escaping without making serde_json a required renderer dependency.
fn json_string(value: &str) -> String {
    use std::fmt::Write;
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c <= '\u{1f}' => { write!(out, "\\u{:04x}", c as u32).unwrap(); }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_labels_round_trip_in_pipeline_log() {
        let label = "shader \"glass\"\\rail\n\t\0 / ☀";
        assert_eq!(serde_json::from_str::<String>(&json_string(label)).unwrap(), label);
    }
}
