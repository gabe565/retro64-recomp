/*
 * recomp.h — machine model for statically recompiled 32-bit x86 code.
 *
 * Every generated function has the signature `void f_XXXXXXXX(Cpu *c)`.
 * Semantics are spelled out explicitly so generated code never depends on
 * undefined or implementation-defined C behaviour:
 *   - all integer arithmetic is done on unsigned types (wrapping is defined);
 *   - signed views go through memcpy-based reinterpretation;
 *   - shift counts are masked exactly as x86 masks them, and never reach the
 *     width of the C type;
 *   - guest memory is accessed with memcpy (no alignment or aliasing UB);
 *   - x87 values are IEEE binary64 with explicit rounding helpers. This is only
 *     exact under precision control = 53 bits, which is what the original game
 *     runs at (measured: control word 0x027F). Other precision or rounding modes
 *     are reported through rt_x87_note().
 *
 * The host must be little-endian (checked at runtime start-up by the Rust side).
 * Portable C99; no compiler extensions.
 */
#ifndef BUGATRON_RECOMP_H
#define BUGATRON_RECOMP_H

#include <stdint.h>
#include <string.h>
#include <math.h>

/* ---- CPU state (layout mirrored in crates/runtime/src/cpu.rs) ---------- */
typedef struct Cpu {
    uint32_t eax, ecx, edx, ebx, esp, ebp, esi, edi;   /* 0x00 */
    uint32_t fl_kind, fl_size, fl_a, fl_b, fl_res, fl_cin; /* 0x20 lazy flags */
    uint32_t df;                                         /* 0x38 direction flag */
    uint32_t fs_base;                                    /* 0x3c */
    uint32_t ftop;                                       /* 0x40 x87 TOP */
    uint32_t fcw, fsw, ftag;                             /* 0x44 control/status/tag (low 16 bits used) */
    uint32_t x87_notes;                                  /* 0x50 bitmask of rt_x87_note kinds raised */
    uint32_t pad0;
    double st[8];                                        /* 0x58 physical registers R0..R7 */
} Cpu;

extern uint8_t *recomp_mem;  /* base of the 4 GiB guest address space */

/* Runtime services implemented in Rust. */
void rt_import(Cpu *c, uint32_t thunk_addr);
void rt_override(Cpu *c, uint32_t func_addr);
void rt_unresolved_call(Cpu *c, uint32_t target, uint32_t site);
void rt_unimpl(Cpu *c, uint32_t addr, const char *what);
void rt_bad_jump(Cpu *c, uint32_t site, uint32_t target);
void rt_div_error(Cpu *c, uint32_t site);
void rt_trap(Cpu *c, uint32_t site, const char *reason);
void rt_x87_note(Cpu *c, uint32_t kind);
void rt_note_indirect(uint32_t target, uint32_t site);

#define RT_THUNK_BASE 0xF0000000u
#define RT_THUNK_END  0xF1000000u

typedef void (*RecompFn)(Cpu *);
RecompFn recomp_lookup(uint32_t addr);
void recomp_call_indirect(Cpu *c, uint32_t target, uint32_t site);

/* ---- guest memory ------------------------------------------------------- */
static inline uint8_t  rd8 (uint32_t a) { return recomp_mem[a]; }
static inline uint16_t rd16(uint32_t a) { uint16_t v; memcpy(&v, recomp_mem + a, 2); return v; }
static inline uint32_t rd32(uint32_t a) { uint32_t v; memcpy(&v, recomp_mem + a, 4); return v; }
static inline uint64_t rd64(uint32_t a) { uint64_t v; memcpy(&v, recomp_mem + a, 8); return v; }
static inline void wr8 (uint32_t a, uint8_t  v) { recomp_mem[a] = v; }
static inline void wr16(uint32_t a, uint16_t v) { memcpy(recomp_mem + a, &v, 2); }
static inline void wr32(uint32_t a, uint32_t v) { memcpy(recomp_mem + a, &v, 4); }
static inline void wr64(uint32_t a, uint64_t v) { memcpy(recomp_mem + a, &v, 8); }

