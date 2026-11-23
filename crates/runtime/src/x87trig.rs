//! x87-compatible FSIN / FCOS / FPTAN, and FPATAN.
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
//! the 64-bit rounding increased the magnitude; PE (sticky) on every inexact
//! result, i.e. any nonzero in-range operand; ±∞ is an invalid operation
//! (IE, result = the default QNaN "indefinite"); NaN propagates (quieted).
//! All of these were confirmed on an AMD Ryzen 7 9800X3D (oracle kit 3).
//!
//! FSIN/FCOS/FPTAN take a fast path first (`fast`): the same steps in 128-bit
//! integers and double-double arithmetic, kept only when the result's distance
//! from every 64-bit rounding point proves the rounding (and C1) the same as
//! here; otherwise, and outside its range, the arbitrary-precision model above.
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
    Value { value: f64, c1: bool, ext_inexact: bool, inexact: bool },
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
        return TrigOutcome::Value { value, c1: false, ext_inexact: false, inexact: false };
    }
    if let Some(v) = fast::trig(op, x) {
        return v;
    }
    exact_trig(op, x)
}

/// The arbitrary-precision model, for finite nonzero |x| < 2⁶³.
pub(crate) fn exact_trig(op: TrigOp, x: f64) -> TrigOutcome {
    let frac = if x.abs() < 8.271806125530277e-25 { FRAC_TINY } else { FRAC_NORMAL }; // 2^-80
    let xf = to_fixed(x, frac);
    let half_pi = BigInt::from(PI66_SCALED) << (frac - 65);
    let k = round_div(&xf, &half_pi);
    let mut r = &xf - &k * &half_pi;
    let q = k.mod_floor(&BigInt::from(4)).to_u32().unwrap();
    // Small reduced arguments: the series correction (r³/6 for sin, r²/2 for
    // cos) decides the rounding direction (C1) and must stay well above the
    // working precision. r is exact, so widen the fixed point losslessly.
    let mut frac = frac;
    let magnitude = frac as i64 - r.bits() as i64; // ≈ −log2|r|
    if magnitude > 16 {
        let extra = (2 * magnitude) as u32;
        r <<= extra;
        frac += extra;
    }
    let one = BigInt::one() << frac;
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
    // sin/cos/tan of a nonzero binary64 is irrational: always inexact (PE).
    TrigOutcome::Value { value, c1, ext_inexact, inexact: true }
}

