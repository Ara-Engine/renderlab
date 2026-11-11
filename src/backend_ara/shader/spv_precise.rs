//! Float arithmetic that the driver may not fuse, as on Metal.
//!
//! The Metal path compiles every pass with `-fno-fast-math -ffp-contract=off`, so a
//! multiply and an add stay two rounded operations unless the source calls `fma`.
//! SPIR-V leaves that choice to the driver unless a result is decorated
//! `NoContraction`. Without it the two backends can land on different sides of an
//! exact comparison, such as an edge function at a triangle edge.
use anyhow::{Result, ensure};

/// `OpDecorate`'s `NoContraction`.
const NO_CONTRACTION: u32 = 42;

/// The float arithmetic instructions the decoration applies to: negate, add, subtract,
/// multiply, divide, remainder, modulo, the vector and matrix products, and dot.
fn arithmetic(opcode: u32) -> bool {
    matches!(opcode, 127 | 129 | 131 | 133 | 136 | 140 | 141 | 142..=148)
}

/// Decorate every float arithmetic result `NoContraction`. Explicit `fma` calls are
/// extended instructions and stay fused, as the source asked.
pub(super) fn decorate(words: &mut Vec<u32>) -> Result<()> {
    ensure!(words.len() >= 5, "short SPIR-V header");
    let mut results = Vec::new();
    // Annotations end where types, constants, globals or functions begin.
    let mut declarations = None;
    let mut at = 5;
    while at < words.len() {
        let count = (words[at] >> 16) as usize;
        let opcode = words[at] & 0xffff;
        ensure!(
            count > 0 && at + count <= words.len(),
            "invalid SPIR-V instruction"
        );
        if arithmetic(opcode) {
            ensure!(count >= 4, "short arithmetic instruction");
            results.push(words[at + 2]);
        } else if matches!(opcode, 19..=39 | 41..=52 | 54 | 59) {
            declarations.get_or_insert(at);
        }
        at += count;
    }
    let Some(insert) = declarations else {
        ensure!(
            results.is_empty(),
            "arithmetic without a declaration section"
        );
        return Ok(());
    };
    words.splice(
        insert..insert,
        results
            .into_iter()
            .flat_map(|id| [(3 << 16) | 71, id, NO_CONTRACTION]),
    );
    Ok(())
}
