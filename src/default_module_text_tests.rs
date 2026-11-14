//! CPU-only measurement (U-RTVIS, 2026-11-14): the default traced shading kernels' text and
//! SPIR-V size, so two commits' default modules can be compared. The standard program on both
//! paths' heads, composed as the renderer composes them with every switch unset.
//!
//!     RENDERLAB_MODULE_DUMP=<dir> cargo test --lib default_module_text -- --ignored --nocapture
//!
//! writes `<path>-<kernel>.wgsl` per kernel and `summary.txt` (bytes, SHA-256 of the text, naga
//! function count, SPIR-V words per entry point).
use super::*;
use sha2::{Digest, Sha256};

#[test]
#[ignore = "measurement: writes the default modules to RENDERLAB_MODULE_DUMP"]
fn default_module_text() {
    let dir = std::path::PathBuf::from(
        std::env::var_os("RENDERLAB_MODULE_DUMP").expect("set RENDERLAB_MODULE_DUMP=<dir>"),
    );
    std::fs::create_dir_all(&dir).unwrap();
    let scene = include_str!("../shaders/scene.wgsl");
    let program = crate::STANDARD_PROGRAM;
    let parameters = crate::program::declares_parameters(program);
    let rt = shading_heads(scene).unwrap();
    let (_, non_rt) = non_rt_sources(scene).unwrap();
    let mut summary = String::new();
    for (path, heads) in [("rt", rt), ("nonrt", non_rt)] {
        for (kernel, head) in heads.iter().enumerate() {
            let source = crate::program::with_reflection_history(crate::program::compose(
                head,
                program,
                crate::program::Stage::Traced,
                parameters,
            ))
            .source;
            std::fs::write(dir.join(format!("{path}-{kernel}.wgsl")), &source).unwrap();
            let module = naga::front::wgsl::parse_str(&source).unwrap();
            let info = naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
            let mut words = Vec::new();
            for entry in &module.entry_points {
                let (resolved, resolved_info) = naga::back::pipeline_constants::process_overrides(
                    &module,
                    &info,
                    Some((entry.stage, &entry.name)),
                    &Default::default(),
                )
                .unwrap();
                let binary = naga::back::spv::write_vec(
                    &resolved,
                    &resolved_info,
                    &naga::back::spv::Options {
                        lang_version: (1, 3),
                        ..Default::default()
                    },
                    Some(&naga::back::spv::PipelineOptions {
                        shader_stage: entry.stage,
                        entry_point: entry.name.clone(),
                    }),
                )
                .unwrap();
                words.push(format!("{}={}", entry.name, binary.len()));
            }
            let line = format!(
                "{path}-{kernel}: bytes={} sha256={:x} functions={} spirv_words {}\n",
                source.len(),
                Sha256::digest(source.as_bytes()),
                module.functions.len(),
                words.join(" "),
            );
            print!("{line}");
            summary.push_str(&line);
        }
    }
    std::fs::write(dir.join("summary.txt"), summary).unwrap();
}
