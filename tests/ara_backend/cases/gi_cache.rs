//! The world radiance cache's sky-visibility kernels through the adapter.
use super::*;
use renderlab::backend_ara::{gi_cache::Entry, kernel::Kernel};

/// An entry, its workgroup size, and the inputs it adds to the three every stage takes.
type Stage = (Entry, [u32; 3], &'static [(u32, u32)]);

/// Every sky-visibility entry of the hardware-ray module reflects the interface its
/// stage needs, emits SPIR-V that validates, and becomes a native pipeline on a
/// ray-query device. None of them is replayed against a reference here.
#[test]
fn sky_visibility_kernels_reflect_validate_and_compile() -> Result<()> {
    let path = root("gi-cache-kernels");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    // Table, work words and frame record everywhere; then what each stage adds.
    let expected: [Stage; 9] = [
        (
            Entry::Probe,
            [8, 8, 1],
            &[(0, 7), (0, 21), (1, 0), (1, 1), (1, 2), (1, 3), (1, 5)],
        ),
        (Entry::Claim, [64, 1, 1], &[]),
        (Entry::Resolve, [64, 1, 1], &[(0, 33), (0, 37)]),
        (Entry::ResolveLast, [64, 1, 1], &[(0, 33), (0, 37)]),
        (Entry::Scan, [64, 1, 1], &[(3, 0), (3, 1)]),
        (
            Entry::Setup,
            [64, 1, 1],
            &[
                (0, 34),
                (0, 36),
                (0, 37),
                (1, 0),
                (1, 1),
                (1, 2),
                (1, 3),
                (1, 5),
            ],
        ),
        (Entry::Rays, [64, 1, 1], &[(0, 34)]),
        (
            Entry::Finish,
            [64, 1, 1],
            &[(0, 37), (3, 0), (3, 6), (3, 7), (3, 9)],
        ),
        (
            Entry::Integrate,
            [64, 1, 1],
            &[(0, 37), (3, 0), (3, 6), (3, 7), (3, 9)],
        ),
    ];
    ensure!(
        expected.map(|e| e.0) == Entry::ALL,
        "entry list is out of encode order"
    );
    for (entry, threads, extra) in expected {
        let program = entry.program()?;
        let mut keys = vec![(0, 30), (0, 31), (0, 32)];
        keys.extend_from_slice(extra);
        keys.sort_unstable();
        let reflected: Vec<_> = program
            .inputs()
            .iter()
            .map(|i| (i.key.group, i.key.binding))
            .collect();
        ensure!(
            reflected == keys && program.threads()? == threads,
            "{entry:?} interface changed: {:?} in groups of {:?}",
            program.inputs(),
            program.threads()
        );
        program.msl()?;
        forward_cases::validate_spirv(&program, &format!("gi-{entry:?}"))?;
        Kernel::from_program(&mut gpu, &cache, &program)?.retire(&mut gpu)?;
    }
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