static inline void push32(Cpu *c, uint32_t v) { c->esp -= 4; wr32(c->esp, v); }
static inline uint32_t pop32(Cpu *c) { uint32_t v = rd32(c->esp); c->esp += 4; return v; }

/* ---- signed reinterpretation (defined behaviour) ------------------------ */
static inline int32_t s32(uint32_t x) { int32_t r; memcpy(&r, &x, 4); return r; }
static inline int64_t s64(uint64_t x) { int64_t r; memcpy(&r, &x, 8); return r; }
static inline uint32_t sx8(uint32_t x)  { x &= 0xffu;   return (x & 0x80u)   ? (x | 0xffffff00u) : x; }
static inline uint32_t sx16(uint32_t x) { x &= 0xffffu; return (x & 0x8000u) ? (x | 0xffff0000u) : x; }
static inline uint64_t sx32_64(uint32_t x) { return (x & 0x80000000u) ? ((uint64_t)x | 0xffffffff00000000ull) : (uint64_t)x; }

/* ---- partial registers --------------------------------------------------- */
static inline uint32_t set8lo(uint32_t r, uint32_t v)  { return (r & 0xffffff00u) | (v & 0xffu); }
static inline uint32_t set8hi(uint32_t r, uint32_t v)  { return (r & 0xffff00ffu) | ((v & 0xffu) << 8); }
static inline uint32_t set16(uint32_t r, uint32_t v)   { return (r & 0xffff0000u) | (v & 0xffffu); }

/* ---- flags --------------------------------------------------------------- */
enum { FK_EXPLICIT = 0, FK_ADD, FK_SUB, FK_LOGIC, FK_INC, FK_DEC, FK_SHL, FK_SHR, FK_SAR,
       FK_ADC, FK_SBB, FK_MUL };
#define F_CF 0x001u
#define F_PF 0x004u
#define F_AF 0x010u
#define F_ZF 0x040u
#define F_SF 0x080u
#define F_OF 0x800u

static inline uint32_t szmask(uint32_t size) { return size == 8 ? 0xffu : size == 16 ? 0xffffu : 0xffffffffu; }
static inline uint32_t szmsb(uint32_t size)  { return size == 8 ? 0x80u : size == 16 ? 0x8000u : 0x80000000u; }

static inline void setfl(Cpu *c, uint32_t kind, uint32_t size, uint32_t a, uint32_t b, uint32_t res) {
    c->fl_kind = kind; c->fl_size = size; c->fl_a = a; c->fl_b = b; c->fl_res = res;
}

static inline uint32_t parity8(uint32_t v) {
    v &= 0xffu; v ^= v >> 4; v ^= v >> 2; v ^= v >> 1;
    return (~v) & 1u;
}

/* Compute the full arithmetic flag set from the lazy record. */
static inline uint32_t flags_get(Cpu *c) {
    uint32_t k = c->fl_kind;
    if (k == FK_EXPLICIT) return c->fl_res;
    uint32_t m = szmask(c->fl_size), msb = szmsb(c->fl_size);
    uint32_t a = c->fl_a & m, b = c->fl_b & m, r = c->fl_res & m, f = 0;
    switch (k) {
    case FK_ADD: case FK_ADC:
        if (k == FK_ADD ? (r < a) : (r < a || (c->fl_cin && r == a))) f |= F_CF;
        if (((a ^ r) & (b ^ r)) & msb) f |= F_OF;
        if ((a ^ b ^ r) & 0x10u) f |= F_AF;
        break;
    case FK_SUB: case FK_SBB:
        if (k == FK_SUB ? (a < b) : ((uint64_t)a < (uint64_t)b + (uint64_t)(c->fl_cin & 1u))) f |= F_CF;
        if (((a ^ b) & (a ^ r)) & msb) f |= F_OF;
        if ((a ^ b ^ r) & 0x10u) f |= F_AF;
        break;
    case FK_LOGIC:
        break;
    case FK_INC:
        if (c->fl_cin) f |= F_CF;
        if (r == msb) f |= F_OF;
        if ((r & 0xfu) == 0) f |= F_AF;
        break;
    case FK_DEC:
        if (c->fl_cin) f |= F_CF;
        if (r == (msb - 1u)) f |= F_OF;
        if ((r & 0xfu) == 0xfu) f |= F_AF;
        break;
    case FK_SHL: /* b = count (1..31) */
        if (b <= c->fl_size && ((a >> (c->fl_size - b)) & 1u)) f |= F_CF;
        if (((r & msb) != 0) != ((f & F_CF) != 0)) f |= F_OF;
        break;
    case FK_SHR:
        if ((a >> (b - 1u)) & 1u) f |= F_CF;
        if (a & msb) f |= F_OF;
        break;
    case FK_SAR: {
        uint32_t sa = (c->fl_size == 8) ? sx8(a) : (c->fl_size == 16) ? sx16(a) : a;
        uint32_t sh = b - 1u;
        uint32_t bit = (sh >= 31u) ? (sa >> 31) : ((sa >> sh) & 1u);
        if (sh >= c->fl_size) bit = (sa >> 31) & 1u;
        if (bit & 1u) f |= F_CF;
        break;
    }
    case FK_MUL: /* b = 1 if CF/OF set */
        if (b) f |= F_CF | F_OF;
        break;
    }
    if (r == 0) f |= F_ZF;
    if (r & msb) f |= F_SF;
    f |= parity8(r) ? F_PF : 0;
    return f;
}

