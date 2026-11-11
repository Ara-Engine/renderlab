//! Temporal reconstruction through the adapter.
use super::*;
use renderlab::backend_ara::{kernel::Kernel, temporal::resolve_program};

/// `tr1_resolve` reflects the interface `reconstruction_layout` declares, emits SPIR-V
/// that validates and becomes a native pipeline. Nothing is replayed here.
#[test]
fn temporal_resolve_reflects_validates_and_compiles() -> Result<()> {
    let path = root("temporal-kernels");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let program = resolve_program()?;
    let reflected: Vec<_> = program
        .inputs()
        .iter()
        .map(|i| (i.key.group, i.key.binding))
        .collect();
    // The hit records, the primary frame, the tiles, the TR1 frame record, the frame's
    // samples, history in and out (colour, anchor, layer), the output; then the
    // geometry arrays with previous models and previous skinned vertices.
    let expected = [
        (0, 7),
        (0, 10),
        (0, 21),
        (0, 40),
        (0, 41),
        (0, 42),
        (0, 43),
        (0, 44),
        (0, 45),
        (0, 46),
        (0, 47),
        (0, 48),
        (1, 0),
        (1, 2),
        (1, 3),
        (1, 5),
        (1, 6),
    ];
    ensure!(
        reflected == expected && program.threads()? == [8, 8, 1],
        "temporal resolve interface changed: {:?}",
        program.inputs()
    );
    program.msl()?;
    forward_cases::validate_spirv(&program, "tr1-resolve")?;
    Kernel::from_program(&mut gpu, &cache, &program)?.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
