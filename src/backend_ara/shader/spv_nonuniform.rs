//! Preserve divergent resource identity where Naga's SPIR-V drops it.
use anyhow::{Result, ensure};
use std::collections::BTreeSet;
/// Decorates `NonUniform` what Naga leaves undecorated:
///
/// - the combined resource of a divergent image and sampler. Naga marks the two
///   loads; Vulkan requires the decoration on the operand of the final sampling
///   command.
/// - the element pointer `OpArrayLength` reads through. Naga decorates the loads and
///   stores through `heap[i]` and not `arrayLength(&heap[i].data)`, so a driver may
///   take the index as the same for every invocation of a workgroup. RADV does: each
///   invocation then gets one element's length, and the renderer's
///   `primary_edge_neighbour`, which tells an index buffer with adjacency from one
///   without by its length, judged every mesh by another's.
pub(super) fn repair(words: &mut Vec<u32>) -> Result<()> {
    ensure!(words.len() >= 5, "short SPIR-V header");
    let mut decorated = BTreeSet::new();
    let mut combined = Vec::new();
    // Pointers made by an indexed access chain, and the pointers `OpArrayLength` reads.
    let (mut elements, mut lengths) = (BTreeSet::new(), Vec::new());
    let mut capable = false;
    let mut annotations = None;
    let mut first_type = None;
    let mut at = 5;
    while at < words.len() {
        let n = (words[at] >> 16) as usize;
        let op = words[at] & 0xffff;
        ensure!(n > 0 && at + n <= words.len(), "invalid SPIR-V instruction");
        match op {
            // OpCapability ShaderNonUniform.
            17 => capable |= n == 2 && words[at + 1] == 5301,
            71 => {
                ensure!(n >= 3, "short OpDecorate");
                annotations.get_or_insert(at);
                if words[at + 2] == 5300 {
                    decorated.insert(words[at + 1]);
                }
            }
            86 => {
                ensure!(n == 5, "invalid OpSampledImage");
                combined.push((words[at + 2], words[at + 3], words[at + 4]));
            }
            // OpAccessChain and OpInBoundsAccessChain with at least one index.
            65 | 66 if n >= 5 => {
                elements.insert(words[at + 2]);
            }
            68 => {
                ensure!(n == 5, "invalid OpArrayLength");
                lengths.push(words[at + 3]);
            }
            19..=39 => {
                first_type.get_or_insert(at);
            }
            _ => {}
        }
        at += n;
    }
    let mut additions: Vec<u32> = combined
        .into_iter()
        .filter_map(|(result, image, sampler)| {
            if (decorated.contains(&image) || decorated.contains(&sampler))
                && decorated.insert(result)
            {
                Some([(3 << 16) | 71, result, 5300])
            } else {
                None
            }
        })
        .flatten()
        .collect();
    for pointer in lengths {
        if elements.contains(&pointer) && decorated.insert(pointer) {
            // Every module that indexes the heap already declares the capability
            // through Naga's own decorations; one that only asks a length would not.
            ensure!(
                capable,
                "array length through a heap element in a module without ShaderNonUniform"
            );
            additions.extend([(3 << 16) | 71, pointer, 5300]);
        }
    }
    if !additions.is_empty() {
        let insert = annotations
            .or(first_type)
            .ok_or_else(|| anyhow::anyhow!("SPIR-V annotation section missing"))?;
        words.splice(insert..insert, additions);
    }
    Ok(())
}
