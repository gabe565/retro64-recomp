/* The flag readers' fast paths (recomp.h) against flags_get: over random states
 * made by the real instruction helpers, every condition must match the one
 * flags_get's full flag set gives. Returns the number of mismatches. */
#include "recomp.h"

static uint64_t next(uint64_t *s) {
    *s ^= *s << 13; *s ^= *s >> 7; *s ^= *s << 17;
    return *s;
}

uint32_t flags_selfcheck(uint64_t seed, uint32_t n) {
    static const uint32_t sizes[3] = { 8u, 16u, 32u };
    Cpu cpu = { 0 };
    Cpu *c = &cpu;
    uint32_t bad = 0;
    for (uint32_t i = 0; i < n; i++) {
        uint64_t r = next(&seed);
        uint32_t sz = sizes[r % 3u];
        /* Operands with garbage above the operand size, and edge values. */
        uint32_t a = (uint32_t)next(&seed), b = (uint32_t)next(&seed);
        if ((r >> 8) % 4u == 0) a = szmsb(sz) - (uint32_t)((r >> 12) % 3u);
        if ((r >> 16) % 4u == 0) b = (uint32_t)((r >> 20) % 3u);
        flags_set_explicit(c, (uint32_t)(r >> 32) & (F_CF | F_PF | F_AF | F_ZF | F_SF | F_OF));
        switch ((r >> 24) % 6u) {
        case 0: op_sub(c, sz, a & szmask(sz), b & szmask(sz)); break;
        case 1: op_add(c, sz, a & szmask(sz), b & szmask(sz)); break;
        case 2: op_logic(c, sz, a); break;
        case 3: op_inc(c, sz, a & szmask(sz)); break;
        case 4: op_dec(c, sz, a & szmask(sz)); break;
        default: break; /* explicit */
        }
        uint32_t f = flags_get(c);
        int cf = (f & F_CF) != 0, of = (f & F_OF) != 0, sf = (f & F_SF) != 0, zf = (f & F_ZF) != 0, pf = (f & F_PF) != 0;
        int l = sf != of;
        bad += (get_cf(c) != 0) != cf;
        bad += (cc_b(c) != 0) != cf;
        bad += (cc_o(c) != 0) != of;
        bad += (cc_e(c) != 0) != zf;
        bad += (cc_s(c) != 0) != sf;
        bad += (cc_p(c) != 0) != pf;
        bad += (cc_be(c) != 0) != (cf || zf);
        bad += (cc_l(c) != 0) != l;
        bad += (cc_le(c) != 0) != (zf || l);
    }
    return bad;
}
