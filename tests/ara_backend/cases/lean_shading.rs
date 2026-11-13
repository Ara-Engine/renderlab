//! Lean shading on the port (`program::lean_variant`): the composed text sizes and cold compile
//! times of the full and lean standard traced shading modules. Measurement, ignored by default.
use super::*;
use renderlab::backend_ara::primary_shade;

#[test]
#[ignore]
fn lean_module_size_and_compile_time() -> Result<()> {
    let path = root("lean-shading");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    for lean in [false, true] {
        let started = std::time::Instant::now();
        let (program, len) = primary_shade::lean_probe(renderlab::STANDARD_PROGRAM, lean)?;
        let composed = started.elapsed();
        let started = std::time::Instant::now();
        renderlab::backend_ara::kernel::Kernel::from_program(&mut gpu, &cache, &program)?;
        println!(
            "lean shading: lean={lean} text={len} bytes, compose {:.1} ms, compile {:.0} ms",
            composed.as_secs_f64() * 1e3,
            started.elapsed().as_secs_f64() * 1e3
        );
    }
    Ok(())
}
