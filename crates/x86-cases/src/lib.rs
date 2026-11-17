//! x86 instruction-semantics regression cases.
//!
//! Each case is a small 32-bit snippet assembled with iced-x86. The same bytes
//! are (a) run through the static recompiler and the runtime (`bugatron-x86-tests`)
//! and (b) executed natively on a real x86 CPU (`bugatron-x86-oracle`).
//!
//! Conventions:
//! - On entry EBX points at a 256-byte scratch buffer (`mem` initialises it).
//! - The harness appends `ret`; snippets must leave ESP balanced.
//! - Results are compared on registers, the defined flags in `flag_mask`,
//!   the scratch buffer, and x87 status bits in `fsw_mask` plus TOP.
//! - Registers listed in `ptr_regs` hold scratch pointers on exit and are
//!   compared relative to the scratch base.
//! - `expect` holds hand-derived values from the Intel SDM. They are checked
//!   in addition to (not instead of) the native oracle's results.

// Expected values are exact bit patterns measured on hardware or derived from
// the x87 algorithm; they must not be replaced by library constants.
#![allow(clippy::approx_constant)]

pub use iced_x86_asm as iced;
use iced_x86_asm::code_asm::*;
use iced_x86_asm::IcedError;

pub const SCRATCH_SIZE: usize = 256;

pub const CF: u32 = 0x001;
pub const PF: u32 = 0x004;
pub const AF: u32 = 0x010;
pub const ZF: u32 = 0x040;
pub const SF: u32 = 0x080;
pub const OF: u32 = 0x800;
pub const OSZAPC: u32 = OF | SF | ZF | AF | PF | CF;
pub const SZPC: u32 = SF | ZF | PF | CF;

/// Register order used everywhere: eax ecx edx ebx esp ebp esi edi.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum R { Eax = 0, Ecx = 1, Edx = 2, Ebx = 3, Esp = 4, Ebp = 5, Esi = 6, Edi = 7 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Must match the native oracle exactly (on the compared fields).
    Strict,
    /// Compared and reported, but a mismatch is expected and documented.
    KnownDiscrepancy(&'static str),
    /// Measured only (e.g. transcendental ULP differences).
    ReportOnly,
}

#[derive(Clone)]
pub struct Expect {
    pub regs: &'static [(R, u32)],
    /// Expected flags (compared under `flag_mask`).
    pub flags: Option<u32>,
    pub mem: Vec<(usize, Vec<u8>)>,
}

pub const NO_EXPECT: Expect = Expect { regs: &[], flags: None, mem: Vec::new() };

#[derive(Clone)]
pub struct Case {
    pub name: &'static str,
    pub asm: fn(&mut CodeAssembler) -> Result<(), IcedError>,
    /// Initial eax..edi (ESP and EBX are set by the harness).
    pub regs: [u32; 8],
    /// Initial arithmetic flags (CF/PF/AF/ZF/SF/OF).
    pub eflags: u32,
    pub fcw: u16,
    pub mem: Vec<(usize, Vec<u8>)>,
    pub ptr_regs: &'static [R],
    pub flag_mask: u32,
    /// x87 status-word bits compared (C0=0x100, C1=0x200, C2=0x400, C3=0x4000).
    pub fsw_mask: u16,
    pub kind: Kind,
    pub expect: Expect,
}

/// Assemble a case at `base`, appending `ret`.
pub fn assemble(case: &Case, base: u32) -> Result<Vec<u8>, IcedError> {
    let mut a = CodeAssembler::new(32)?;
    (case.asm)(&mut a)?;
    a.ret()?;
    a.assemble(base as u64)
}

const Z: [u32; 8] = [0; 8];
const FCW: u16 = 0x027f;

fn regs(a: u32, c: u32, d: u32) -> [u32; 8] {
    [a, c, d, 0, 0, 0, 0, 0]
}

macro_rules! f64b { ($v:expr) => { ($v as f64).to_le_bytes() } }
macro_rules! mem { ($(($o:expr, $b:expr)),* $(,)?) => { vec![$(($o, AsRef::<[u8]>::as_ref(&$b).to_vec())),*] } }