static inline void flags_set_explicit(Cpu *c, uint32_t f) { c->fl_kind = FK_EXPLICIT; c->fl_res = f; }
/* Single flags and conditions: direct for the common kinds, each the same as
 * flags_get's (the x86 semantics suite checks them); flags_get otherwise. */
static inline int32_t sxsz(uint32_t v, uint32_t size) {
    return size == 8 ? (int32_t)(int8_t)v : size == 16 ? (int32_t)(int16_t)v : (int32_t)v;
}
static inline uint32_t get_cf(Cpu *c) {
    uint32_t m = szmask(c->fl_size);
    switch (c->fl_kind) {
    case FK_SUB: return (c->fl_a & m) < (c->fl_b & m);
    case FK_ADD: return (c->fl_res & m) < (c->fl_a & m);
    case FK_LOGIC: return 0;
    case FK_INC: case FK_DEC: return c->fl_cin ? 1u : 0u;
    case FK_EXPLICIT: return c->fl_res & F_CF;
    default: return flags_get(c) & F_CF;
    }
}

/* Condition codes (Jcc/SETcc). */
static inline int cc_o (Cpu *c) {
    uint32_t m = szmask(c->fl_size), msb = szmsb(c->fl_size);
    uint32_t a = c->fl_a & m, b = c->fl_b & m, r = c->fl_res & m;
    switch (c->fl_kind) {
    case FK_SUB: return (((a ^ b) & (a ^ r)) & msb) != 0;
    case FK_ADD: return (((a ^ r) & (b ^ r)) & msb) != 0;
    case FK_LOGIC: return 0;
    case FK_INC: return r == msb;
    case FK_DEC: return r == msb - 1u;
    default: return (flags_get(c) & F_OF) != 0;
    }
}
static inline int cc_b (Cpu *c) { return get_cf(c) != 0; }
static inline int cc_e (Cpu *c) {
    if (c->fl_kind != FK_EXPLICIT) return (c->fl_res & szmask(c->fl_size)) == 0;
    return (c->fl_res & F_ZF) != 0;
}
static inline int cc_be(Cpu *c) { return cc_b(c) || cc_e(c); }
static inline int cc_s (Cpu *c) {
    if (c->fl_kind != FK_EXPLICIT) return (c->fl_res & szmsb(c->fl_size)) != 0;
    return (c->fl_res & F_SF) != 0;
}
static inline int cc_p (Cpu *c) {
    /* Every lazy kind takes PF from the result's low byte. */
    if (c->fl_kind != FK_EXPLICIT) return parity8(c->fl_res & szmask(c->fl_size)) != 0;
    return (c->fl_res & F_PF) != 0;
}
/* SF != OF: after CMP/SUB, a signed a < b; after a logic op (OF = 0), SF. */
static inline int cc_l (Cpu *c) {
    uint32_t sz = c->fl_size, m = szmask(sz);
    switch (c->fl_kind) {
    case FK_SUB: return sxsz(c->fl_a & m, sz) < sxsz(c->fl_b & m, sz);
    case FK_LOGIC: return (c->fl_res & szmsb(sz)) != 0;
    default: { uint32_t f = flags_get(c); return ((f & F_SF) != 0) != ((f & F_OF) != 0); }
    }
}
static inline int cc_le(Cpu *c) {
    uint32_t sz = c->fl_size, m = szmask(sz);
    switch (c->fl_kind) {
    case FK_SUB: return sxsz(c->fl_a & m, sz) <= sxsz(c->fl_b & m, sz);
    case FK_LOGIC: return (c->fl_res & m) == 0 || (c->fl_res & szmsb(sz)) != 0;
    default: { uint32_t f = flags_get(c); return (f & F_ZF) || (((f & F_SF) != 0) != ((f & F_OF) != 0)); }
    }
}

