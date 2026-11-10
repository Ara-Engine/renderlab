//! Native Metal/Vulkan replays. All generated files stay in ignored target directories.
#![cfg(feature = "backend-ara")]
#[path = "ara_backend/reference.rs"]
mod reference;
use anyhow::{Result, ensure};
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
    for pass in [
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
        emit_spirv(pass, &path)?;
    }
    println!("SPIR-V output: {}", path.display());
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

#[path = "ara_backend/cases/sky.rs"]
mod sky_cases;
#[path = "ara_backend/sky.rs"]
mod sky_reference;
#[path = "ara_backend/cases/volumes.rs"]
mod volume_cases;
#[path = "ara_backend/volumes.rs"]
mod volume_reference;
