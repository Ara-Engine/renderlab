//! Preserve divergent resource identity through separate image/sampler combination.
use anyhow::{Result, ensure};
use std::collections::BTreeSet;
/// Naga marks divergent image/sampler loads but omits their combined resource.
/// Vulkan requires NonUniform on the operand of the final image-sampling command.
pub(super) fn repair(words: &mut Vec<u32>) -> Result<()> {
    ensure!(words.len() >= 5, "short SPIR-V header");
    let mut decorated = BTreeSet::new();
    let mut combined = Vec::new();
    let mut annotations = None;
    let mut first_type = None;
    let mut at = 5;
    while at < words.len() {
        let n = (words[at] >> 16) as usize;
        let op = words[at] & 0xffff;
        ensure!(n > 0 && at + n <= words.len(), "invalid SPIR-V instruction");
        match op {
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
            19..=39 => {
                first_type.get_or_insert(at);
            }
            _ => {}
        }
        at += n;
    }
    let additions: Vec<u32> = combined
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
    if !additions.is_empty() {
        let insert = annotations
            .or(first_type)
            .ok_or_else(|| anyhow::anyhow!("SPIR-V annotation section missing"))?;
        words.splice(insert..insert, additions);
    }
    Ok(())
}