/// x87 FPATAN: `atan2(y, x)` for ST(1) = y, ST(0) = x, with the result's sign and
/// quadrant per the x87/C99 table. The angle is computed exactly in wide fixed
/// point, rounded to the 64-bit significand (C1), then to binary64 like the
/// other functions here. Unlike FSIN, FPATAN takes no argument reduction, so
/// the hardware's internal π only enters through the quadrant terms (π/2, π).
/// Those use π₆₆, as FSIN's reduction does. Evidence (AMD Ryzen 7 9800X3D):
/// atan2(2, −3) = π − atan(2/3) lies just above a 64-bit rounding midpoint, so
/// with the true π it rounds up (C1 = 1); the hardware rounds it down (C1 = 0),
/// as π₆₆ (about 4·10⁻²¹ below π) does.
pub fn x87_fpatan(y: f64, x: f64) -> TrigOutcome {
    if y.is_nan() || x.is_nan() {
        // Two NaNs: the larger significand wins; one: that one, quieted.
        let pick = match (y.is_nan(), x.is_nan()) {
            (true, true) => if (y.to_bits() & 0x000F_FFFF_FFFF_FFFF) >= (x.to_bits() & 0x000F_FFFF_FFFF_FFFF) { y } else { x },
            (true, false) => y,
            _ => x,
        };
        return TrigOutcome::Nan(f64::from_bits(pick.to_bits() | 0x0008_0000_0000_0000));
    }
    let neg = y.is_sign_negative();
    let x_neg = x.is_sign_negative();
    // Table entries, as multiples of π/4 (0: an exact signed zero).
    let special = if y == 0.0 {
        Some(if x_neg { 4 } else { 0 })
    } else if y.is_infinite() {
        Some(if x == f64::INFINITY { 1 } else if x == f64::NEG_INFINITY { 3 } else { 2 })
    } else if x == 0.0 {
        Some(2)
    } else if x.is_infinite() {
        Some(if x_neg { 4 } else { 0 })
    } else {
        None
    };
    let (v, frac) = match special {
        Some(0) => return TrigOutcome::Value { value: if neg { -0.0 } else { 0.0 }, c1: false, ext_inexact: false, inexact: false },
        Some(n) => {
            let frac = FRAC_NORMAL;
            (pi_fixed(frac) * BigInt::from(n) / BigInt::from(4), frac)
        }
        None => {
            let (ay, ax) = (y.abs(), x.abs());
            let (small, big) = if ay <= ax { (ay, ax) } else { (ax, ay) };
            let (ms, es) = f64_parts(small);
            let (mb, eb) = f64_parts(big);
            // log2(small/big) ≈ (es + bits(ms)) − (eb + bits(mb)); widen the fixed
            // point for small ratios so the result keeps its relative precision.
            let l2 = (es + ms.bits() as i64) - (eb + mb.bits() as i64);
            let frac = FRAC_NORMAL + if l2 < -16 { (-3 * l2) as u32 } else { 0 };
            let sh = frac as i64 + es - eb;
            let t = if sh >= 0 { (ms << sh as u32) / &mb } else { ms / (mb << (-sh) as u32) };
            let mut a = atan_fixed(&t, frac);
            let pi = BigInt::from(PI66_SCALED) << (frac - 64);
            if ay > ax {
                a = (&pi >> 1u32) - a;
            }
            if x_neg {
                a = pi - a;
            }
            (a, frac)
        }
    };
    let v = if neg { -v } else { v };
    let (value, c1, ext_inexact) = round_fixed_to_x87(&v, frac);
    TrigOutcome::Value { value, c1, ext_inexact, inexact: true }
}

/// A finite nonzero binary64 as `m · 2^e` (m a positive integer).
fn f64_parts(v: f64) -> (BigInt, i64) {
    let b = v.to_bits();
    let exp = ((b >> 52) & 0x7ff) as i64;
    let frac = b & 0x000F_FFFF_FFFF_FFFF;
    if exp == 0 {
        (BigInt::from(frac), -1074)
    } else {
        (BigInt::from(frac | (1 << 52)), exp - 1075)
    }
}

/// atan(t) for 0 ≤ t ≤ 1 in fixed point: halve the angle with
/// atan(t) = 2·atan(t / (1 + √(1 + t²))) until t ≤ 1/8, then the Taylor series.
fn atan_fixed(t: &BigInt, frac: u32) -> BigInt {
    let one = BigInt::one() << frac;
    let mut t = t.clone();
    let mut doublings = 0u32;
    while t > (&one >> 3u32) {
        let t2 = mul_fixed(&t, &t, frac);
        let root = ((&one + t2) << frac).sqrt();
        t = div_fixed(&t, &(&one + root), frac);
        doublings += 1;
    }
    atan_series(&t, frac) << doublings
}

/// t − t³/3 + t⁵/5 − … in fixed point (|t| ≤ 1/8, or 1/5 for π).
fn atan_series(t: &BigInt, frac: u32) -> BigInt {
    let t2 = mul_fixed(t, t, frac);
    let mut power = t.clone();
    let mut sum = BigInt::zero();
    let mut n = 1u32;
    while !power.is_zero() {
        let term = &power / BigInt::from(n);
        if (n / 2).is_multiple_of(2) { sum += term } else { sum -= term }
        power = mul_fixed(&power, &t2, frac);
        n += 2;
    }
    sum
}

