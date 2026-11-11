//! Lower scalar TLAS packet indices after naga's incomplete binding-array analysis.
use anyhow::{Context, Result, ensure};
use std::collections::BTreeMap;
/// Naga 30 panics on dynamic acceleration binding-array access. Emit a constant
/// marker, then load its actual index from the same immutable packet as Metal.
pub(super) fn repair(words: &mut Vec<u32>) -> Result<()> {
    ensure!(words.len() >= 5, "short SPIR-V header");
    let mut names = BTreeMap::new();
    let mut uint = None;
    let mut pointers = BTreeMap::new();
    let mut structs = BTreeMap::new();
    let mut runtime = BTreeMap::new();
    let mut variables = BTreeMap::new();
    let mut constants = BTreeMap::new();
    let mut at = 5;
    while at < words.len() {
        let n = (words[at] >> 16) as usize;
        ensure!(n > 0 && at + n <= words.len(), "invalid SPIR-V instruction");
        match words[at] & 0xffff {
            5 => {
                ensure!(n >= 3, "short OpName");
                let bytes: Vec<_> = words[at + 2..at + n]
                    .iter()
                    .flat_map(|w| w.to_le_bytes())
                    .take_while(|b| *b != 0)
                    .collect();
                names.insert(String::from_utf8(bytes)?, words[at + 1]);
            }
            21 if n == 4 && words[at + 2] == 32 && words[at + 3] == 0 => {
                uint = Some(words[at + 1]);
            }
            29 => {
                ensure!(n == 3, "short runtime array");
                runtime.insert(words[at + 1], words[at + 2]);
            }
            30 => {
                ensure!(n >= 2, "short struct");
                structs.insert(words[at + 1], words[at + 2..at + n].to_vec());
            }
            32 => {
                ensure!(n == 4, "short pointer");
                pointers.insert(words[at + 1], (words[at + 2], words[at + 3]));
            }
            43 if n == 4 => {
                constants.insert(words[at + 2], (words[at + 1], words[at + 3]));
            }
            59 => {
                ensure!(n >= 4, "short variable");
                variables.insert(words[at + 2], words[at + 1]);
            }
            _ => {}
        }
        at += n;
    }
    let arrays: BTreeMap<_, _> = names
        .iter()
        .filter_map(|(name, id)| {
            name.strip_prefix("ara_acceleration_")
                .and_then(|i| i.parse::<u32>().ok())
                .map(|i| (*id, i))
        })
        .collect();
    if arrays.is_empty() {
        return Ok(());
    }
    let uint = uint.context("packet u32 type missing")?;
    let root = *names.get("ara_root").context("root variable missing")?;
    let packet = *names.get("ara_packet").context("packet variable missing")?;
    let (space, outer) = pointers
        .get(variables.get(&root).context("root pointer type missing")?)
        .context("root pointer missing")?;
    let outer = structs.get(outer).context("root wrapper missing")?;
    ensure!(
        *space == 9
            && outer.len() == 1
            && structs.get(&outer[0]).is_some_and(|s| s == &vec![uint; 8]),
        "unexpected root wrapper shape"
    );
    let (space, array) = pointers
        .get(
            variables
                .get(&packet)
                .context("packet pointer type missing")?,
        )
        .context("packet pointer missing")?;
    let record = runtime
        .get(array)
        .and_then(|ty| structs.get(ty))
        .context("packet descriptor array shape missing")?;
    ensure!(
        *space == 12 && record.len() == 1 && runtime.get(&record[0]) == Some(&uint),
        "unexpected packet record shape"
    );
    let mut bound = words[3];
    let mut fresh = || -> Result<u32> {
        let id = bound;
        bound = bound.checked_add(1).context("SPIR-V ID overflow")?;
        Ok(id)
    };
    let mut declarations = Vec::new();
    let mut scalar_pointer = |space| -> Result<u32> {
        if let Some((id, _)) = pointers.iter().find(|(_, pair)| **pair == (space, uint)) {
            return Ok(*id);
        }
        let id = fresh()?;
        declarations.extend([(4 << 16) | 32, id, space, uint]);
        Ok(id)
    };
    let push_pointer = scalar_pointer(9)?;
    let storage_pointer = scalar_pointer(12)?;
    let mut constant = |value| -> Result<u32> {
        let id = fresh()?;
        declarations.extend([(4 << 16) | 43, uint, id, value]);
        Ok(id)
    };
    let zero = constant(0)?;
    let one = constant(1)?;
    let two = constant(2)?;
    let mut ordinals = BTreeMap::new();
    for (array, ordinal) in arrays {
        ordinals.insert(array, constant(ordinal)?);
    }
    let mut out = words[..5].to_vec();
    let mut declared = false;
    let mut patched = 0;
    at = 5;
    while at < words.len() {
        let n = (words[at] >> 16) as usize;
        let op = words[at] & 0xffff;
        if op == 54 && !declared {
            out.extend(&declarations);
            declared = true;
        }
        if matches!(op, 65 | 66)
            && n >= 4
            && let Some(ordinal) = ordinals.get(&words[at + 3])
        {
            ensure!(
                n == 5 && constants.get(&words[at + 4]) == Some(&(uint, 0)),
                "unexpected TLAS descriptor marker shape"
            );
            let root0 = fresh()?;
            let slot = fresh()?;
            let root1 = fresh()?;
            let offset = fresh()?;
            let shifted = fresh()?;
            let index = fresh()?;
            let address = fresh()?;
            let value = fresh()?;
            out.extend([
                (6 << 16) | 65,
                push_pointer,
                root0,
                root,
                zero,
                zero,
                (4 << 16) | 61,
                uint,
                slot,
                root0,
                (6 << 16) | 65,
                push_pointer,
                root1,
                root,
                zero,
                one,
                (4 << 16) | 61,
                uint,
                offset,
                root1,
                (5 << 16) | 194,
                uint,
                shifted,
                offset,
                two,
                (5 << 16) | 128,
                uint,
                index,
                shifted,
                *ordinal,
                (7 << 16) | 65,
                storage_pointer,
                address,
                packet,
                slot,
                zero,
                index,
                (4 << 16) | 61,
                uint,
                value,
                address,
            ]);
            out.extend_from_slice(&words[at..at + n]);
            let end = out.len();
            out[end - 1] = value;
            patched += 1;
        } else {
            out.extend_from_slice(&words[at..at + n]);
        }
        at += n;
    }
    ensure!(declared && patched > 0, "TLAS descriptor marker missing");
    out[3] = bound;
    *words = out;
    Ok(())
}