/* ---- integer helpers ------------------------------------------------------ */
static inline uint32_t op_add(Cpu *c, uint32_t sz, uint32_t a, uint32_t b) {
    uint32_t r = (a + b) & szmask(sz); setfl(c, FK_ADD, sz, a, b, r); return r;
}
static inline uint32_t op_adc(Cpu *c, uint32_t sz, uint32_t a, uint32_t b) {
    uint32_t cin = get_cf(c); uint32_t r = (a + b + cin) & szmask(sz);
    setfl(c, FK_ADC, sz, a, b, r); c->fl_cin = cin; return r;
}
static inline uint32_t op_sub(Cpu *c, uint32_t sz, uint32_t a, uint32_t b) {
    uint32_t r = (a - b) & szmask(sz); setfl(c, FK_SUB, sz, a, b, r); return r;
}
static inline uint32_t op_sbb(Cpu *c, uint32_t sz, uint32_t a, uint32_t b) {
    uint32_t cin = get_cf(c); uint32_t r = (a - b - cin) & szmask(sz);
    setfl(c, FK_SBB, sz, a, b, r); c->fl_cin = cin; return r;
}
static inline uint32_t op_logic(Cpu *c, uint32_t sz, uint32_t r) {
    r &= szmask(sz); setfl(c, FK_LOGIC, sz, 0, 0, r); return r;
}
static inline uint32_t op_inc(Cpu *c, uint32_t sz, uint32_t a) {
    uint32_t cin = get_cf(c); uint32_t r = (a + 1u) & szmask(sz);
    setfl(c, FK_INC, sz, a, 1, r); c->fl_cin = cin; return r;
}
static inline uint32_t op_dec(Cpu *c, uint32_t sz, uint32_t a) {
    uint32_t cin = get_cf(c); uint32_t r = (a - 1u) & szmask(sz);
    setfl(c, FK_DEC, sz, a, 1, r); c->fl_cin = cin; return r;
}
static inline uint32_t op_neg(Cpu *c, uint32_t sz, uint32_t a) {
    uint32_t r = (0u - a) & szmask(sz); setfl(c, FK_SUB, sz, 0, a, r); return r;
}
/* Shifts: count already masked to 5 bits by the caller; count 0 leaves flags. */
static inline uint32_t op_shl(Cpu *c, uint32_t sz, uint32_t a, uint32_t n) {
    if (n == 0) return a & szmask(sz);
    uint32_t r = (n >= 32u) ? 0u : ((a << n) & szmask(sz));
    setfl(c, FK_SHL, sz, a, n, r); return r;
}
static inline uint32_t op_shr(Cpu *c, uint32_t sz, uint32_t a, uint32_t n) {
    a &= szmask(sz);
    if (n == 0) return a;
    uint32_t r = (n >= 32u) ? 0u : (a >> n);
    setfl(c, FK_SHR, sz, a, n, r); return r;
}
static inline uint32_t op_sar(Cpu *c, uint32_t sz, uint32_t a, uint32_t n) {
    a &= szmask(sz);
    if (n == 0) return a;
    uint32_t sa = (sz == 8) ? sx8(a) : (sz == 16) ? sx16(a) : a;
    uint32_t r = (n >= 31u) ? ((sa & 0x80000000u) ? 0xffffffffu : 0u)
                            : ((sa >> n) | ((sa & 0x80000000u) ? ~(0xffffffffu >> n) : 0u));
    r &= szmask(sz);
    setfl(c, FK_SAR, sz, a, n, r); return r;
}
static inline uint32_t op_rol(Cpu *c, uint32_t sz, uint32_t a, uint32_t n) {
    a &= szmask(sz);
    uint32_t k = n % sz; if (n == 0) return a;
    uint32_t r = k ? (((a << k) | (a >> (sz - k))) & szmask(sz)) : a;
    uint32_t f = flags_get(c) & ~(F_CF | F_OF);
    if (r & 1u) f |= F_CF;
    if (((r & szmsb(sz)) != 0) != ((r & 1u) != 0)) f |= F_OF;
    flags_set_explicit(c, f); return r;
}
static inline uint32_t op_ror(Cpu *c, uint32_t sz, uint32_t a, uint32_t n) {
    a &= szmask(sz);
    uint32_t k = n % sz; if (n == 0) return a;
    uint32_t r = k ? (((a >> k) | (a << (sz - k))) & szmask(sz)) : a;
    uint32_t f = flags_get(c) & ~(F_CF | F_OF);
    if (r & szmsb(sz)) f |= F_CF;
    if (((r ^ (r << 1)) & szmsb(sz)) != 0) f |= F_OF;
    flags_set_explicit(c, f); return r;
}
/* RCL/RCR: rotate through carry. Count is masked to 5 bits by the caller,
 * then reduced modulo (size + 1). OF is defined only for a masked count of 1;
 * for other counts this model computes the same formula (documented as
 * "undefined" by Intel, so tests do not compare it). Other flags unchanged. */