/// π in fixed point (Machin: 16·atan(1/5) − 4·atan(1/239)), cached per width.
fn pi_fixed(frac: u32) -> BigInt {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static CACHE: Mutex<Option<HashMap<u32, BigInt>>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap();
    let map = cache.get_or_insert_with(HashMap::new);
    map.entry(frac)
        .or_insert_with(|| {
            // 16 guard bits absorb the series' truncation errors.
            let f = frac + 16;
            let one = BigInt::one() << f;
            let a = atan_series(&(&one / BigInt::from(5)), f);
            let b = atan_series(&(&one / BigInt::from(239)), f);
            (a * BigInt::from(16) - b * BigInt::from(4)) >> 16u32
        })
        .clone()
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
    let (f, ext_inexact) = round64_to_f64(m64, e64);
    (if neg { -f } else { f }, c1, ext_inexact)
}

/// `m64 · 2^e64` (2^63 ≤ m64 < 2^64) rounded to binary64; and whether that was inexact.
fn round64_to_f64(m64: u128, e64: i64) -> (f64, bool) {
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
    (f, ext_inexact)
}

/// The fast path: exact argument reduction in i128 and the series in
/// double-double (about 106 bits), accepted only when provably rounded as the
/// exact model rounds.
mod fast {
    use super::{round64_to_f64, TrigOp, TrigOutcome, PI66_SCALED};

    /// A double-double: `hi + lo`, |lo| ≤ ulp(hi)/2.
    #[derive(Clone, Copy)]
    struct Dd(f64, f64);

    fn two_sum(a: f64, b: f64) -> Dd {
        let s = a + b;
        let bb = s - a;
        Dd(s, (a - (s - bb)) + (b - bb))
    }
    fn split(a: f64) -> (f64, f64) {
        let t = 134_217_729.0 * a; // 2^27 + 1
        let hi = t - (t - a);
        (hi, a - hi)
    }
    fn two_prod(a: f64, b: f64) -> Dd {
        let p = a * b;
        let (ah, al) = split(a);
        let (bh, bl) = split(b);
        Dd(p, ((ah * bh - p) + ah * bl + al * bh) + al * bl)
    }
    fn add(a: Dd, b: Dd) -> Dd {
        let s = two_sum(a.0, b.0);
        let t = two_sum(a.1, b.1);
        let s = two_sum(s.0, s.1 + t.0);
        two_sum(s.0, s.1 + t.1)
    }
    fn mul(a: Dd, b: Dd) -> Dd {
        let p = two_prod(a.0, b.0);
        two_sum(p.0, p.1 + (a.0 * b.1 + a.1 * b.0))
    }
    fn div(a: Dd, b: Dd) -> Dd {
        let q1 = a.0 / b.0;
        let r = add(a, neg(mul(b, Dd(q1, 0.0))));
        let q2 = r.0 / b.0;
        let r = add(r, neg(mul(b, Dd(q2, 0.0))));
        let q3 = r.0 / b.0;
        add(two_sum(q1, q2), Dd(q3, 0.0))
    }
    fn div_small(a: Dd, n: f64) -> Dd {
        div(a, Dd(n, 0.0))
    }
    fn neg(a: Dd) -> Dd {
        Dd(-a.0, -a.1)
    }

    /// Taylor terms kept: for |r| ≤ 0.79 the first left out is below 2⁻⁹⁷ of the
    /// result, well inside the 2⁻⁹⁰ that `round` allows for.
    const TERMS: usize = 13;

