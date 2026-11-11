//! Native Metal/Vulkan replays. All generated files stay in ignored target directories.
#![cfg(feature = "backend-ara")]
#[path = "ara_backend/reference.rs"]
mod reference;
use anyhow::{Context, Result, ensure};
use ara_gpu::{
    Access, Device, GpuDevice, Queue, Resource,
    commands::{Command, Recorder},
    descriptors::Format,
    vk,
};
use half::f16;
use renderlab::backend_ara::{
    cache::{ShaderCache, emit_spirv},
    graph::Graph,
    image::Texture,
    post::{FilterInputs, PostPasses, SettingsRecord, ToneInputs},
    shader::Pass,
};
fn root(label: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("renderlab-ara")
        .join(format!("{label}-{}", std::process::id()))
}
fn half_bytes(pixels: &[[f32; 4]]) -> Vec<u8> {
    pixels
        .iter()
        .flatten()
        .flat_map(|v| f16::from_f32(*v).to_bits().to_le_bytes())
        .collect()
}
fn upload(
    gpu: &mut GpuDevice,
    graph: &mut Graph,
    rec: &mut Recorder,
    image: ara_gpu::Image,
    bytes: &[u8],
) -> Result<ara_gpu::Buffer> {
    let buffer = gpu.create_buffer(bytes.len() as u64, vk::BufferUsageFlags::TRANSFER_SRC, true)?;
    gpu.upload(buffer, bytes)?;
    graph.import(Resource::Buffer(buffer), Access::HostWrite);
    graph.node(
        rec,
        &[
            (Resource::Buffer(buffer), Access::CopySource),
            (Resource::Image(image), Access::CopyDestination),
        ],
    )?;
    rec.push(Command::UploadImage {
        source: buffer,
        destination: image,
    });
    Ok(buffer)
}
#[test]
fn all_ported_stages_emit_bindless_spirv() -> Result<()> {
    let path = root("spirv");
    let mut precise = 0;
    for pass in [
        Pass::BufferArrayProbe,
        Pass::HandleArrayProbe,
        Pass::PrimaryRayProbe,
        Pass::PrimaryTrace,
        Pass::PrimaryTraceReceivers,
        Pass::TransparencySeed,
        Pass::TransparencyGlass,
        Pass::ReceiverRaster,
        Pass::FreshVisibility,
        Pass::ShadowRaster,
        Pass::TransparencySunShadow,
        Pass::TransmissionShadow,
        Pass::FallbackPresent,
        Pass::VisibilitySoftware,
        Pass::VisibilityHardware,
        Pass::VisibilityResolve,
        Pass::PostPrimarySurface,
        Pass::TransparencyHitDepth,
        Pass::PrimaryCheck,
        Pass::PrimaryShadeTiles,
        Pass::PrimaryShadeList,
        Pass::VisibilityCull,
        Pass::VisibilityFinish,
        Pass::VisibilityReset,
        Pass::ProgramBins,
        Pass::HzbBase,
        Pass::HzbReduce,
        Pass::ParticleBillboard,
        Pass::ParticleUpdate,
        Pass::MeshToon,
        Pass::TransparencyDepth,
        Pass::MeshDiagnostic,
        Pass::ParticleSort,
        Pass::Downsample,
        Pass::Upsample,
        Pass::Tone,
        Pass::Display,
        Pass::TileMax,
        Pass::MotionBlur,
        Pass::DepthOfField,
        Pass::LightCluster,
        Pass::AtmosphereBackdrop,
        Pass::AtmosphereAerial,
        Pass::AtmosphereFogIntegrate,
        Pass::AtmosphereTransmittance,
        Pass::AtmosphereMultiscatter,
        Pass::AtmosphereSky,
        Pass::AtmosphereAmbient,
        Pass::EnvironmentSpecular,
        Pass::EnvironmentDiffuse,
        Pass::SkinVertices,
        Pass::SkinClusters,
        Pass::RasterSurface,
        Pass::RasterSurfaceFour,
    ] {
        let stages = emit_spirv(pass, &path).map_err(|e| anyhow::anyhow!("{pass:?}: {e:#}"))?;
        for stage in &stages {
            let bytes = std::fs::read(path.join(&stage.spirv))?;
            ensure!(
                bytes.len() >= 20 && bytes.len() % 4 == 0,
                "invalid SPIR-V length"
            );
            let words: Vec<_> = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| u32::from_le_bytes(*b))
                .collect();
            assert_nonuniform_sampled_images(&words)?;
            let mut at = 5;
            let mut entries = Vec::new();
            while at < words.len() {
                let n = (words[at] >> 16) as usize;
                ensure!(n > 0 && at + n <= words.len(), "invalid SPIR-V instruction");
                if words[at] & 0xffff == 15 {
                    ensure!(n >= 4, "short OpEntryPoint");
                    let name: Vec<_> = words[at + 3..at + n]
                        .iter()
                        .flat_map(|w| w.to_le_bytes())
                        .take_while(|b| *b != 0)
                        .collect();
                    entries.push(String::from_utf8(name)?);
                }
                at += n;
            }
            ensure!(
                entries == [stage.spirv_entry.clone()],
                "{pass:?} native entry ABI mismatch: {entries:?} != {}",
                stage.spirv_entry
            );
            precise += assert_precise_arithmetic(&words)?;
        }
        if let Some(sdk) = std::env::var_os("VULKAN_SDK") {
            let sdk = std::path::PathBuf::from(sdk);
            let validator = if cfg!(windows) {
                sdk.join("Bin/spirv-val.exe")
            } else {
                sdk.join("bin/spirv-val")
            };
            let validator = if validator.is_file() {
                validator
            } else {
                std::path::PathBuf::from("spirv-val")
            };
            for stage in stages {
                let output = std::process::Command::new(&validator)
                    .args(["--target-env", "vulkan1.3"])
                    .arg(path.join(stage.spirv))
                    .output()?;
                ensure!(
                    output.status.success(),
                    "spirv-val failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
    ensure!(
        precise > 1000,
        "the cohort lost its float arithmetic: {precise}"
    );
    println!(
        "SPIR-V output: {}; {precise} float arithmetic results, all NoContraction",
        path.display()
    );
    Ok(())
}

#[path = "ara_backend/clusters.rs"]
mod clusters;

#[path = "ara_backend/atmosphere.rs"]
mod atmosphere_reference;

#[path = "ara_backend/environment.rs"]
mod environment_reference;

#[path = "ara_backend/skinning.rs"]
mod skinning_reference;

#[path = "ara_backend/cases/post.rs"]
mod post_cases;

#[path = "ara_backend/cases/filters.rs"]
mod filters_cases;

#[path = "ara_backend/cases/clusters.rs"]
mod clusters_cases;

#[path = "ara_backend/cases/atmosphere.rs"]
mod atmosphere_cases;

#[path = "ara_backend/cases/environment.rs"]
mod environment_cases;

#[path = "ara_backend/cases/skinning.rs"]
mod skinning_cases;

#[path = "ara_backend/cases/post_surface.rs"]
mod post_surface_cases;

#[path = "ara_backend/cases/mesh.rs"]
mod mesh_cases;
#[path = "ara_backend/mesh.rs"]
mod mesh_reference;
#[path = "ara_backend/cases/particles.rs"]
mod particle_cases;
#[path = "ara_backend/cases/post_runtime.rs"]
mod post_runtime_cases;
#[path = "ara_backend/cases/sky.rs"]
mod sky_cases;
#[path = "ara_backend/sky.rs"]
mod sky_reference;
#[path = "ara_backend/cases/toon.rs"]
mod toon_cases;
#[path = "ara_backend/toon.rs"]
mod toon_reference;
#[path = "ara_backend/cases/transparency_depth.rs"]
mod transparency_depth_cases;
#[path = "ara_backend/cases/volumes.rs"]
mod volume_cases;
#[path = "ara_backend/volumes.rs"]
mod volume_reference;

#[path = "ara_backend/cases/particle_update.rs"]
mod particle_update_cases;
#[path = "ara_backend/particle_update.rs"]
mod particle_update_reference;

#[path = "ara_backend/cases/particle_draw.rs"]
mod particle_draw_cases;
#[path = "ara_backend/particle_draw.rs"]
mod particle_draw_reference;

#[path = "ara_backend/cases/hzb.rs"]
mod hzb_cases;

#[path = "ara_backend/cases/program_bins.rs"]
mod program_bins_cases;
#[path = "ara_backend/program_bins.rs"]
mod program_bins_reference;

#[path = "ara_backend/cases/visibility_cull.rs"]
mod visibility_cull_cases;
#[path = "ara_backend/visibility_cull.rs"]
mod visibility_cull_reference;

#[path = "ara_backend/cases/graph.rs"]
mod graph_cases;

#[cfg(target_os = "macos")]
#[path = "ara_backend/cases/buffer_arrays.rs"]
mod buffer_array_cases;
// Compile the pure adapter tests here as well: the repository's unrelated lib-test
// target currently hits an objc2 inference recursion error on this Mac.
#[path = "../src/backend_ara/shader/array_access.rs"]
mod array_access_tests;
#[path = "ara_backend/cases/visibility_raster.rs"]
mod visibility_raster_cases;
#[path = "ara_backend/visibility_sw_reference.rs"]
mod visibility_sw_reference;

#[path = "ara_backend/cases/visibility_resolve.rs"]
mod visibility_resolve_cases;
#[path = "ara_backend/visibility_resolve_reference.rs"]
mod visibility_resolve_reference;

#[path = "ara_backend/cases/post_primary.rs"]
mod post_primary_cases;
#[path = "ara_backend/post_primary_reference.rs"]
mod post_primary_reference;

#[path = "ara_backend/cases/transparency_hit_depth.rs"]
mod transparency_hit_depth_cases;
#[path = "ara_backend/transparency_hit_depth_reference.rs"]
mod transparency_hit_depth_reference;

#[path = "ara_backend/cases/primary_check.rs"]
mod primary_check_cases;
#[path = "ara_backend/primary_check_reference.rs"]
mod primary_check_reference;

#[path = "ara_backend/cases/handle_arrays.rs"]
mod handle_array_cases;

#[path = "ara_backend/cases/primary_shade.rs"]
mod primary_shade_cases;
#[path = "ara_backend/primary_shade_reference.rs"]
mod primary_shade_reference;

/// Every float arithmetic result must be decorated NoContraction (42), so a Vulkan
/// driver fuses nothing the Metal compile, which runs with contraction off, keeps
/// apart. Returns how many results there were.
fn assert_precise_arithmetic(words: &[u32]) -> Result<usize> {
    let (mut arithmetic, mut precise) = (Vec::new(), std::collections::BTreeSet::new());
    let mut at = 5;
    while at < words.len() {
        let n = (words[at] >> 16) as usize;
        ensure!(n > 0 && at + n <= words.len(), "invalid SPIR-V instruction");
        match words[at] & 0xffff {
            71 if n == 3 && words[at + 2] == 42 => {
                precise.insert(words[at + 1]);
            }
            127 | 129 | 131 | 133 | 136 | 140 | 141 | 142..=148 => arithmetic.push(words[at + 2]),
            _ => {}
        }
        at += n;
    }
    ensure!(
        arithmetic.iter().all(|id| precise.contains(id)) && precise.len() == arithmetic.len(),
        "{} float arithmetic results, {} NoContraction decorations",
        arithmetic.len(),
        precise.len()
    );
    Ok(arithmetic.len())
}

// Inspect the final operand consumed by sampling; spirv-val does not enforce this
// semantic descriptor-indexing rule. Independent from the production repair pass.
fn assert_nonuniform_sampled_images(words: &[u32]) -> Result<()> {
    let mut decorations = std::collections::BTreeSet::new();
    let mut combined = Vec::new();
    let mut at = 5;
    while at < words.len() {
        let n = (words[at] >> 16) as usize;
        ensure!(n > 0 && at + n <= words.len(), "invalid SPIR-V instruction");
        match words[at] & 0xffff {
            71 if n == 3 && words[at + 2] == 5300 => {
                decorations.insert(words[at + 1]);
            }
            86 => {
                ensure!(n == 5, "short sampled image");
                combined.push(&words[at + 1..at + n]);
            }
            _ => {}
        }
        at += n;
    }
    for values in combined {
        if decorations.contains(&values[2]) || decorations.contains(&values[3]) {
            ensure!(
                decorations.contains(&values[1]),
                "sampled image {} lost divergent image/sampler identity",
                values[1]
            );
        }
    }
    Ok(())
}

#[path = "ara_backend/cases/primary_ray.rs"]
mod primary_ray_cases;
#[path = "ara_backend/primary_ray_reference.rs"]
mod primary_ray_reference;

#[path = "ara_backend/cases/primary_trace.rs"]
mod primary_trace_cases;
#[path = "ara_backend/primary_trace_reference.rs"]
mod primary_trace_reference;

#[path = "ara_backend/cases/fullscreen_leaves.rs"]
mod fullscreen_leaf_cases;

#[path = "ara_backend/cases/transparency_glass.rs"]
mod transparency_glass_cases;
#[path = "ara_backend/transparency_glass_reference.rs"]
mod transparency_glass_reference;

#[path = "ara_backend/cases/receiver_raster.rs"]
mod receiver_raster_cases;
#[path = "ara_backend/receiver_raster_reference.rs"]
mod receiver_raster_reference;

#[path = "ara_backend/cases/fresh_visibility.rs"]
mod fresh_visibility_cases;
#[path = "ara_backend/fresh_visibility_reference.rs"]
mod fresh_visibility_reference;

#[path = "ara_backend/cases/shadow_raster.rs"]
mod shadow_raster_cases;

#[path = "ara_backend/cases/transmission_shadow.rs"]
mod transmission_shadow_cases;
#[path = "ara_backend/transmission_shadow_reference.rs"]
mod transmission_shadow_reference;

#[path = "ara_backend/cases/forward.rs"]
mod forward_cases;

#[path = "ara_backend/cases/gi_cache.rs"]
mod gi_cache_cases;
#[path = "ara_backend/gi_cache_reference.rs"]
mod gi_cache_reference;

#[path = "ara_backend/cases/temporal.rs"]
mod temporal_cases;
