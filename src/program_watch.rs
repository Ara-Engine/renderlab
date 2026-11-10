//! Caller-owned polling of engine and registered program files, compared by content. Each
//! changed file reloads in its own transaction, and a rejection keeps the last working
//! pipelines. A rejected file is retried after the next successful reload of another
//! file, so a program that needs a new prelude helper applies once that helper lands.
use crate::shader_compiler::CompilePolicy;
use crate::{Program, ReloadError, ReloadReport, Renderer};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `Renderer::reload`: the prelude and every program recomposed against it.
    Prelude,
    /// `Renderer::reload_program` on the standard program.
    Standard,
    /// A caller-registered program. The watcher retains its owned handle.
    Program(Program),
}
impl Kind {
    fn program(&self, renderer: &Renderer) -> Option<Program> {
        match self {
            Self::Prelude => None,
            Self::Standard => Some(renderer.standard_program().clone()),
            Self::Program(p) => Some(p.clone()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReloadStage {
    Preview,
    Optimized,
}

struct File {
    path: PathBuf,
    kind: Kind,
    /// The text last offered to the renderer.
    text: String,
    rejected: bool,
    queued: Option<Change>,
    optimization: Option<Optimization>,
}

#[derive(Clone, Copy)]
struct Change {
    detected: Instant,
    file_age: Duration,
    retry: bool,
}
struct Optimization {
    text: String,
    change: Change,
    after_frame: u64,
}

struct Pending {
    index: usize,
    text: String,
    change: Change,
    job: crate::renderer::ProgramReload,
    stage: ReloadStage,
    upgrade: bool,
}

/// One reload attempt.
pub struct Reload {
    pub stage: ReloadStage,
    pub kind: Kind,
    pub path: PathBuf,
    pub result: Result<ReloadReport, ReloadError>,
    /// When the change was read, and the file's age then: save to detection, which the
    /// poll interval bounds.
    pub detected: Instant,
    pub file_age: Duration,
    pub retry: bool,
}

pub struct ShaderWatch {
    files: Vec<File>,
    pending: Option<Pending>,
    preview: Option<CompilePolicy>,
}

impl ShaderWatch {
    /// `prelude_text` is what the renderer was created with; the standard program starts
    /// as the built-in source, so an edited file on disk applies on the first poll.
    pub fn new(prelude: PathBuf, prelude_text: String, standard: PathBuf) -> Self {
        Self {
            files: vec![
                File {
                    path: standard,
                    kind: Kind::Standard,
                    text: crate::STANDARD_PROGRAM.into(),
                    rejected: false,
                    queued: None,
                    optimization: None,
                },
                File {
                    path: prelude,
                    kind: Kind::Prelude,
                    text: prelude_text,
                    rejected: false,
                    queued: None,
                    optimization: None,
                },
            ],
            pending: None,
            preview: None,
        }
    }

    /// Watches a program's file, using its last accepted source as the content baseline.
    /// Call after `try_create_program`; no I/O or compilation happens here. Registering
    /// the same handle again replaces its file. Call `poll` on the renderer that owns it.
    /// The caller chooses the polling cadence (the viewer uses 50 ms).
    pub fn watch_program(
        &mut self,
        program: &Program,
        path: impl Into<PathBuf>,
        accepted_source: &str,
    ) {
        let kind = Kind::Program(program.clone());
        let file = File {
            path: path.into(),
            kind: kind.clone(),
            text: accepted_source.into(),
            rejected: false,
            queued: None,
            optimization: None,
        };
        if let Some(existing) = self.files.iter_mut().find(|f| f.kind == kind) {
            *existing = file;
        } else {
            let prelude = self
                .files
                .iter()
                .position(|f| f.kind == Kind::Prelude)
                .unwrap_or(self.files.len());
            self.files.insert(prelude, file);
        }
    }

    /// Registers the engine prelude, using its accepted source as the baseline.
    /// Re-registering replaces the prelude watch. Programs always poll before it.
    pub fn watch_prelude(&mut self, path: impl Into<PathBuf>, accepted_source: &str) {
        let file = File {
            path: path.into(),
            kind: Kind::Prelude,
            text: accepted_source.into(),
            rejected: false,
            queued: None,
            optimization: None,
        };
        if let Some(existing) = self.files.iter_mut().find(|f| f.kind == Kind::Prelude) {
            *existing = file;
        } else {
            self.files.push(file);
        }
    }

    /// A watcher for registered programs only, without engine-file watches.
    pub fn programs_only() -> Self {
        Self {
            files: vec![],
            pending: None,
            preview: None,
        }
    }

    /// Whether a file's current text is rejected, so the last working pipelines render.
    pub fn rejected(&self) -> bool {
        self.files.iter().any(|f| f.rejected)
    }

    /// Opt-in, transient development output. Configure while idle on a device made
    /// by shader_compiler::request_device. A preview is presented for at least one
    /// submitted frame before its optimized replacement starts. Temporal frames
    /// always use optimized pipelines. SpirvPreview also supports optimized builds
    /// for measuring edit latency; ordinary callers keep the default None.
    pub fn set_preview_policy(
        &mut self,
        policy: Option<CompilePolicy>,
    ) -> Result<(), crate::Error> {
        if policy.is_some()
            && ((!cfg!(debug_assertions) && policy != Some(CompilePolicy::SpirvPreview))
                || !matches!(
                    policy,
                    Some(
                        CompilePolicy::SpirvPreview
                            | CompilePolicy::Preview
                            | CompilePolicy::PreviewHelpers
                            | CompilePolicy::PreviewNoInline
                            | CompilePolicy::LibraryFast
                    )
                ))
        {
            return Err(crate::Error::InvalidInput(
                "shader preview requires a supported policy; native hint policies require a development build",
            ));
        }
        if self.pending.is_some() || self.files.iter().any(|f| f.optimization.is_some()) {
            return Err(crate::Error::InvalidInput(
                "change shader compile policy after the current reload finishes",
            ));
        }
        self.preview = policy;
        Ok(())
    }

    /// The standard program beside a prelude: `<prelude dir>/programs/standard.wgsl`.
    pub fn standard_beside(prelude: &Path) -> PathBuf {
        prelude.with_file_name("programs").join("standard.wgsl")
    }

    /// Reads every file and reloads those whose text changed, programs first; then, if
    /// any reload succeeded, retries the files still rejected. Unreadable files (a save in
    /// progress, a missing optional file) are skipped until they read.
    pub fn poll(&mut self, renderer: &mut Renderer) -> Vec<Reload> {
        let mut reloads = vec![];
        for i in 0..self.files.len() {
            let file = &mut self.files[i];
            let Ok(text) = std::fs::read_to_string(&file.path) else {
                continue;
            };
            if text == file.text {
                continue;
            }
            let detected = Instant::now();
            let file_age = std::fs::metadata(&file.path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|saved| SystemTime::now().duration_since(saved).ok())
                .unwrap_or_default();
            file.text = text;
            reloads.push(self.reload(i, renderer, detected, file_age, false));
        }
        if reloads.iter().any(|r| r.result.is_ok()) {
            for i in 0..self.files.len() {
                if self.files[i].rejected {
                    reloads.push(self.reload(i, renderer, Instant::now(), Duration::ZERO, true));
                }
            }
        }
        reloads
    }

    /// Nonblocking program reloads: one owned compile job, with newer saves coalesced
    /// per file. Frames keep using the previous revision. Finished results install
    /// between frames only if both file contents and renderer snapshot still match.
    /// Prelude edits use the existing synchronous transaction after the worker settles.
    /// Do not mix this with `poll`, or re-register watches while a job is pending.
    pub fn poll_async(&mut self, renderer: &mut Renderer) -> Vec<Reload> {
        for file in &mut self.files {
            let Ok(text) = std::fs::read_to_string(&file.path) else {
                continue;
            };
            if text != file.text {
                file.text = text;
                let file_age = std::fs::metadata(&file.path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|saved| SystemTime::now().duration_since(saved).ok())
                    .unwrap_or_default();
                file.queued = Some(Change {
                    detected: Instant::now(),
                    file_age,
                    retry: false,
                });
            }
        }
        let mut reloads = vec![];
        if self.pending.as_ref().is_some_and(|p| p.job.is_finished()) {
            let pending = self.pending.take().unwrap();
            let file = &mut self.files[pending.index];
            let matches = if pending.upgrade {
                file.kind
                    .program(renderer)
                    .is_some_and(|p| renderer.program_source_matches(&p, &pending.text))
            } else {
                pending.text == file.text
            };
            if matches && pending.job.current(renderer) {
                let change = if file.text == pending.text {
                    file.queued.take().unwrap_or(pending.change)
                } else {
                    pending.change
                };
                let result = renderer.finish_program_reload(pending.job);
                if file.text == pending.text {
                    file.rejected = matches!(result, Err(ReloadError::Rejected { .. }));
                }
                if result.is_ok() && pending.stage == ReloadStage::Preview {
                    file.optimization = Some(Optimization {
                        text: pending.text,
                        change,
                        after_frame: renderer.stats().frames,
                    });
                }
                reloads.push(Reload {
                    stage: pending.stage,
                    kind: file.kind.clone(),
                    path: file.path.clone(),
                    result,
                    detected: change.detected,
                    file_age: change.file_age,
                    retry: change.retry,
                });
            } else {
                // A renderer change needs a fresh snapshot even when the file did not
                // change. Preserve the original save timestamp for latency accounting.
                if pending.upgrade {
                    if matches {
                        file.optimization = Some(Optimization {
                            text: pending.text,
                            change: pending.change,
                            after_frame: renderer.stats().frames.saturating_sub(1),
                        });
                    }
                } else {
                    file.queued.get_or_insert(pending.change);
                }
                pending.job.discard();
            }
        }
        if reloads.iter().any(|r| r.result.is_ok()) {
            for file in &mut self.files {
                if file.rejected && file.queued.is_none() {
                    file.queued = Some(Change {
                        detected: Instant::now(),
                        file_age: Duration::ZERO,
                        retry: true,
                    });
                }
            }
        }
        if self.pending.is_none()
            && let Some(index) = self.files.iter().position(|f| f.queued.is_some())
        {
            let file = &mut self.files[index];
            let change = file.queued.take().unwrap();
            let program = file.kind.program(renderer);
            if let Some(program) = program {
                let preview = self.preview.filter(|_| renderer.can_preview_programs());
                let stage = if preview.is_some() {
                    ReloadStage::Preview
                } else {
                    ReloadStage::Optimized
                };
                match renderer.start_program_reload_with_policy(
                    &program,
                    &file.text,
                    preview.unwrap_or_default(),
                ) {
                    Ok(job) => {
                        self.pending = Some(Pending {
                            index,
                            text: file.text.clone(),
                            change,
                            job,
                            stage,
                            upgrade: false,
                        })
                    }
                    Err(result) => reloads.push(Reload {
                        stage,
                        kind: file.kind.clone(),
                        path: file.path.clone(),
                        result: Err(result),
                        detected: change.detected,
                        file_age: change.file_age,
                        retry: change.retry,
                    }),
                }
            } else {
                let reload = self.reload(
                    index,
                    renderer,
                    change.detected,
                    change.file_age,
                    change.retry,
                );
                if reload.result.is_ok() {
                    for file in &mut self.files {
                        if file.rejected && file.queued.is_none() {
                            file.queued = Some(Change {
                                detected: Instant::now(),
                                file_age: Duration::ZERO,
                                retry: true,
                            });
                        }
                    }
                }
                reloads.push(reload);
            }
        }
        if self.pending.is_none() && !self.files.iter().any(|f| f.queued.is_some()) {
            let frames = renderer.stats().frames;
            for file in &mut self.files {
                if let Some(plan) = &file.optimization
                    && !file
                        .kind
                        .program(renderer)
                        .is_some_and(|p| renderer.program_source_matches(&p, &plan.text))
                {
                    file.optimization = None;
                }
            }
            let next = self
                .files
                .iter()
                .enumerate()
                .filter_map(|(i, f)| {
                    f.optimization
                        .as_ref()
                        .filter(|p| frames > p.after_frame)
                        .map(|p| (i, p.change.detected))
                })
                .min_by_key(|(_, saved)| *saved);
            if let Some((index, _)) = next {
                let file = &mut self.files[index];
                let plan = file.optimization.take().unwrap();
                let program = file.kind.program(renderer).unwrap();
                let policy = if self.preview == Some(CompilePolicy::LibraryFast) {
                    CompilePolicy::LibraryLto
                } else {
                    CompilePolicy::Optimized
                };
                match renderer.start_program_reload_with_policy(&program, &plan.text, policy) {
                    Ok(job) => {
                        self.pending = Some(Pending {
                            index,
                            text: plan.text,
                            change: plan.change,
                            job,
                            stage: ReloadStage::Optimized,
                            upgrade: true,
                        })
                    }
                    Err(error) => reloads.push(Reload {
                        stage: ReloadStage::Optimized,
                        kind: file.kind.clone(),
                        path: file.path.clone(),
                        result: Err(error),
                        detected: plan.change.detected,
                        file_age: plan.change.file_age,
                        retry: plan.change.retry,
                    }),
                }
            }
        }
        reloads
    }

    fn reload(
        &mut self,
        i: usize,
        renderer: &mut Renderer,
        detected: Instant,
        file_age: Duration,
        retry: bool,
    ) -> Reload {
        let file = &mut self.files[i];
        let result = match &file.kind {
            Kind::Prelude => renderer.reload(&file.text),
            Kind::Standard => {
                let standard = renderer.standard_program().clone();
                renderer.reload_program(&standard, &file.text)
            }
            Kind::Program(program) => renderer.reload_program(program, &file.text),
        };
        file.rejected = matches!(result, Err(ReloadError::Rejected { .. }));
        Reload {
            stage: ReloadStage::Optimized,
            kind: file.kind.clone(),
            path: file.path.clone(),
            result,
            detected,
            file_age,
            retry,
        }
    }
}

impl Drop for ShaderWatch {
    fn drop(&mut self) {
        // Compile jobs never outlive their owner, including when a viewer closes
        // halfway through an edit. This wait is only on watcher shutdown.
        if let Some(pending) = self.pending.take() {
            pending.job.discard();
        }
    }
}