static inline uint32_t op_rcl(Cpu *c, uint32_t sz, uint32_t a, uint32_t n) {
    a &= szmask(sz);
    uint32_t k = n % (sz + 1u);
    if (n == 0) return a;
    uint32_t cf = get_cf(c);
    uint64_t m = (sz == 32u) ? 0x1ffffffffull : ((1ull << (sz + 1u)) - 1u);
    uint64_t v = ((uint64_t)cf << sz) | (uint64_t)a;
    if (k) v = ((v << k) | (v >> (sz + 1u - k))) & m;
    uint32_t r = (uint32_t)(v & szmask(sz)), ncf = (uint32_t)((v >> sz) & 1u);
    uint32_t f = flags_get(c) & ~(F_CF | F_OF);
    if (ncf) f |= F_CF;
    if (((r & szmsb(sz)) != 0) != (ncf != 0)) f |= F_OF;
    flags_set_explicit(c, f); return r;
}
static inline uint32_t op_rcr(Cpu *c, uint32_t sz, uint32_t a, uint32_t n) {
    a &= szmask(sz);
    uint32_t k = n % (sz + 1u);
    if (n == 0) return a;
    uint32_t cf = get_cf(c);
    uint64_t m = (sz == 32u) ? 0x1ffffffffull : ((1ull << (sz + 1u)) - 1u);
    uint64_t v = ((uint64_t)cf << sz) | (uint64_t)a;
    if (k) v = ((v >> k) | (v << (sz + 1u - k))) & m;
    uint32_t r = (uint32_t)(v & szmask(sz)), ncf = (uint32_t)((v >> sz) & 1u);
    uint32_t f = flags_get(c) & ~(F_CF | F_OF);
    if (ncf) f |= F_CF;
    if (((a & szmsb(sz)) != 0) != (cf != 0)) f |= F_OF; /* MSB(dest) XOR CF, before rotation */
    flags_set_explicit(c, f); return r;
}
/* BT family: CF = selected bit; ZF unchanged; OF/SF/AF/PF undefined (kept). */
static inline void bt_setcf(Cpu *c, uint32_t bit) {
    uint32_t f = flags_get(c) & ~F_CF;
    if (bit) f |= F_CF;
    flags_set_explicit(c, f);
}
static inline void op_pushad(Cpu *c) {
    uint32_t t = c->esp;
    push32(c, c->eax); push32(c, c->ecx); push32(c, c->edx); push32(c, c->ebx);
    push32(c, t); push32(c, c->ebp); push32(c, c->esi); push32(c, c->edi);
}
static inline void op_popad(Cpu *c) {
    c->edi = pop32(c); c->esi = pop32(c); c->ebp = pop32(c); c->esp += 4u;
    c->ebx = pop32(c); c->edx = pop32(c); c->ecx = pop32(c); c->eax = pop32(c);
}
static inline uint32_t op_shld(Cpu *c, uint32_t a, uint32_t b, uint32_t n) {
    if (n == 0) return a;
    uint32_t r = (a << n) | (b >> (32u - n));
    uint32_t f = flags_get(c) & ~(F_CF | F_OF | F_ZF | F_SF | F_PF);
    if ((a >> (32u - n)) & 1u) f |= F_CF;
    if (r == 0) f |= F_ZF;
    if (r & 0x80000000u) f |= F_SF;
    if (parity8(r)) f |= F_PF;
    if (((r ^ a) & 0x80000000u) != 0) f |= F_OF;
    flags_set_explicit(c, f); return r;
}
static inline uint32_t op_shrd(Cpu *c, uint32_t a, uint32_t b, uint32_t n) {
    if (n == 0) return a;
    uint32_t r = (a >> n) | (b << (32u - n));
    uint32_t f = flags_get(c) & ~(F_CF | F_OF | F_ZF | F_SF | F_PF);
    if ((a >> (n - 1u)) & 1u) f |= F_CF;
    if (r == 0) f |= F_ZF;
    if (r & 0x80000000u) f |= F_SF;
    if (parity8(r)) f |= F_PF;
    if (((r ^ a) & 0x80000000u) != 0) f |= F_OF;
    flags_set_explicit(c, f); return r;
}