pub fn cases() -> Vec<Case> {
    let base = Case {
        name: "",
        asm: |_| Ok(()),
        regs: Z,
        eflags: 0,
        fcw: FCW,
        mem: Vec::new(),
        ptr_regs: &[],
        flag_mask: OSZAPC,
        fsw_mask: 0,
        kind: Kind::Strict,
        expect: NO_EXPECT,
    };
    vec![
        // ---- add/adc/sub/sbb/inc/dec/neg flags ---------------------------------
        Case { name: "add_signed_overflow", regs: regs(0x7fff_ffff, 0, 0), asm: |a| a.add(eax, 1),
               expect: Expect { regs: &[(R::Eax, 0x8000_0000)], flags: Some(OF | SF | AF | PF), mem: mem![] }, ..base.clone() },
        Case { name: "add_carry_to_zero", regs: regs(0xffff_ffff, 0, 0), asm: |a| a.add(eax, 1),
               expect: Expect { regs: &[(R::Eax, 0)], flags: Some(CF | ZF | AF | PF), mem: mem![] }, ..base.clone() },
        Case { name: "adc_with_carry_in", regs: regs(0xffff_ffff, 0, 0), eflags: CF, asm: |a| a.adc(eax, 0),
               expect: Expect { regs: &[(R::Eax, 0)], flags: Some(CF | ZF | AF | PF), mem: mem![] }, ..base.clone() },
        Case { name: "sub_borrow", asm: |a| a.sub(eax, 1),
               expect: Expect { regs: &[(R::Eax, 0xffff_ffff)], flags: Some(CF | SF | AF | PF), mem: mem![] }, ..base.clone() },
        Case { name: "sbb_with_borrow_in", eflags: CF, asm: |a| a.sbb(eax, 0),
               expect: Expect { regs: &[(R::Eax, 0xffff_ffff)], flags: Some(CF | SF | AF | PF), mem: mem![] }, ..base.clone() },
        Case { name: "neg_sbb_boolean_idiom", regs: regs(5, 0, 0), asm: |a| { a.neg(eax)?; a.sbb(eax, eax) },
               expect: Expect { regs: &[(R::Eax, 0xffff_ffff)], flags: Some(CF | SF | AF | PF), mem: mem![] }, ..base.clone() },
        Case { name: "inc_preserves_cf", regs: regs(0x7fff_ffff, 0, 0), eflags: CF, asm: |a| a.inc(eax),
               expect: Expect { regs: &[(R::Eax, 0x8000_0000)], flags: Some(CF | OF | SF | AF | PF), mem: mem![] }, ..base.clone() },
        Case { name: "dec_to_zero", regs: regs(1, 0, 0), asm: |a| a.dec(eax),
               expect: Expect { regs: &[(R::Eax, 0)], flags: Some(ZF | PF), mem: mem![] }, ..base.clone() },
        Case { name: "neg_zero", eflags: CF, asm: |a| a.neg(eax),
               expect: Expect { regs: &[(R::Eax, 0)], flags: Some(ZF | PF), mem: mem![] }, ..base.clone() },
        Case { name: "cmp_8bit_sign", regs: regs(0x0000_007f, 0x0000_0080, 0), asm: |a| a.cmp(al, cl),
               expect: Expect { regs: &[], flags: Some(CF | OF | SF | PF), mem: mem![] }, ..base.clone() },
        // ---- shifts and rotates ---------------------------------------------------
        Case { name: "shl_1_cf_of", regs: regs(0xc000_0000, 0, 0), asm: |a| a.shl(eax, 1), flag_mask: OF | SZPC,
               expect: Expect { regs: &[(R::Eax, 0x8000_0000)], flags: Some(CF | SF | PF), mem: mem![] }, ..base.clone() },
        Case { name: "shr_4", regs: regs(0x8000_0008, 0, 0), asm: |a| a.shr(eax, 4), flag_mask: SZPC,
               expect: Expect { regs: &[(R::Eax, 0x0800_0000)], flags: Some(CF | PF), mem: mem![] }, ..base.clone() },
        Case { name: "sar_31", regs: regs(0x8000_0000, 0, 0), asm: |a| a.sar(eax, 31), flag_mask: SZPC,
               expect: Expect { regs: &[(R::Eax, 0xffff_ffff)], flags: Some(SF | PF), mem: mem![] }, ..base.clone() },
        Case { name: "shl_count0_keeps_flags", regs: regs(5, 0, 0), eflags: CF | ZF, asm: |a| a.shl(eax, cl),
               expect: Expect { regs: &[(R::Eax, 5)], flags: Some(CF | ZF), mem: mem![] }, ..base.clone() },
        Case { name: "shl_count32_masked", regs: regs(5, 32, 0), eflags: CF, asm: |a| a.shl(eax, cl),
               expect: Expect { regs: &[(R::Eax, 5)], flags: Some(CF), mem: mem![] }, ..base.clone() },
        Case { name: "rol_1", regs: regs(0x8000_0001, 0, 0), asm: |a| a.rol(eax, 1), flag_mask: OF | CF,
               expect: Expect { regs: &[(R::Eax, 3)], flags: Some(CF | OF), mem: mem![] }, ..base.clone() },
        Case { name: "ror_4", regs: regs(0x0000_000f, 0, 0), asm: |a| a.ror(eax, 4), flag_mask: CF,
               expect: Expect { regs: &[(R::Eax, 0xf000_0000)], flags: Some(CF), mem: mem![] }, ..base.clone() },
        Case { name: "rcr_1_carry_in", regs: regs(2, 0, 0), eflags: CF, asm: |a| a.rcr(eax, 1), flag_mask: OF | CF,
               expect: Expect { regs: &[(R::Eax, 0x8000_0001)], flags: Some(OF), mem: mem![] }, ..base.clone() },
        Case { name: "rcr_64bit_shift_idiom", regs: regs(0, 0, 1), asm: |a| { a.shr(edx, 1)?; a.rcr(eax, 1) }, flag_mask: CF,
               expect: Expect { regs: &[(R::Eax, 0x8000_0000), (R::Edx, 0)], flags: Some(0), mem: mem![] }, ..base.clone() },
        Case { name: "rcr_ebx_eax_crt_aulldiv_form", regs: [0x0000_0003, 0, 0, 0, 0, 0, 0, 0], asm: |a| { a.mov(ebx, 0x0000_0005u32)?; a.shr(ebx, 1)?; a.rcr(eax, 1) },
               ptr_regs: &[], flag_mask: CF,
               expect: Expect { regs: &[(R::Eax, 0x8000_0001), (R::Ebx, 2)], flags: Some(CF), mem: mem![] }, ..base.clone() },
        Case { name: "rcl_8bit", regs: regs(0x81, 0, 0), eflags: CF, asm: |a| a.rcl(al, 1), flag_mask: OF | CF,
               expect: Expect { regs: &[(R::Eax, 0x03)], flags: Some(CF | OF), mem: mem![] }, ..base.clone() },
        Case { name: "rcr_8bit_count9_is_identity", regs: regs(0x01, 0, 0), eflags: CF, asm: |a| a.rcr(al, 9), flag_mask: CF,
               expect: Expect { regs: &[(R::Eax, 0x01)], flags: Some(CF), mem: mem![] }, ..base.clone() },
        Case { name: "rcl_16bit_count3", regs: regs(0x8001, 0, 0), asm: |a| a.rcl(ax, 3), flag_mask: CF,
               expect: Expect { regs: &[(R::Eax, 0x000a)], flags: Some(0), mem: mem![] }, ..base.clone() },
        Case { name: "shld_4", regs: regs(0x1234_5678, 0, 0x9abc_def0), asm: |a| a.shld(eax, edx, 4), flag_mask: SZPC,
               expect: Expect { regs: &[(R::Eax, 0x2345_6789)], flags: Some(CF), mem: mem![] }, ..base.clone() },
        Case { name: "shrd_8", regs: regs(0x1234_5678, 0, 0x0000_00ab), asm: |a| a.shrd(eax, edx, 8), flag_mask: SZPC,
               expect: Expect { regs: &[(R::Eax, 0xab12_3456)], flags: Some(SF | PF), mem: mem![] }, ..base.clone() },
        // ---- bit tests --------------------------------------------------------------
        Case { name: "bts_reg_twice", regs: regs(0, 5, 0), asm: |a| { a.bts(eax, ecx)?; a.mov(edx, 0u32)?; a.bts(eax, ecx)?; a.adc(edx, 0) }, flag_mask: CF,
               expect: Expect { regs: &[(R::Eax, 0x20), (R::Edx, 1)], flags: Some(0), mem: mem![] }, ..base.clone() },
        Case { name: "bts_mem_bitstring_positive", regs: regs(40, 0, 0), asm: |a| a.bts(dword_ptr(ebx), eax), flag_mask: CF,
               expect: Expect { regs: &[], flags: Some(0), mem: mem![(5, &[0x01])] }, ..base.clone() },
        Case { name: "bts_mem_bitstring_negative", regs: regs(0xffff_ffff, 0, 0), asm: |a| a.bts(dword_ptr(ebx + 8), eax), flag_mask: CF,
               expect: Expect { regs: &[], flags: Some(0), mem: mem![(7, &[0x80])] }, ..base.clone() },
        Case { name: "bts_esp_strcspn_form", regs: regs(0x41, 0, 0), asm: |a| {
                    a.sub(esp, 32)?; a.mov(ecx, 8u32)?;
                    a.mov(edx, esp)?; a.mov(dword_ptr(edx), 0u32)?; a.mov(dword_ptr(edx + 4), 0u32)?; a.mov(dword_ptr(edx + 8), 0u32)?;
                    a.mov(dword_ptr(edx + 12), 0u32)?;
                    a.bts(dword_ptr(esp), eax)?; a.mov(ecx, dword_ptr(esp + 8))?; a.mov(dword_ptr(ebx), ecx)?; a.add(esp, 32)?; a.xor(edx, edx) },
               flag_mask: 0, ptr_regs: &[],
               expect: Expect { regs: &[], flags: None, mem: mem![(0, &[0x02, 0, 0, 0])] }, ..base.clone() },
        Case { name: "btr_btc_bt_imm", regs: regs(0xffff_0000, 0, 0), asm: |a| { a.btr(eax, 16)?; a.btc(eax, 0)?; a.bt(eax, 31) }, flag_mask: CF,
               expect: Expect { regs: &[(R::Eax, 0xfffe_0001)], flags: Some(CF), mem: mem![] }, ..base.clone() },
        // ---- pushad/popad --------------------------------------------------------
        Case { name: "pushad_popad", regs: [1, 2, 3, 0, 0, 6, 7, 8], asm: |a| {
                    a.pushad()?; a.mov(edx, dword_ptr(esp + 12))?; a.sub(edx, esp)?; a.mov(dword_ptr(ebx), edx)?;
                    a.mov(eax, 0u32)?; a.mov(ecx, 0u32)?; a.mov(esi, 0u32)?; a.popad() },
               flag_mask: 0,
               expect: Expect { regs: &[(R::Eax, 1), (R::Ecx, 2), (R::Edx, 3), (R::Ebp, 6), (R::Esi, 7), (R::Edi, 8)], flags: None, mem: mem![(0, &[32, 0, 0, 0])] }, ..base.clone() },
        // ---- multiply / divide -----------------------------------------------------
        Case { name: "imul2_overflow", regs: regs(0x10000, 0, 0), asm: |a| a.imul_2(eax, eax), flag_mask: OF | CF,
               expect: Expect { regs: &[(R::Eax, 0)], flags: Some(OF | CF), mem: mem![] }, ..base.clone() },
        Case { name: "imul1_signed", regs: regs(0xffff_fffe, 3, 0), asm: |a| a.imul(ecx), flag_mask: OF | CF,
               expect: Expect { regs: &[(R::Eax, 0xffff_fffa), (R::Edx, 0xffff_ffff)], flags: Some(0), mem: mem![] }, ..base.clone() },
        Case { name: "mul1_unsigned", regs: regs(0xffff_ffff, 2, 0), asm: |a| a.mul(ecx), flag_mask: OF | CF,
               expect: Expect { regs: &[(R::Eax, 0xffff_fffe), (R::Edx, 1)], flags: Some(OF | CF), mem: mem![] }, ..base.clone() },
        Case { name: "div_unsigned_64", regs: regs(0, 2, 1), asm: |a| a.div(ecx), flag_mask: 0,
               expect: Expect { regs: &[(R::Eax, 0x8000_0000), (R::Edx, 0)], flags: None, mem: mem![] }, ..base.clone() },
        Case { name: "idiv_negative_truncates", regs: regs(0xffff_fff9, 2, 0xffff_ffff), asm: |a| a.idiv(ecx), flag_mask: 0,
               expect: Expect { regs: &[(R::Eax, 0xffff_fffd), (R::Edx, 0xffff_ffff)], flags: None, mem: mem![] }, ..base.clone() },
        Case { name: "cdq_movsx_movzx", regs: regs(0x8000_0000, 0, 0), mem: mem![(0, &[0xf0, 0x80])], asm: |a| {
                    a.cdq()?; a.movsx(ecx, byte_ptr(ebx))?; a.movzx(esi, word_ptr(ebx)) },
               flag_mask: 0,
               expect: Expect { regs: &[(R::Edx, 0xffff_ffff), (R::Ecx, 0xffff_fff0), (R::Esi, 0x80f0)], flags: None, mem: mem![] }, ..base.clone() },
        Case { name: "setcc_after_cmp", regs: regs(1, 2, 0), asm: |a| {
                    a.cmp(eax, ecx)?; a.setl(byte_ptr(ebx))?; a.setb(byte_ptr(ebx + 1))?; a.setg(byte_ptr(ebx + 2))?; a.setne(byte_ptr(ebx + 3))?;
                    a.mov(eax, 0xffff_ffffu32)?; a.cmp(eax, 1)?; a.setl(byte_ptr(ebx + 4))?; a.seta(byte_ptr(ebx + 5)) },
               expect: Expect { regs: &[], flags: None, mem: mem![(0, &[1, 1, 0, 1, 1, 1])] }, ..base.clone() },
        // ---- string instructions --------------------------------------------------
        Case { name: "rep_movsb", mem: mem![(0, b"hello")], asm: |a| {
                    a.lea(esi, dword_ptr(ebx))?; a.lea(edi, dword_ptr(ebx + 16))?; a.mov(ecx, 5u32)?; a.rep().movsb() },
               ptr_regs: &[R::Esi, R::Edi], flag_mask: 0,
               expect: Expect { regs: &[(R::Ecx, 0), (R::Esi, 5), (R::Edi, 21)], flags: None, mem: mem![(16, b"hello")] }, ..base.clone() },
        Case { name: "repne_scasb_strlen", mem: mem![(0, b"abcdef\0")], asm: |a| {
                    a.lea(edi, dword_ptr(ebx))?; a.mov(ecx, 0xffff_ffffu32)?; a.xor(eax, eax)?; a.repne().scasb()?; a.not(ecx)?; a.dec(ecx) },
               ptr_regs: &[R::Edi], flag_mask: 0,
               expect: Expect { regs: &[(R::Ecx, 6), (R::Edi, 7)], flags: None, mem: mem![] }, ..base.clone() },
        Case { name: "rep_stosd", regs: regs(0xaabb_ccdd, 0, 0), asm: |a| { a.lea(edi, dword_ptr(ebx + 4))?; a.mov(ecx, 2u32)?; a.rep().stosd() },
               ptr_regs: &[R::Edi], flag_mask: 0,
               expect: Expect { regs: &[(R::Edi, 12)], flags: None, mem: mem![(4, &[0xdd, 0xcc, 0xbb, 0xaa, 0xdd, 0xcc, 0xbb, 0xaa])] }, ..base.clone() },
        Case { name: "repe_cmpsb_mismatch", mem: mem![(0, b"abcX"), (16, b"abcY")], asm: |a| {
                    a.lea(esi, dword_ptr(ebx))?; a.lea(edi, dword_ptr(ebx + 16))?; a.mov(ecx, 4u32)?; a.repe().cmpsb() },
               ptr_regs: &[R::Esi, R::Edi], flag_mask: SZPC,
               expect: Expect { regs: &[(R::Ecx, 0), (R::Esi, 4), (R::Edi, 20)], flags: Some(CF | SF | PF), mem: mem![] }, ..base.clone() },
        Case { name: "std_movsd_backwards", mem: mem![(8, &[1, 2, 3, 4])], asm: |a| {
                    a.std()?; a.lea(esi, dword_ptr(ebx + 8))?; a.lea(edi, dword_ptr(ebx + 32))?; a.movsd()?; a.cld() },
               ptr_regs: &[R::Esi, R::Edi], flag_mask: 0,
               expect: Expect { regs: &[(R::Esi, 4), (R::Edi, 28)], flags: None, mem: mem![(32, &[1, 2, 3, 4])] }, ..base.clone() },
        // ---- x87 (control word 0x027F unless stated) -----------------------------------
        Case { name: "x87_div_third_pc53", asm: |a| { a.fld1()?; a.fld(qword_ptr(ebx))?; a.fdivp(st1, st0)?; a.fstp(qword_ptr(ebx + 8)) },
               mem: mem![(0, f64b!(3.0))], flag_mask: 0,
               expect: Expect { regs: &[], flags: None, mem: mem![(8, f64b!(1.0 / 3.0))] }, ..base.clone() },
        Case { name: "x87_fistp_round_nearest_even", mem: mem![(0, f64b!(2.5)), (8, f64b!(3.5)), (16, f64b!(-2.5))], asm: |a| {
                    a.fld(qword_ptr(ebx))?; a.fistp(dword_ptr(ebx + 32))?; a.fld(qword_ptr(ebx + 8))?; a.fistp(dword_ptr(ebx + 36))?;
                    a.fld(qword_ptr(ebx + 16))?; a.fistp(dword_ptr(ebx + 40)) },
               flag_mask: 0,
               expect: Expect { regs: &[], flags: None, mem: mem![(32, &[2, 0, 0, 0, 4, 0, 0, 0, 0xfe, 0xff, 0xff, 0xff])] }, ..base.clone() },
        Case { name: "x87_ftol_truncate_rc3", mem: mem![(0, f64b!(2.7)), (8, f64b!(-2.7)), (16, &[0x7f, 0x0e])], asm: |a| {
                    a.fnstcw(word_ptr(ebx + 24))?; a.fldcw(word_ptr(ebx + 16))?;
                    a.fld(qword_ptr(ebx))?; a.fistp(qword_ptr(ebx + 32))?; a.fld(qword_ptr(ebx + 8))?; a.fistp(qword_ptr(ebx + 40))?;
                    a.fldcw(word_ptr(ebx + 24)) },
               flag_mask: 0,
               expect: Expect { regs: &[], flags: None, mem: mem![(24, &[0x7f, 0x02]), (32, &[2, 0, 0, 0, 0, 0, 0, 0, 0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff])] }, ..base.clone() },
        Case { name: "x87_fcom_fnstsw_sahf", mem: mem![(0, f64b!(1.0)), (8, f64b!(2.0))], asm: |a| {
                    a.fld(qword_ptr(ebx + 8))?; a.fld(qword_ptr(ebx))?; a.fcompp()?; a.fnstsw(ax)?; a.sahf()?; a.setb(byte_ptr(ebx + 16))?; a.sete(byte_ptr(ebx + 17)) },
               flag_mask: CF | ZF, fsw_mask: 0x4500, ptr_regs: &[],
               expect: Expect { regs: &[], flags: Some(CF), mem: mem![(16, &[1, 0])] }, ..base.clone() },
        Case { name: "x87_fxch_fchs_fabs_fsqrt", mem: mem![(0, f64b!(2.0)), (8, f64b!(-9.0))], asm: |a| {
                    a.fld(qword_ptr(ebx))?; a.fld(qword_ptr(ebx + 8))?; a.fxch(st0, st1)?; a.fsqrt()?; a.fstp(qword_ptr(ebx + 16))?;
                    a.fabs()?; a.fchs()?; a.fstp(qword_ptr(ebx + 24)) },
               flag_mask: 0,
               expect: Expect { regs: &[], flags: None, mem: mem![(16, f64b!(std::f64::consts::SQRT_2)), (24, f64b!(-9.0))] }, ..base.clone() },
        Case { name: "x87_fist_overflow_indefinite", mem: mem![(0, f64b!(3.0e10))], asm: |a| { a.fld(qword_ptr(ebx))?; a.fistp(dword_ptr(ebx + 8)) },
               flag_mask: 0,
               expect: Expect { regs: &[], flags: None, mem: mem![(8, &[0, 0, 0, 0x80])] }, ..base.clone() },
        Case { name: "x87_frndint_rc_modes", mem: mem![(0, f64b!(-1.5)), (8, &[0x7f, 0x06])], asm: |a| {
                    a.fld(qword_ptr(ebx))?; a.frndint()?; a.fstp(qword_ptr(ebx + 16))?;
                    a.fnstcw(word_ptr(ebx + 10))?; a.fldcw(word_ptr(ebx + 8))?;
                    a.fld(qword_ptr(ebx))?; a.frndint()?; a.fstp(qword_ptr(ebx + 24))?; a.fldcw(word_ptr(ebx + 10)) },
               flag_mask: 0,
               expect: Expect { regs: &[], flags: None, mem: mem![(16, f64b!(-2.0)), (24, f64b!(-2.0))] }, ..base.clone() },
        Case { name: "x87_f80_store_load", asm: |a| { a.fld1()?; a.fstp(tword_ptr(ebx))?; a.fld(tword_ptr(ebx))?; a.fstp(qword_ptr(ebx + 16)) },
               flag_mask: 0,
               expect: Expect { regs: &[], flags: None, mem: mem![(0, &[0, 0, 0, 0, 0, 0, 0, 0x80, 0xff, 0x3f]), (16, f64b!(1.0))] }, ..base.clone() },
        Case { name: "x87_fild_fimul_fstp_m32", mem: mem![(0, &[7, 0, 0, 0]), (4, &[3, 0])], asm: |a| {
                    a.fild(dword_ptr(ebx))?; a.fimul(word_ptr(ebx + 4))?; a.fstp(dword_ptr(ebx + 8)) },
               flag_mask: 0,
               expect: Expect { regs: &[], flags: None, mem: mem![(8, 21f32.to_le_bytes())] }, ..base.clone() },
        Case { name: "x87_pc24_division", mem: mem![(0, f64b!(3.0)), (8, &[0x7f, 0x00])], asm: |a| {
                    a.fnstcw(word_ptr(ebx + 10))?; a.fldcw(word_ptr(ebx + 8))?;
                    a.fld1()?; a.fdiv(qword_ptr(ebx))?; a.fstp(qword_ptr(ebx + 16))?; a.fldcw(word_ptr(ebx + 10)) },
               flag_mask: 0, kind: Kind::Strict,
               expect: Expect { regs: &[], flags: None, mem: mem![(16, f64b!((1.0f32 / 3.0f32) as f64))] }, ..base.clone() },
        Case { name: "x87_pc64_extended_sum", mem: mem![(0, 9007199254740992i64.to_le_bytes()), (8, &[0x7f, 0x03])], asm: |a| {
                    a.fnstcw(word_ptr(ebx + 10))?; a.fldcw(word_ptr(ebx + 8))?;
                    a.fild(qword_ptr(ebx))?; a.fld1()?; a.faddp(st1, st0)?; a.fistp(qword_ptr(ebx + 16))?; a.fldcw(word_ptr(ebx + 10)) },
               flag_mask: 0, kind: Kind::KnownDiscrepancy("64-bit precision control needs 80-bit arithmetic; the model computes in binary64 and raises X87_NOTE_PC64"),
               expect: Expect { regs: &[], flags: None, mem: mem![(16, 9007199254740993i64.to_le_bytes())] }, ..base.clone() },
        Case { name: "x87_transcendentals", mem: mem![(0, f64b!(1.0)), (8, f64b!(0.5))], asm: |a| {
                    a.fld(qword_ptr(ebx))?; a.fsin()?; a.fstp(qword_ptr(ebx + 16))?;
                    a.fld(qword_ptr(ebx))?; a.fcos()?; a.fstp(qword_ptr(ebx + 24))?;
                    a.fld(qword_ptr(ebx))?; a.fld(qword_ptr(ebx + 8))?; a.fpatan()?; a.fstp(qword_ptr(ebx + 32))?;
                    a.fld(qword_ptr(ebx + 8))?; a.fptan()?; a.fstp(qword_ptr(ebx + 40))?; a.fstp(qword_ptr(ebx + 48)) },
               // Hardware values (AMD Ryzen 7 9800X3D).
               flag_mask: 0, kind: Kind::Strict,
               expect: Expect { regs: &[], flags: None, mem: mem![(16, f64b!(0.8414709848078965)), (24, f64b!(0.5403023058681398)),
                   (32, f64b!(1.1071487177940904f64)), (40, f64b!(1.0)), (48, f64b!(0.5463024898437905))] }, ..base.clone() },
        Case { name: "x87_transcendental_sweep",
               mem: mem![(0, f64b!(0.1)), (8, f64b!(std::f64::consts::FRAC_PI_4)), (16, f64b!(std::f64::consts::FRAC_PI_2)), (24, f64b!(2.0)),
                         (32, f64b!(std::f64::consts::PI)), (40, f64b!(4.0)), (48, f64b!(5.5)), (56, f64b!(std::f64::consts::TAU))],
               asm: |a| {
                    for i in 0..8 {
                        a.fld(qword_ptr(ebx + 8 * i))?; a.fsin()?; a.fstp(qword_ptr(ebx + 64 + 8 * i))?;
                        a.fld(qword_ptr(ebx + 8 * i))?; a.fcos()?; a.fstp(qword_ptr(ebx + 128 + 8 * i))?;
                    }
                    Ok(())
               },
               // Hardware values (AMD Ryzen 7 9800X3D).
               flag_mask: 0, kind: Kind::Strict,
               expect: Expect { regs: &[], flags: None, mem: mem![
                   (64, f64b!(0.09983341664682815)), (72, f64b!(0.7071067811865475)), (80, f64b!(1.0)), (88, f64b!(0.9092974268256817)),
                   (96, f64b!(1.2246063538223773e-16)), (104, f64b!(-0.7568024953079282)), (112, f64b!(-0.7055403255703919)), (120, f64b!(-2.4492127076447545e-16)),
                   (128, f64b!(0.9950041652780258)), (136, f64b!(0.7071067811865476)), (144, f64b!(6.123031769111886e-17)), (152, f64b!(-0.4161468365471424)),
                   (160, f64b!(-1.0)), (168, f64b!(-0.6536436208636119)), (176, f64b!(0.70866977429126)), (184, f64b!(1.0))] }, ..base.clone() },
        // Half-angles fed to fsin/fcos by r3d 0x421680, from real game angles (float32; VM capture 2026-11-15): ±π/2, π, 2π and sampled values. Expected values: x87 reduction with the 66-bit π (prototype); confirmed on hardware by the oracle fixture.
        Case { name: "x87_trig_workload_angles", mem: mem![(0, f64b!(0.7853981852531433f64)), (8, f64b!(-0.7853981852531433f64)), (16, f64b!(1.5707963705062866f64)), (24, f64b!(3.1415927410125732f64)), (32, f64b!(1.874469518661499f64)), (40, f64b!(1.029619574546814f64)), (48, f64b!(0.05551586300134659f64)), (56, f64b!(1.5243351025873153e-10f64))],
               asm: |a| {
                    for i in 0..8 {
                        a.fld(qword_ptr(ebx + 8 * i))?; a.fsin()?; a.fstp(qword_ptr(ebx + 64 + 8 * i))?;
                        a.fld(qword_ptr(ebx + 8 * i))?; a.fcos()?; a.fstp(qword_ptr(ebx + 128 + 8 * i))?;
                    }
                    Ok(())
               },
               flag_mask: 0, kind: Kind::Strict,
               expect: Expect { regs: &[], flags: None, mem: mem![(64, f64b!(0.7071067966408575f64)), (72, f64b!(-0.7071067966408575f64)), (80, f64b!(0.999999999999999f64)), (88, f64b!(-8.74227800037288e-08f64)), (96, f64b!(0.9542445442926789f64)), (104, f64b!(0.8571030769652557f64)), (112, f64b!(0.05548735064498265f64)), (120, f64b!(1.5243351025873153e-10f64)), (128, f64b!(0.7071067657322372f64)), (136, f64b!(0.7071067657322372f64)), (144, f64b!(-4.371139000186444e-08f64)), (152, f64b!(-0.9999999999999962f64)), (160, f64b!(-0.2990273393649107f64)), (168, f64b!(0.5151449460653681f64)), (176, f64b!(0.9984593902199532f64)), (184, f64b!(1.0f64))] }, ..base.clone() },
        // Negative angles and larger multiples of π/2 (the reduction error grows with k).
        Case { name: "x87_trig_negative_and_multiples", mem: mem![(0, f64b!(-std::f64::consts::PI)), (8, f64b!(-std::f64::consts::FRAC_PI_2)), (16, f64b!(-std::f64::consts::TAU)), (24, f64b!(3.0 * std::f64::consts::FRAC_PI_2)), (32, f64b!(100.0 * std::f64::consts::PI)), (40, f64b!(-1000.5 * std::f64::consts::PI)), (48, f64b!(-1000000.0f64)), (56, f64b!(1000000000000000.0f64))],
               asm: |a| {
                    for i in 0..8 {
                        a.fld(qword_ptr(ebx + 8 * i))?; a.fsin()?; a.fstp(qword_ptr(ebx + 64 + 8 * i))?;
                        a.fld(qword_ptr(ebx + 8 * i))?; a.fcos()?; a.fstp(qword_ptr(ebx + 128 + 8 * i))?;
                    }
                    Ok(())
               },
               flag_mask: 0, kind: Kind::Strict,
               expect: Expect { regs: &[], flags: None, mem: mem![(64, f64b!(-1.2246063538223773e-16f64)), (72, f64b!(-1.0f64)), (80, f64b!(2.4492127076447545e-16f64)), (88, f64b!(-1.0f64)), (96, f64b!(1.964791176978231e-15f64)), (104, f64b!(-1.0f64)), (112, f64b!(0.34999350217129177f64)), (120, f64b!(0.8582721324763734f64)), (128, f64b!(-1.0f64)), (136, f64b!(6.123031769111886e-17f64)), (144, f64b!(1.0f64)), (152, f64b!(-1.836909530733566e-16f64)), (160, f64b!(1.0f64)), (168, f64b!(1.562726456485336e-13f64)), (176, f64b!(0.9367521275331452f64)), (184, f64b!(-0.5131948427395374f64))] }, ..base.clone() },
        // Boundaries: signed zero, tiny, largest in-range, out of range (|x| >= 2^63:
        // C2=1, operand unchanged), +inf (invalid -> indefinite QNaN, IE), quiet NaN.
        // Status words after each FSIN are stored at 128.. for the oracle comparison.
        Case { name: "x87_trig_boundaries",
               mem: mem![(0, f64b!(0.0)), (8, f64b!(-0.0f64)), (16, f64b!(1.0e-20)), (24, f64b!(9223372036854774784.0)),
                         (32, f64b!(9223372036854775808.0)), (40, f64b!(-9223372036854775808.0)), (48, f64b!(f64::INFINITY)),
                         (56, 0x7ff8_0000_0000_0001u64.to_le_bytes())],
               asm: |a| {
                    for i in 0..8 {
                        a.fld(qword_ptr(ebx + 8 * i))?; a.fsin()?; a.fnstsw(ax)?; a.mov(word_ptr(ebx + 128 + 2 * i), ax)?;
                        a.fstp(qword_ptr(ebx + 64 + 8 * i))?;
                    }
                    a.fldz()?; a.fcos()?; a.fstp(qword_ptr(ebx + 144))?;
                    a.fld(qword_ptr(ebx + 32))?; a.fcos()?; a.fnstsw(ax)?; a.mov(word_ptr(ebx + 152), ax)?; a.fstp(qword_ptr(ebx + 160))?;
                    a.xor(eax, eax)
               },
               flag_mask: 0, fsw_mask: 0x4700, kind: Kind::Strict,
               expect: Expect { regs: &[], flags: None, mem: mem![(64, f64b!(0.0)), (72, f64b!(-0.0f64)), (80, f64b!(1.0e-20)),
                   (88, f64b!(0.9873418913143515)), (96, f64b!(9223372036854775808.0)), (104, f64b!(-9223372036854775808.0)),
                   (112, 0xfff8_0000_0000_0000u64.to_le_bytes()), (120, 0x7ff8_0000_0000_0001u64.to_le_bytes()),
                   (144, f64b!(1.0)), (160, f64b!(9223372036854775808.0))] }, ..base.clone() },
        // FPATAN (ST(1)=y, ST(0)=x): quadrants, results next to ±π and π/2 (where the
        // quadrant terms meet the x87's internal π), and table entries. Each result is
        // stored with its C1 (the 64-bit rounding direction); C0/C2/C3 are undefined.
        // Expected values are the model's (x87trig::x87_fpatan); oracle runs confirm them.
        Case { name: "x87_fpatan_quadrants_and_table",
               mem: mem![(0, f64b!(2.0)), (8, f64b!(-3.0)), (16, f64b!(-0.75)), (24, f64b!(-0.5)),
                         (32, f64b!(-3.0)), (40, f64b!(7.0)), (48, f64b!(1.0e-300)), (56, f64b!(-1.0)),
                         (64, f64b!(-1.0e-20)), (72, f64b!(-3.0)), (80, f64b!(0.0)), (88, f64b!(-0.0)),
                         (96, f64b!(f64::INFINITY)), (104, f64b!(f64::NEG_INFINITY)), (112, f64b!(12345.678)), (120, f64b!(-0.001))],
               asm: |a| {
                    for i in 0..8 {
                        a.fld(qword_ptr(ebx + 16 * i))?; a.fld(qword_ptr(ebx + 16 * i + 8))?; a.fpatan()?;
                        a.fnstsw(ax)?; a.and(ax, 0x0200)?; a.mov(word_ptr(ebx + 192 + 2 * i), ax)?;
                        a.fstp(qword_ptr(ebx + 128 + 8 * i))?;
                    }
                    a.xor(eax, eax)
               },
               flag_mask: 0, kind: Kind::Strict,
               expect: Expect { regs: &[], flags: None, mem: mem![
                   (128, 0x40046dc09ec29433u64.to_le_bytes()), (136, 0xc00145385fa3af71u64.to_le_bytes()),
                   (144, 0xbfd9e9bf3d20dc71u64.to_le_bytes()), (152, 0x400921fb54442d18u64.to_le_bytes()),
                   (160, 0xc00921fb54442d18u64.to_le_bytes()), (168, 0x400921fb54442d18u64.to_le_bytes()),
                   (176, 0x4002d97c7f3321d2u64.to_le_bytes()), (184, 0x3ff921fb6a027448u64.to_le_bytes()),
                   (192, [0x00u8, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x00, 0x02, 0x00, 0x02, 0x00, 0x02, 0x00, 0x02])] }, ..base.clone() },
        // FPTAN: value, the pushed 1.0, and the out-of-range case (C2=1, nothing pushed).
        Case { name: "x87_fptan_values_and_stack",
               mem: mem![(0, f64b!(0.5)), (8, f64b!(-1.0)), (16, f64b!(std::f64::consts::FRAC_PI_4)), (24, f64b!(std::f64::consts::FRAC_PI_2)),
                         (32, f64b!(1.0e10)), (40, f64b!(9223372036854775808.0))],
               asm: |a| {
                    for i in 0..5 {
                        a.fld(qword_ptr(ebx + 8 * i))?; a.fptan()?; a.fstp(qword_ptr(ebx + 48 + 16 * i))?; a.fstp(qword_ptr(ebx + 56 + 16 * i))?;
                    }
                    a.fld(qword_ptr(ebx + 40))?; a.fptan()?; a.fnstsw(ax)?; a.mov(word_ptr(ebx + 128), ax)?;
                    a.fstp(qword_ptr(ebx + 136))?; a.fnstsw(ax)?; a.mov(word_ptr(ebx + 130), ax)?;
                    a.xor(eax, eax)
               },
               flag_mask: 0, fsw_mask: 0x4700, kind: Kind::Strict,
               expect: Expect { regs: &[], flags: None, mem: mem![
                   (48, f64b!(1.0)), (56, f64b!(0.5463024898437905)), (64, f64b!(1.0)), (72, f64b!(-1.5574077246549023)),
                   (80, f64b!(1.0)), (88, f64b!(0.9999999999999999)), (96, f64b!(1.0)), (104, f64b!(1.6331778728383844e16)),
                   (112, f64b!(1.0)), (120, f64b!(-0.5583496377943541)), (136, f64b!(9223372036854775808.0))] }, ..base.clone() },
    ]
}


/// Machine state after a case, in a form both the model and the oracle produce.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub regs: [u32; 8],
    pub eflags: u32,
    pub fcw: u16,
    pub fsw: u16,
    pub mem: Vec<u8>,
}
