//! The material pass on the port (RENDERLAB_MATERIAL_PASS): cold compile times of the standard
//! program's single shading kernel against the material pass's record writer and the
//! record-reading shading, tiles and list. Measurement, ignored by default.
use super::*;
use renderlab::backend_ara::primary_shade;

#[test]
#[ignore]
fn material_pass_compile_seconds() -> Result<()> {
    let path = root("material-pass");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let standard = renderlab::STANDARD_PROGRAM;
    for list in [false, true] {
        let full = if list {
            primary_shade::hardware_soft_list_program(standard)?
        } else {
            primary_shade::hardware_soft_program(standard)?
        };
        let writer = primary_shade::hardware_soft_material_writer(standard, list)?
            .context("the standard program declares the record mapping")?;
        let reader = primary_shade::hardware_soft_material_reader(standard, list)?
            .context("the standard program declares the record mapping")?;
        for (name, program) in [("single kernel", full), ("material writer", writer), ("record reader", reader)] {
            let started = std::time::Instant::now();
            renderlab::backend_ara::kernel::Kernel::from_program(&mut gpu, &cache, &program)?;
            println!(
                "material pass compile: list={list} {name}: {:.2} s",
                started.elapsed().as_secs_f64()
            );
        }
    }
    Ok(())
}
