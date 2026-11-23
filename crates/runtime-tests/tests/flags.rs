//! The flag readers' fast paths in recomp.h agree with the full flag computation
//! (c/flags_check.c), over millions of states from the real instruction helpers.

// Links the library crate, whose archive holds the C check.
use bugatron_runtime_tests as _;

extern "C" {
    fn flags_selfcheck(seed: u64, n: u32) -> u32;
}

#[test]
fn fast_flag_readers_match_flags_get() {
    let n = if cfg!(debug_assertions) { 1_000_000 } else { 10_000_000 };
    // SAFETY: a pure C function over its own locals.
    let bad = unsafe { flags_selfcheck(0x9e37_79b9_7f4a_7c15, n) };
    assert_eq!(bad, 0, "{bad} mismatched conditions in {n} states");
}