/* ---- x87 ------------------------------------------------------------------ */
enum { X87_NOTE_PC24 = 1, X87_NOTE_PC64 = 2, X87_NOTE_RC = 4, X87_NOTE_STACK = 8,
       X87_NOTE_TRANSCENDENTAL = 16, X87_NOTE_F80 = 32, X87_NOTE_EXT64 = 64 };
/* FSIN/FCOS/FPTAN with x87-compatible argument reduction, and FPATAN (runtime/src/x87trig.rs). */
void rt_x87_fsin(Cpu *c);
void rt_x87_fcos(Cpu *c);
void rt_x87_fptan(Cpu *c);
/* FPATAN: ST(1) = atan2(ST(1), ST(0)), then pop. */
void rt_x87_fpatan(Cpu *c);

#define ST(i) (c->st[(c->ftop + (uint32_t)(i)) & 7u])
static inline void fpush(Cpu *c, double v) {
    c->ftop = (c->ftop - 1u) & 7u;
    if (((c->ftag >> (2u * c->ftop)) & 3u) != 3u) rt_x87_note(c, X87_NOTE_STACK); /* overflow */
    c->st[c->ftop] = v;
    c->ftag &= ~(3u << (2u * c->ftop));
}
static inline void fpop(Cpu *c) {
    c->ftag |= (3u << (2u * c->ftop));
    c->ftop = (c->ftop + 1u) & 7u;
}
/* Round an arithmetic result according to precision control. */
static inline double fprnd(Cpu *c, double v) {
    uint32_t pc = (c->fcw >> 8) & 3u, rc = (c->fcw >> 10) & 3u;
    if (rc != 0) rt_x87_note(c, X87_NOTE_RC);
    if (pc == 2u) return v;
    if (pc == 0u) { rt_x87_note(c, X87_NOTE_PC24); return (double)(float)v; }
    rt_x87_note(c, X87_NOTE_PC64);
    return v;
}
static inline float f32_from_bits(uint32_t b) { float f; memcpy(&f, &b, 4); return f; }
static inline uint32_t f32_bits(float f) { uint32_t b; memcpy(&b, &f, 4); return b; }
static inline double f64_from_bits(uint64_t b) { double d; memcpy(&d, &b, 8); return d; }
static inline uint64_t f64_bits(double d) { uint64_t b; memcpy(&b, &d, 8); return b; }

