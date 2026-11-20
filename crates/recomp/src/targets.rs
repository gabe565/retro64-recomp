//! Target-specific configuration for Bugatron Gold v1.20a (`Bug3D.exe`) and
//! Z-Ball Gold (`Z-Ball.exe`).
//!
//! Everything here is backed by evidence recorded in `docs/`.

use std::collections::BTreeMap;

/// Instructions that are only reachable while the MSVC C++ exception
/// dispatcher is running (after `RaiseException`). The runtime does not
/// dispatch exceptions, so these become explicit diagnostic traps instead of
/// guessed indirect jumps. See docs/recompilation-feasibility.md (D8).
pub fn gold_120a_exception_traps() -> BTreeMap<u32, String> {
    let mut m = BTreeMap::new();
    for (addr, what) in [
        (0x42a76b, "C++ EH _JumpToContinuation (0x42a740): resumes after a catch block"),
        (0x42a779, "C++ EH _CallMemberFunction1 (0x42a774): copy-constructs a catch object"),
        (0x42a780, "C++ EH _CallMemberFunction2 (0x42a77b): copy-constructs a catch object"),
        (0x42a9a0, "C++ EH frame handler (0x42a936): calls an unwind funclet"),
    ] {
        m.insert(addr, format!("exception dispatch path reached: {what}"));
    }
    m
}

/// Z-Ball Gold's copies of the same exception-dispatch sites: the same MSVC runtime
/// code, 0x18c lower (matched byte for byte, apart from one address operand in the
/// frame handler).
pub fn zball_gold_exception_traps() -> BTreeMap<u32, String> {
    gold_120a_exception_traps().into_iter().map(|(a, what)| (a - 0x18c, what.replace("(0x42a740)", "(0x42a5b4)").replace("(0x42a774)", "(0x42a5e8)").replace("(0x42a77b)", "(0x42a5ef)").replace("(0x42a936)", "(0x42a7aa)"))).collect()
}
