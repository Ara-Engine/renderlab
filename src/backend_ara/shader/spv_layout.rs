//! Keep physical buffer layouts separate from logical function/workgroup values.
use anyhow::{Context, Result, ensure};
use std::collections::{BTreeMap, BTreeSet};

fn walk(roots: Vec<u32>, types: &BTreeMap<u32, Vec<u32>>) -> BTreeSet<u32> {
    let mut pending = roots;
    let mut seen = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if seen.insert(id)
            && let Some(children) = types.get(&id)
        {
            pending.extend(children);
        }
    }
    seen
}
fn layout(instruction: &[u32]) -> bool {
    match instruction[0] & 0xffff {
        71 => instruction.len() >= 3 && matches!(instruction[2], 2 | 3 | 6),
        72 => instruction.len() >= 4 && matches!(instruction[3], 4 | 5 | 7 | 35),
        _ => false,
    }
}
/// Naga30 reuses decorated structs for mutable source values. Vulkan forbids
/// explicit offsets/strides on Function and Workgroup memory. Clone shared
/// physical types and bridge whole loads/stores using SPIR-V1.4 OpCopyLogical;
/// their member types/counts stay identical, including arrays and nested structs.
pub(super) fn repair(words: &mut Vec<u32>) -> Result<()> {
    let mut instructions = Vec::<Vec<u32>>::new();
    let mut at = 5;
    while at < words.len() {
        let n = (words[at] >> 16) as usize;
        ensure!(n > 0 && at + n <= words.len(), "invalid SPIR-V instruction");
        instructions.push(words[at..at + n].to_vec());
        at += n;
    }
    let mut types = BTreeMap::<u32, Vec<u32>>::new();
    let mut pointers = BTreeMap::<u32, (u32, u32)>::new();
    let mut result_types = BTreeMap::new();
    let mut physical_roots = Vec::new();
    let mut internal_roots = Vec::new();
    let physical = |class| matches!(class, 2 | 9 | 12 | 5349);
    for i in &instructions {
        match i[0] & 0xffff {
            28 | 29 => {
                types.insert(i[1], vec![i[2]]);
            }
            30 => {
                types.insert(i[1], i[2..].to_vec());
            }
            32 => {
                pointers.insert(i[1], (i[2], i[3]));
                if physical(i[2]) {
                    physical_roots.push(i[3]);
                } else {
                    internal_roots.push(i[3]);
                }
            }
            // ResultType/ResultId operations that can produce a pointer or a
            // composite copied to a physical buffer. Unknown origins fail below.
            1 | 41 | 42 | 43 | 44 | 45 | 46 | 48 | 49 | 50 | 51 | 52 | 55 | 57 | 59 | 61 | 65
            | 66 | 80 | 81 | 82 | 83 | 124 | 169 | 245 => {
                result_types.insert(i[2], i[1]);
            }
            _ => {}
        }
    }
    let physical_types = walk(physical_roots, &types);
    let internal_types = walk(internal_roots, &types);
    let mut split = BTreeSet::new();
    for i in &instructions {
        if layout(i) && internal_types.contains(&i[1]) && physical_types.contains(&i[1]) {
            split.insert(i[1]);
        }
    }
    // Every physical ancestor must point to the physical version of its child.
    loop {
        let before = split.len();
        for (id, children) in &types {
            if physical_types.contains(id) && children.iter().any(|c| split.contains(c)) {
                split.insert(*id);
            }
        }
        if split.len() == before {
            break;
        }
    }
    let mut bound = words[3];
    let mut next = || -> Result<u32> {
        let id = bound;
        bound = bound.checked_add(1).context("SPIR-V id bound overflow")?;
        Ok(id)
    };
    let mut clones = BTreeMap::new();
    for id in split {
        clones.insert(id, next()?);
    }
    let mut out = words[..5].to_vec();
    for mut i in instructions {
        let op = i[0] & 0xffff;
        if matches!(op, 71 | 72) {
            if let Some(clone) = clones.get(&i[1]) {
                let mut copied = i.clone();
                copied[1] = *clone;
                out.extend(copied);
                // Non-layout decorations still apply to logical values as well.
                if !layout(&i) || !internal_types.contains(&i[1]) {
                    out.extend(i);
                }
                continue;
            }
            if layout(&i) && internal_types.contains(&i[1]) {
                continue;
            }
        }
        if let Some(&clone) = clones.get(&i.get(1).copied().unwrap_or(0))
            && matches!(op, 28..=30)
        {
            let mut copied = i.clone();
            copied[1] = clone;
            let end = if op == 30 { copied.len() } else { 3 };
            for child in &mut copied[2..end] {
                if let Some(c) = clones.get(child) {
                    *child = *c;
                }
            }
            out.extend(i);
            out.extend(copied);
            continue;
        }
        if op == 32
            && physical(i[2])
            && let Some(clone) = clones.get(&i[3])
        {
            i[3] = *clone;
        } else if op == 61 && clones.contains_key(&i[1]) {
            let ptr_type = result_types
                .get(&i[3])
                .context("composite load pointer origin unsupported")?;
            let &(class, pointee) = pointers
                .get(ptr_type)
                .context("composite load pointer type missing")?;
            if physical(class) {
                ensure!(pointee == i[1], "composite load pointer mismatch");
                let original_type = i[1];
                let original_result = i[2];
                let loaded = next()?;
                i[1] = clones[&original_type];
                i[2] = loaded;
                out.extend(i);
                out.extend([(4 << 16) | 400, original_type, original_result, loaded]);
                continue;
            }
        } else if op == 62 {
            let pointer = result_types.get(&i[1]).and_then(|t| pointers.get(t));
            if let Some(&(class, pointee)) = pointer
                && physical(class)
                && let Some(&clone) = clones.get(&pointee)
            {
                ensure!(
                    result_types.get(&i[2]) == Some(&pointee),
                    "composite store value type unsupported"
                );
                let stored = next()?;
                out.extend([(4 << 16) | 400, clone, stored, i[2]]);
                i[2] = stored;
            }
        }
        out.extend(i);
    }
    out[3] = bound;
    *words = out;
    Ok(())
}