static inline double fld_m32(uint32_t a) { return (double)f32_from_bits(rd32(a)); }
static inline double fld_m64(uint32_t a) { return f64_from_bits(rd64(a)); }
static inline void fst_m32(Cpu *c, uint32_t a, double v) { (void)c; wr32(a, f32_bits((float)v)); }
static inline void fst_m64(uint32_t a, double v) { wr64(a, f64_bits(v)); }
double rt_f80_load(Cpu *c, uint32_t a);
void rt_f80_store(Cpu *c, uint32_t a, double v);

/* Round to integer under the x87 rounding control (exact, independent of C fenv). */
static inline double frnd_rc(Cpu *c, double v) {
    uint32_t rc = (c->fcw >> 10) & 3u;
    if (v != v || v == INFINITY || v == -INFINITY) return v;
    double fl = floor(v);
    switch (rc) {
    case 0: { double d = v - fl; if (d > 0.5) return fl + 1.0; if (d < 0.5) return fl;
              return (fmod(fl, 2.0) == 0.0) ? fl : fl + 1.0; }
    case 1: return fl;
    case 2: return ceil(v);
    default: return (v < 0) ? ceil(v) : fl;
    }
}
static inline int64_t fist_value(Cpu *c, double v, int bits) {
    double r = frnd_rc(c, v);
    double lim = (bits == 16) ? 32768.0 : (bits == 32) ? 2147483648.0 : 9223372036854775808.0;
    if (r != r || r >= lim || r < -lim) {
        return (bits == 16) ? -32768 : (bits == 32) ? (int64_t)INT32_MIN : INT64_MIN;
    }
    return (int64_t)r;
}
static inline void fcom_set(Cpu *c, double a, double b) {
    uint32_t sw = c->fsw & ~(0x4500u);
    if (a != a || b != b) sw |= 0x4500u;
    else if (a < b) sw |= 0x0100u;
    else if (a == b) sw |= 0x4000u;
    c->fsw = sw;
}
static inline uint32_t fnstsw(Cpu *c) { return ((c->fsw & ~0x3800u) | ((c->ftop & 7u) << 11)) & 0xffffu; }
static inline void fninit(Cpu *c) { c->fcw = 0x037fu; c->fsw = 0; c->ftag = 0xffffu; c->ftop = 0; }

/* SAHF: SF ZF AF PF CF from AH. */
static inline void op_sahf(Cpu *c) {
    uint32_t ah = (c->eax >> 8) & 0xffu;
    flags_set_explicit(c, ah & (F_SF | F_ZF | F_AF | F_PF | F_CF));
}
static inline void op_lahf(Cpu *c) {
    uint32_t f = flags_get(c) & (F_SF | F_ZF | F_AF | F_PF | F_CF);
    c->eax = set8hi(c->eax, f | 0x02u);
}
/* PUSHFD/POPFD: the arithmetic flags and DF, with IF set and bit 1 (always 1).
 * The ID bit (21) never changes, as on a CPU without CPUID: the Visual C++ 2003
 * runtime's SSE2 check (Bugatron Worlds) then skips CPUID and takes its x87 paths. */
#define F_DF 0x400u
#define F_IF 0x200u
static inline void op_pushfd(Cpu *c) {
    uint32_t f = flags_get(c) & (F_OF | F_SF | F_ZF | F_AF | F_PF | F_CF);
    push32(c, f | (c->df ? F_DF : 0u) | F_IF | 0x02u);
}
static inline void op_popfd(Cpu *c) {
    uint32_t v = pop32(c);
    flags_set_explicit(c, v & (F_OF | F_SF | F_ZF | F_AF | F_PF | F_CF));
    c->df = (v & F_DF) != 0;
}

#endif /* BUGATRON_RECOMP_H */
