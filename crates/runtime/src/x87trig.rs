//! x87-compatible FSIN / FCOS / FPTAN.
//!
//! The x87 reduces the argument modulo π/2 using an internal **66-bit
//! approximation of π** (Intel SDM: "uses a 66-bit approximation of π"), not
//! the true π. Results near zeros of the function therefore carry an absolute
//! error of about 4e-21 compared with a mathematically correct `sin`/`cos`.
//! This module reproduces that behaviour:
//!
//! 1. `x` is converted exactly to fixed point (192 fractional bits; 1200 for
//!    |x| < 2⁻⁸⁰, e.g. the denormal float angles the game produces);
//! 2. `k = round(x / (π₆₆/2))`, `r = x − k·π₆₆/2`, computed exactly;
//! 3. the needed series (`sin r` or `cos r`, both for `tan`) is evaluated to
//!    about FRAC−8 bits — at least 2⁻¹²⁰ relative to any result, far beyond the
//!    2⁻⁶⁴ the x87 delivers;
//! 4. the result is rounded to the x87's 64-bit significand (round to nearest
//!    even), which is what lands in ST(0);
//! 5. the machine model keeps ST registers as binary64, so the value is then
//!    rounded again to 53 bits — exactly what a real `FSTP m64` does. If that
//!    second rounding was inexact, `X87_NOTE_EXT64` is raised, because later
//!    *arithmetic* on the register would have seen the 64-bit value.
//!
//! Status: C2 = 1 and ST(0) unchanged for |x| ≥ 2⁶³ (out of range); C1 = 1 if
//! the 64-bit rounding increased the magnitude; ±∞ is an invalid operation
//! (IE, result = the default QNaN "indefinite"); NaN propagates (quieted).
//!
//! Validated against an AMD Ryzen 7 9800X3D (tests/fixtures/x86-oracle-*.json).
//! The CPU's own polynomial can still differ by one ULP of the 64-bit result in
//! rare cases; none has been observed so far (docs/recompilation-feasibility.md).

use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::{One, Signed, ToPrimitive, Zero};

