//! Caller-owned polling of engine and registered program files, compared by content. Each
//! changed file reloads in its own transaction, and a rejection keeps the last working
//! pipelines. A rejected file is retried after the next successful reload of another
//! file, so a program that needs a new prelude helper applies once that helper lands.
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

struct File {
    path: PathBuf,
    kind: Kind,
    /// The text last offered to the renderer.
    text: String,
    rejected: bool,
}

/// One reload attempt.
pub struct Reload {
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
                },
                File {
                    path: prelude,
                    kind: Kind::Prelude,
                    text: prelude_text,
                    rejected: false,
                },
            ],
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
        };
        if let Some(existing) = self.files.iter_mut().find(|f| f.kind == Kind::Prelude) {
            *existing = file;
        } else {
            self.files.push(file);
        }
    }

    /// A watcher for registered programs only, without engine-file watches.
    pub fn programs_only() -> Self {
        Self { files: vec![] }
    }

    /// Whether a file's current text is rejected, so the last working pipelines render.
    pub fn rejected(&self) -> bool {
        self.files.iter().any(|f| f.rejected)
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
            kind: file.kind.clone(),
            path: file.path.clone(),
            result,
            detected,
            file_age,
            retry,
        }
    }
}