    /// (−1)ᵏ/(2k+1)! and (−1)ᵏ/(2k)! for k < TERMS, as double-doubles.
    fn coefficients() -> &'static ([Dd; TERMS], [Dd; TERMS]) {
        static C: std::sync::OnceLock<([Dd; TERMS], [Dd; TERMS])> = std::sync::OnceLock::new();
        C.get_or_init(|| {
            let (mut sin, mut cos) = ([Dd(0.0, 0.0); TERMS], [Dd(0.0, 0.0); TERMS]);
            let mut f = Dd(1.0, 0.0); // 1/n!
            for n in 0..2 * TERMS {
                if n > 0 {
                    f = div_small(f, n as f64);
                }
                let signed = if (n / 2) % 2 == 0 { f } else { neg(f) };
                if n % 2 == 0 { cos[n / 2] = signed } else { sin[n / 2] = signed }
            }
            (sin, cos)
        })
    }

    /// sin r and cos r for |r| ≤ π/4 (Horner, in r²).
    fn sin_cos(r: Dd, want_sin: bool, want_cos: bool) -> (Dd, Dd) {
        let r2 = mul(r, r);
        let (cs, cc) = coefficients();
        let horner = |c: &[Dd; TERMS]| c.iter().rev().fold(Dd(0.0, 0.0), |acc, &k| add(mul(acc, r2), k));
        let s = if want_sin { mul(r, horner(cs)) } else { Dd(0.0, 0.0) };
        let c = if want_cos { horner(cc) } else { Dd(0.0, 0.0) };
        (s, c)
    }

    /// x − k·π₆₆/2 for |x| ≥ 0.78 (exact: x and π₆₆/2 are integers at 2⁻⁶⁵ there),
    /// as a double-double, with k mod 4.
    fn reduce(x: f64) -> (Dd, u32) {
        let bits = x.to_bits();
        let mant = ((bits & ((1u64 << 52) - 1)) | (1u64 << 52)) as i128;
        let e = ((bits >> 52) & 0x7ff) as i32 - 1075 + 65;
        let xi = if e >= 0 { mant << e } else { mant >> -e };
        let xi = if x < 0.0 { -xi } else { xi };
        let p = PI66_SCALED as i128; // π₆₆/2 · 2⁶⁵
        // k = round(x / (π₆₆/2)), ties to even.
        let (q, rem) = (xi.div_euclid(p), xi.rem_euclid(p));
        let k = match (2 * rem).cmp(&p) {
            std::cmp::Ordering::Greater => q + 1,
            std::cmp::Ordering::Less => q,
            std::cmp::Ordering::Equal => q + (q & 1),
        };
        let ri = xi - k * p;
        let quadrant = k.rem_euclid(4) as u32;
        let r = {
            let hi = ri as f64;
            let lo = (ri - hi as i128) as f64;
            let scale = 2f64.powi(-65);
            Dd(hi * scale, lo * scale)
        };
        (r, quadrant)
    }

    pub(super) fn trig(op: TrigOp, x: f64) -> Option<TrigOutcome> {
        let ax = x.abs();
        // Tiny: the correction (x³/6, x²/2, x³/3) is below half a 64-bit ULP, so
        // the result rounds to x (or 1), and C1 follows its sign.
        if ax < 1.164_153_218_269_348_1e-10 {
            // 2⁻³³
            let (value, c1) = match op {
                TrigOp::Sin => (x, true),
                TrigOp::Tan => (x, false),
                TrigOp::Cos => (1.0, true),
            };
            return Some(TrigOutcome::Value { value, c1, ext_inexact: false, inexact: true });
        }
        if ax >= 1.099_511_627_776e12 {
            return None; // |x| ≥ 2⁴⁰
        }
        let (r, quadrant) = if ax < 0.78 {
            // Below π₆₆/4 (≈ 0.785): k = 0, r = x exactly.
            (Dd(x, 0.0), 0)
        } else {
            reduce(x)
        };
        let (need_sin, need_cos) = match (op, quadrant % 2) {
            (TrigOp::Tan, _) => (true, true),
            (TrigOp::Sin, 0) | (TrigOp::Cos, 1) => (true, false),
            _ => (false, true),
        };
        let (s, c) = sin_cos(r, need_sin, need_cos);
        let v = match (op, quadrant) {
            (TrigOp::Sin, 0) => s,
            (TrigOp::Sin, 1) => c,
            (TrigOp::Sin, 2) => neg(s),
            (TrigOp::Sin, _) => neg(c),
            (TrigOp::Cos, 0) => c,
            (TrigOp::Cos, 1) => neg(s),
            (TrigOp::Cos, 2) => neg(c),
            (TrigOp::Cos, _) => s,
            (TrigOp::Tan, q) => if q % 2 == 0 { div(s, c) } else { neg(div(c, s)) },
        };
        round(v)
    }

    /// Round `v` to a 64-bit significand and then to binary64, as the exact model
    /// does, if `v` is far enough (2⁻⁹⁰ relative) from every 64-bit rounding
    /// point for its error not to matter; None otherwise.
    fn round(v: Dd) -> Option<TrigOutcome> {
        if v.0 == 0.0 || !v.0.is_finite() {
            return None;
        }
        let neg = v.0 < 0.0;
        let (hi, lo) = if neg { (-v.0, -v.1) } else { (v.0, v.1) };
        // v · 2^(105 − E) as an integer, E = ⌊log2 hi⌋ (truncation error < 1 unit).
        let e = ((hi.to_bits() >> 52) & 0x7ff) as i32 - 1023;
        if e < -1000 {
            return None;
        }
        let scale = 2f64.powi(105 - e);
        let hi_i = (hi * scale) as i128; // exact: hi has 53 bits
        let lo_i = (lo * scale).trunc() as i128;
        let a = hi_i + lo_i;
        if a <= 0 {
            return None;
        }
        let bits = 128 - a.leading_zeros() as i32;
        let sh = bits - 64;
        if sh < 2 {
            return None;
        }
        let rem = a & ((1i128 << sh) - 1);
        let half = 1i128 << (sh - 1);
        // Uncertainty: 2⁻⁹⁰ relative plus the truncation.
        let err = (a >> 90) + 2;
        let near = |p: i128| (rem - p).abs() <= err;
        if near(0) || near(half) || near(2 * half) {
            return None;
        }
        let up = rem > half;
        let m64 = ((a >> sh) + up as i128) as u128;
        let mut e64 = (e - 105 + sh) as i64;
        let m64 = if m64 >> 64 != 0 {
            e64 += 1;
            m64 >> 1
        } else {
            m64
        };
        let (f, ext_inexact) = round64_to_f64(m64, e64);
        Some(TrigOutcome::Value { value: if neg { -f } else { f }, c1: up, ext_inexact, inexact: true })
    }
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

    /// Hardware values from an AMD Ryzen 7 9800X3D (FSTP m64, CW 0x027F): exact
    /// measured bit patterns, not approximations of library constants.
    #[test]
    #[allow(clippy::approx_constant)]
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

    fn fpatan_value(y: f64, x: f64) -> f64 {
        match x87_fpatan(y, x) {
            TrigOutcome::Value { value, .. } => value,
            TrigOutcome::Nan(v) => v,
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn fpatan_follows_the_x87_table_for_zeros_and_infinities() {
        use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};
        let inf = f64::INFINITY;
        for (y, x, want) in [
            (0.0, 1.0, 0.0), (-0.0, 1.0, -0.0), (0.0, 0.0, 0.0), (-0.0, 0.0, -0.0),
            (0.0, -1.0, PI), (-0.0, -1.0, -PI), (0.0, -0.0, PI), (-0.0, -0.0, -PI),
            (1.0, 0.0, FRAC_PI_2), (-1.0, -0.0, -FRAC_PI_2),
            (inf, 1.0, FRAC_PI_2), (inf, inf, FRAC_PI_4), (inf, -inf, 3.0 * FRAC_PI_4), (-inf, -inf, -3.0 * FRAC_PI_4),
            (1.0, inf, 0.0), (-1.0, inf, -0.0), (1.0, -inf, PI), (-1.0, -inf, -PI),
        ] {
            let got = fpatan_value(y, x);
            assert_eq!(got.to_bits(), want.to_bits(), "atan2({y}, {x}) = {got}, want {want}");
        }
        assert!(fpatan_value(f64::NAN, 1.0).is_nan());
        assert!(matches!(x87_fpatan(1.0, 2.0), TrigOutcome::Value { inexact: true, .. }));
    }

    /// Against the host's atan2 over a spread of magnitudes and quadrants: equal,
    /// or (rarely, where rounding first to 64 bits changes the binary64 result)
    /// one ULP apart.
    #[test]
    fn fpatan_agrees_with_libm_within_double_rounding() {
        let mut seed = 0x1234_5678_9abc_def0u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let (mut equal, mut one_ulp) = (0, 0);
        for _ in 0..3000 {
            let mag = |r: u64| (((r >> 11) as f64) / (1u64 << 53) as f64) * 2f64.powi((r % 60) as i32 - 30);
            let (ry, rx) = (next(), next());
            let y = if ry & 1 == 0 { mag(ry) } else { -mag(ry) };
            let x = if rx & 1 == 0 { mag(rx) } else { -mag(rx) };
            let (got, want) = (fpatan_value(y, x), y.atan2(x));
            let d = (got.to_bits() as i64 - want.to_bits() as i64).abs();
            assert!(d <= 1, "atan2({y:e}, {x:e}): model {got:e}, libm {want:e}");
            if d == 0 { equal += 1 } else { one_ulp += 1 }
        }
        assert!(equal > 2900, "{equal} equal, {one_ulp} one ULP apart");
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
        // 2⁶³ is out of range; the largest binary64 below it is in.
        let two63 = 2f64.powi(63);
        assert_eq!(x87_trig(TrigOp::Sin, two63), TrigOutcome::OutOfRange);
        assert_eq!(x87_trig(TrigOp::Cos, -two63), TrigOutcome::OutOfRange);
        assert!(matches!(x87_trig(TrigOp::Sin, two63 - 1024.0), TrigOutcome::Value { .. }));
        // C1 for tiny arguments (hardware: sin(1e-20) rounds up in magnitude).
        assert!(matches!(x87_trig(TrigOp::Sin, 1.0e-20), TrigOutcome::Value { c1: true, inexact: true, .. }));
        assert!(matches!(x87_trig(TrigOp::Tan, 1.0e-20), TrigOutcome::Value { c1: false, .. }));
        assert!(matches!(x87_trig(TrigOp::Cos, 1.0e-20), TrigOutcome::Value { c1: true, .. }));
        assert_eq!(x87_trig(TrigOp::Sin, f64::INFINITY), TrigOutcome::Invalid);
        assert!(matches!(x87_trig(TrigOp::Cos, f64::NAN), TrigOutcome::Nan(_)));
        // Tiny and subnormal arguments (the game passes denormal float angles).
        for &x in &[1.0e-30, 1.0e-300, 5.0e-324, 1.4e-45 * 0.5] {
            assert_eq!(val(TrigOp::Sin, x), x, "sin({x})");
            assert_eq!(val(TrigOp::Tan, x), x, "tan({x})");
            assert_eq!(val(TrigOp::Cos, x), 1.0, "cos({x})");
        }
    }

    /// The fast path, where it answers, matches the arbitrary-precision model bit
    /// for bit (value, C1, inexactness): across magnitudes, both signs, and right
    /// next to multiples of π/2, where the reduction cancels most.
    #[test]
    fn fast_path_matches_the_exact_model() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let n = if cfg!(debug_assertions) { 20_000 } else { 400_000 };
        let (mut answered, mut declined) = (0u32, 0u32);
        for i in 0..n {
            let r = next();
            let x = match i % 4 {
                // Any magnitude from 2⁻⁴⁰ to 2⁴⁰.
                0 | 1 => (((r >> 11) as f64) / (1u64 << 53) as f64 + 0.5) * 2f64.powi((r % 80) as i32 - 40),
                // Next to k·π/2.
                2 => {
                    let k = (r % 2000) as f64;
                    let base = k * std::f64::consts::FRAC_PI_2;
                    f64::from_bits(base.to_bits().wrapping_add((r >> 40) % 64).wrapping_sub(32))
                }
                // Typical game angles.
                _ => ((r >> 11) as f64) / (1u64 << 53) as f64 * 20.0 - 10.0,
            };
            let x = if r & (1 << 62) != 0 { -x } else { x };
            if x == 0.0 || !x.is_finite() {
                continue;
            }
            for op in [TrigOp::Sin, TrigOp::Cos, TrigOp::Tan] {
                match fast::trig(op, x) {
                    Some(f) => {
                        answered += 1;
                        assert_eq!(f, exact_trig(op, x), "{op:?}({x:e}) = {:#x}", x.to_bits());
                    }
                    None => declined += 1,
                }
            }
        }
        let rate = declined as f64 / (answered + declined) as f64;
        println!("fast path: {answered} answered, {declined} declined ({:.4}%)", rate * 100.0);
        assert!(answered > 0);
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