/// π₆₆ · 2⁶⁴: the x87's 66-bit approximation of π (2 integer + 64 fraction bits).
pub const PI66_SCALED: u128 = 0x3_243F_6A88_85A3_08D3;
/// Fixed-point fraction bits for ordinary inputs, and for |x| < 2^-80.
const FRAC_NORMAL: u32 = 192;
const FRAC_TINY: u32 = 1200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrigOp {
    Sin,
    Cos,
    Tan,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrigOutcome {
    /// In range. `value` is the binary64 the model stores in ST(0) (for Tan: the
    /// tangent; 1.0 is pushed separately). `c1`: magnitude rounded up when
    /// rounding to the 64-bit significand. `ext_inexact`: the 64-bit result was
    /// not representable in binary64.
    Value { value: f64, c1: bool, ext_inexact: bool },
    /// |x| ≥ 2⁶³: C2 = 1, ST(0) unchanged, nothing pushed.
    OutOfRange,
    /// ±∞: invalid operation; ST(0) = default QNaN (indefinite).
    Invalid,
    /// NaN operand: propagated (quiet).
    Nan(f64),
}

/// The x87 "real indefinite" QNaN, as binary64.
pub const INDEFINITE: u64 = 0xFFF8_0000_0000_0000;

pub fn x87_trig(op: TrigOp, x: f64) -> TrigOutcome {
    if x.is_nan() {
        return TrigOutcome::Nan(f64::from_bits(x.to_bits() | 0x0008_0000_0000_0000));
    }
    if x.is_infinite() {
        return TrigOutcome::Invalid;
    }
    if x.abs() >= 9_223_372_036_854_775_808.0 {
        return TrigOutcome::OutOfRange;
    }
    if x == 0.0 {
        let value = if op == TrigOp::Cos { 1.0 } else { x };
        return TrigOutcome::Value { value, c1: false, ext_inexact: false };
    }
    let frac = if x.abs() < 8.271806125530277e-25 { FRAC_TINY } else { FRAC_NORMAL }; // 2^-80
    let xf = to_fixed(x, frac);
    let one = BigInt::one() << frac;
    let half_pi = BigInt::from(PI66_SCALED) << (frac - 65);
    let k = round_div(&xf, &half_pi);
    let r = &xf - &k * &half_pi;
    let q = k.mod_floor(&BigInt::from(4)).to_u32().unwrap();
    let r2 = mul_fixed(&r, &r, frac);
    let v = match (op, q) {
        (TrigOp::Sin, 0) => sin_fixed(&r, &r2, frac),
        (TrigOp::Sin, 1) => cos_fixed(&one, &r2, frac),
        (TrigOp::Sin, 2) => -sin_fixed(&r, &r2, frac),
        (TrigOp::Sin, _) => -cos_fixed(&one, &r2, frac),
        (TrigOp::Cos, 0) => cos_fixed(&one, &r2, frac),
        (TrigOp::Cos, 1) => -sin_fixed(&r, &r2, frac),
        (TrigOp::Cos, 2) => -cos_fixed(&one, &r2, frac),
        (TrigOp::Cos, _) => sin_fixed(&r, &r2, frac),
        (TrigOp::Tan, q) => {
            let (s, c) = (sin_fixed(&r, &r2, frac), cos_fixed(&one, &r2, frac));
            if q % 2 == 0 { div_fixed(&s, &c, frac) } else { -div_fixed(&c, &s, frac) }
        }
    };
    let (value, c1, ext_inexact) = round_fixed_to_x87(&v, frac);
    TrigOutcome::Value { value, c1, ext_inexact }
}

/// Exact conversion of a finite binary64 to fixed point with `frac` fraction
/// bits. Exact for every |x| ≥ 2^-80 with frac = 192, and for every binary64
/// (including subnormals) with frac = 1200.
fn to_fixed(x: f64, frac: u32) -> BigInt {
    let bits = x.to_bits();
    let neg = bits >> 63 != 0;
    let exp = ((bits >> 52) & 0x7ff) as i32;
    let fbits = bits & ((1u64 << 52) - 1);
    let (mant, e) = if exp == 0 { (fbits, -1074) } else { (fbits | (1u64 << 52), exp - 1075) };
    let mut v = BigInt::from(mant);
    let shift = e + frac as i32;
    v = if shift >= 0 { v << shift as u32 } else { v >> (-shift) as u32 };
    if neg {
        -v
    } else {
        v
    }
}

/// Round-to-nearest (ties to even) integer quotient.
fn round_div(a: &BigInt, b: &BigInt) -> BigInt {
    let (q, r) = a.div_mod_floor(b);
    let twice: BigInt = &r * 2u32;
    match twice.cmp(b) {
        std::cmp::Ordering::Greater => q + 1,
        std::cmp::Ordering::Less => q,
        std::cmp::Ordering::Equal => {
            if q.is_even() {
                q
            } else {
                q + 1
            }
        }
    }
}

fn mul_fixed(a: &BigInt, b: &BigInt, frac: u32) -> BigInt {
    (a * b) >> frac
}

fn div_fixed(a: &BigInt, b: &BigInt, frac: u32) -> BigInt {
    (a << frac) / b
}

/// Taylor series for |r| ≤ π/4 (+ tiny slack); absolute error about 2^(8-frac).
fn sin_fixed(r: &BigInt, r2: &BigInt, frac: u32) -> BigInt {
    let eps = BigInt::one() << 4;
    let mut s = r.clone();
    let mut term = r.clone();
    let mut n: u64 = 1;
    while term.abs() > eps {
        term = -mul_fixed(&term, r2, frac) / BigInt::from((2 * n) * (2 * n + 1));
        s += &term;
        n += 1;
    }
    s
}

fn cos_fixed(one: &BigInt, r2: &BigInt, frac: u32) -> BigInt {
    let eps = BigInt::one() << 4;
    let mut c = one.clone();
    let mut term = one.clone();
    let mut n: u64 = 1;
    while term.abs() > eps {
        term = -mul_fixed(&term, r2, frac) / BigInt::from((2 * n - 1) * (2 * n));
        c += &term;
        n += 1;
    }
    c
}

/// Round a fixed-point value to a 64-bit significand (RNE), then to binary64
/// (RNE). Returns (binary64, c1 = magnitude increased in the 64-bit rounding,
/// second rounding inexact).
fn round_fixed_to_x87(v: &BigInt, frac: u32) -> (f64, bool, bool) {
    let neg = v.is_negative();
    let a = v.abs();
    if a.is_zero() {
        return (if neg { -0.0 } else { 0.0 }, false, false);
    }
    let len = a.bits() as i64;
    // 64-bit significand.
    let (mut m64, mut e64, c1) = if len > 64 {
        let sh = (len - 64) as u32;
        let q: BigInt = &a >> sh;
        let rem: BigInt = &a - (&q << sh);
        let half = BigInt::one() << (sh - 1);
        let up = rem > half || (rem == half && q.is_odd());
        let q = if up { q + 1 } else { q };
        (q.to_u128().unwrap(), sh as i64 - frac as i64, up)
    } else {
        let sh = (64 - len) as u32;
        ((&a << sh).to_u128().unwrap(), -(sh as i64) - frac as i64, false)
    };
    if m64 >> 64 != 0 {
        m64 >>= 1;
        e64 += 1;
    }
    // value = m64 · 2^e64 with 2^63 ≤ m64 < 2^64. Round to binary64 (RNE),
    // including the subnormal range: keep 53 bits, or fewer so that the result's
    // exponent is not below 2^-1074.
    let shift = 11i64.max(-1074 - e64);
    let (mut m, ext_inexact) = if shift >= 66 {
        (0u64, true)
    } else {
        let sh = shift as u32;
        let q = (m64 >> sh) as u64;
        let rem = m64 & ((1u128 << sh) - 1);
        let half = 1u128 << (sh - 1);
        let up = rem > half || (rem == half && q & 1 == 1);
        (q + up as u64, rem != 0)
    };
    let mut e = e64 + shift;
    if m >> 53 != 0 {
        m >>= 1;
        e += 1;
    }
    let f = if e == -1074 {
        // m · 2^-1074 with m < 2^53 has exactly the bit pattern m (subnormal or the
        // smallest normal binade).
        f64::from_bits(m)
    } else {
        let biased = e + 52 + 1023;
        assert!((1..2047).contains(&biased), "x87 trig result outside the binary64 range");
        f64::from_bits(((biased as u64) << 52) | (m & ((1u64 << 52) - 1)))
    };
    (if neg { -f } else { f }, c1, ext_inexact)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn val(op: TrigOp, x: f64) -> f64 {
        match x87_trig(op, x) {
            TrigOutcome::Value { value, .. } => value,
            o => panic!("{o:?}"),
        }
    }

    /// Hardware values from an AMD Ryzen 7 9800X3D (FSTP m64, CW 0x027F).
    #[test]
    fn matches_hardware_reference() {
        use std::f64::consts::*;
        let cases: &[(TrigOp, f64, f64)] = &[
            (TrigOp::Sin, 0.1, 0.09983341664682815),
            (TrigOp::Cos, 0.1, 0.9950041652780258),
            (TrigOp::Sin, FRAC_PI_4, 0.7071067811865475),
            (TrigOp::Cos, FRAC_PI_4, 0.7071067811865476),
            (TrigOp::Sin, FRAC_PI_2, 1.0),
            (TrigOp::Cos, FRAC_PI_2, 6.123031769111886e-17),
            (TrigOp::Sin, 2.0, 0.9092974268256817),
            (TrigOp::Cos, 2.0, -0.4161468365471424),
            (TrigOp::Sin, PI, 1.2246063538223773e-16),
            (TrigOp::Cos, PI, -1.0),
            (TrigOp::Sin, 4.0, -0.7568024953079282),
            (TrigOp::Cos, 4.0, -0.6536436208636119),
            (TrigOp::Sin, 5.5, -0.7055403255703919),
            (TrigOp::Cos, 5.5, 0.70866977429126),
            (TrigOp::Sin, TAU, -2.4492127076447545e-16),
            (TrigOp::Cos, TAU, 1.0),
            (TrigOp::Sin, 1.0, 0.8414709848078965),
            (TrigOp::Cos, 1.0, 0.5403023058681398),
            (TrigOp::Tan, 0.5, 0.5463024898437905),
        ];
        for &(op, x, want) in cases {
            assert_eq!(val(op, x).to_bits(), want.to_bits(), "{op:?}({x})");
        }
    }

    #[test]
    fn odd_even_symmetry_and_special_values() {
        for &x in &[0.1, 0.5, 1.0, std::f64::consts::PI, 4.0, 100.0, 12345.678] {
            assert_eq!(val(TrigOp::Sin, -x), -val(TrigOp::Sin, x), "sin odd at {x}");
            assert_eq!(val(TrigOp::Cos, -x), val(TrigOp::Cos, x), "cos even at {x}");
            assert_eq!(val(TrigOp::Tan, -x), -val(TrigOp::Tan, x), "tan odd at {x}");
        }
        assert_eq!(val(TrigOp::Sin, -0.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(val(TrigOp::Cos, 0.0), 1.0);
        assert_eq!(x87_trig(TrigOp::Sin, 9.223372036854775808e18), TrigOutcome::OutOfRange);
        assert_eq!(x87_trig(TrigOp::Cos, -9.223372036854775808e18), TrigOutcome::OutOfRange);
        assert!(matches!(x87_trig(TrigOp::Sin, 9.2233720368547748e18), TrigOutcome::Value { .. }));
        assert_eq!(x87_trig(TrigOp::Sin, f64::INFINITY), TrigOutcome::Invalid);
        assert!(matches!(x87_trig(TrigOp::Cos, f64::NAN), TrigOutcome::Nan(_)));
        // Tiny and subnormal arguments (the game passes denormal float angles).
        for &x in &[1.0e-30, 1.0e-300, 5.0e-324, 1.4e-45 * 0.5] {
            assert_eq!(val(TrigOp::Sin, x), x, "sin({x})");
            assert_eq!(val(TrigOp::Tan, x), x, "tan({x})");
            assert_eq!(val(TrigOp::Cos, x), 1.0, "cos({x})");
        }
    }

    /// Cost check: `cargo test --release -p bugatron-runtime -- --ignored --nocapture`.
    /// The game calls FSIN+FCOS about 127 times per frame (r3d 0x421680).
    #[test]
    #[ignore]
    fn bench_per_call() {
        let n = 20_000;
        let t = std::time::Instant::now();
        let mut acc = 0.0;
        for i in 0..n {
            let x = (i as f64) * 0.000314 - 3.0;
            acc += val(TrigOp::Sin, x) + val(TrigOp::Cos, x);
        }
        let per = t.elapsed().as_secs_f64() / (2.0 * n as f64);
        println!("x87 trig: {:.2} us per call; 254 calls/frame = {:.3} ms (acc {acc})", per * 1e6, per * 254.0 * 1e3);
    }

    #[test]
    fn reduction_uses_pi66_not_true_pi() {
        // sin(x) for x = nearest double to π equals π₆₆ − x (to first order),
        // not π − x; the difference is ~4.0e-21.
        let s = val(TrigOp::Sin, std::f64::consts::PI);
        assert!((s - 1.2246467991473532e-16).abs() > 4.0e-21 && (s - 1.2246467991473532e-16).abs() < 4.1e-21);
    }
}
