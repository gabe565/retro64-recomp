//! CPU state shared with generated C. Must match `struct Cpu` in `c/recomp.h`.

#[repr(C)]
#[derive(Clone, Debug)]
pub struct Cpu {
    pub eax: u32,
    pub ecx: u32,
    pub edx: u32,
    pub ebx: u32,
    pub esp: u32,
    pub ebp: u32,
    pub esi: u32,
    pub edi: u32,
    pub fl_kind: u32,
    pub fl_size: u32,
    pub fl_a: u32,
    pub fl_b: u32,
    pub fl_res: u32,
    pub fl_cin: u32,
    pub df: u32,
    pub fs_base: u32,
    pub ftop: u32,
    pub fcw: u32,
    pub fsw: u32,
    pub ftag: u32,
    pub x87_notes: u32,
    pub pad0: u32,
    pub st: [f64; 8],
}

const _: () = assert!(std::mem::size_of::<Cpu>() == 0x58 + 64);
const _: () = assert!(std::mem::offset_of!(Cpu, st) == 0x58);

impl Cpu {
    /// State of a fresh Win32 thread at the executable's entry point.
    /// Windows starts threads with x87 control word 0x027F (53-bit precision,
    /// round to nearest, exceptions masked); the original game was measured
    /// running with exactly that value.
    pub fn new(esp: u32, fs_base: u32) -> Self {
        Cpu {
            eax: 0, ecx: 0, edx: 0, ebx: 0, esp, ebp: 0, esi: 0, edi: 0,
            fl_kind: 0, fl_size: 32, fl_a: 0, fl_b: 0, fl_res: 0, fl_cin: 0,
            df: 0, fs_base, ftop: 0, fcw: 0x027f, fsw: 0, ftag: 0xffff, x87_notes: 0, pad0: 0,
            st: [0.0; 8],
        }
    }
}
