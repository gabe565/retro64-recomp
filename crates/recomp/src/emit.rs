//! C emission for one discovered guest function.

use crate::{Func, Stats};
use iced_x86::{FlowControl, Formatter, Instruction, IntelFormatter, Mnemonic, OpKind, Register};
use std::fmt::Write;

struct Ctx<'a> {
    f: &'a Func,
    out: String,
    stats: &'a mut Stats,
    fmt: IntelFormatter,
}

fn reg32_name(r: Register) -> Option<&'static str> {
    Some(match r {
        Register::EAX | Register::AX | Register::AL | Register::AH => "eax",
        Register::ECX | Register::CX | Register::CL | Register::CH => "ecx",
        Register::EDX | Register::DX | Register::DL | Register::DH => "edx",
        Register::EBX | Register::BX | Register::BL | Register::BH => "ebx",
        Register::ESP | Register::SP => "esp",
        Register::EBP | Register::BP => "ebp",
        Register::ESI | Register::SI => "esi",
        Register::EDI | Register::DI => "edi",
        _ => return None,
    })
}

fn reg_size(r: Register) -> u32 {
    match r {
        Register::AL | Register::CL | Register::DL | Register::BL | Register::AH | Register::CH | Register::DH | Register::BH => 8,
        Register::AX | Register::CX | Register::DX | Register::BX | Register::SP | Register::BP | Register::SI | Register::DI => 16,
        _ => 32,
    }
}

fn is_high8(r: Register) -> bool {
    matches!(r, Register::AH | Register::CH | Register::DH | Register::BH)
}

fn st_index(r: Register) -> Option<u32> {
    let i = (r as u32).wrapping_sub(Register::ST0 as u32);
    (i < 8).then_some(i)
}

fn mask(sz: u32) -> &'static str {
    match sz {
        8 => "0xffu",
        16 => "0xffffu",
        _ => "0xffffffffu",
    }
}

impl<'a> Ctx<'a> {
    fn line(&mut self, s: &str) {
        self.out.push_str("    ");
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn unimpl(&mut self, ins: &Instruction, why: &str) {
        let mut text = String::new();
        self.fmt.format(ins, &mut text);
        let key = format!("{:?}{}", ins.mnemonic(), if why.is_empty() { String::new() } else { format!(" ({why})") });
        *self.stats.unimplemented.entry(key).or_default() += 1;
        let esc = text.replace('\\', "\\\\").replace('"', "'");
        self.line(&format!("rt_unimpl(c, 0x{:08x}u, \"{}\"); return;", ins.ip32(), esc));
    }

    /// Effective address expression for the memory operand (without segment for DS/SS/ES).
    fn ea(&self, ins: &Instruction) -> String {
        let mut parts: Vec<String> = Vec::new();
        if ins.memory_segment() == Register::FS {
            parts.push("c->fs_base".into());
        }
        if let Some(b) = reg32_name(ins.memory_base()) {
            parts.push(format!("c->{b}"));
        }
        if let Some(i) = reg32_name(ins.memory_index()) {
            let s = ins.memory_index_scale();
            parts.push(if s == 1 { format!("c->{i}") } else { format!("c->{i} * {s}u") });
        }
        let d = ins.memory_displacement32();
        if d != 0 || parts.is_empty() {
            parts.push(format!("0x{d:08x}u"));
        }
        format!("(uint32_t)({})", parts.join(" + "))
    }

    fn mem_size(ins: &Instruction) -> u32 {
        (ins.memory_size().size() as u32) * 8
    }

    fn op_size(ins: &Instruction, i: u32) -> u32 {
        match ins.op_kind(i) {
            OpKind::Register => reg_size(ins.op_register(i)),
            OpKind::Memory => Self::mem_size(ins),
            OpKind::Immediate8 | OpKind::Immediate8_2nd => 8,
            OpKind::Immediate16 | OpKind::Immediate8to16 => 16,
            _ => 32,
        }
    }

    /// Read operand i as a zero-extended uint32_t expression (memory uses `ea`).
    fn read(&self, ins: &Instruction, i: u32) -> Option<String> {
        Some(match ins.op_kind(i) {
            OpKind::Register => {
                let r = ins.op_register(i);
                let n = reg32_name(r)?;
                match reg_size(r) {
                    8 if is_high8(r) => format!("((c->{n} >> 8) & 0xffu)"),
                    8 => format!("(c->{n} & 0xffu)"),
                    16 => format!("(c->{n} & 0xffffu)"),
                    _ => format!("c->{n}"),
                }
            }
            OpKind::Memory => match Self::mem_size(ins) {
                8 => "(uint32_t)rd8(ea)".into(),
                16 => "(uint32_t)rd16(ea)".into(),
                32 => "rd32(ea)".into(),
                _ => return None,
            },
            OpKind::Immediate8 | OpKind::Immediate8_2nd | OpKind::Immediate16 | OpKind::Immediate32
            | OpKind::Immediate8to16 | OpKind::Immediate8to32 => {
                let sz = Self::op_size(ins, i).max(Self::op_size(ins, 0));
                let v = (ins.immediate(i) as u32) & if sz == 8 { 0xff } else if sz == 16 { 0xffff } else { 0xffff_ffff };
                format!("0x{v:08x}u")
            }
            _ => return None,
        })
    }

    /// Statement writing `val` (uint32_t expression) to operand i.
    fn write(&self, ins: &Instruction, i: u32, val: &str) -> Option<String> {
        Some(match ins.op_kind(i) {
            OpKind::Register => {
                let r = ins.op_register(i);
                let n = reg32_name(r)?;
                match reg_size(r) {
                    8 if is_high8(r) => format!("c->{n} = set8hi(c->{n}, {val});"),
                    8 => format!("c->{n} = set8lo(c->{n}, {val});"),
                    16 => format!("c->{n} = set16(c->{n}, {val});"),
                    _ => format!("c->{n} = {val};"),
                }
            }
            OpKind::Memory => match Self::mem_size(ins) {
                8 => format!("wr8(ea, (uint8_t)({val}));"),
                16 => format!("wr16(ea, (uint16_t)({val}));"),
                32 => format!("wr32(ea, {val});"),
                _ => return None,
            },
            _ => return None,
        })
    }

    fn has_mem(ins: &Instruction) -> bool {
        (0..ins.op_count()).any(|i| ins.op_kind(i) == OpKind::Memory)
    }

    fn branch_to(&self, from: u32, target: u32) -> String {
        if let Some(&t) = self.f.tail_jump.get(&from) {
            if t == target {
                return format!("{{ f_{t:08x}(c); return; }}");
            }
        }
        format!("goto L_{target:08x};")
    }

    fn cond(m: Mnemonic) -> Option<&'static str> {
        Some(match m {
            Mnemonic::Jo | Mnemonic::Seto => "cc_o(c)",
            Mnemonic::Jno | Mnemonic::Setno => "!cc_o(c)",
            Mnemonic::Jb | Mnemonic::Setb => "cc_b(c)",
            Mnemonic::Jae | Mnemonic::Setae => "!cc_b(c)",
            Mnemonic::Je | Mnemonic::Sete => "cc_e(c)",
            Mnemonic::Jne | Mnemonic::Setne => "!cc_e(c)",
            Mnemonic::Jbe | Mnemonic::Setbe => "cc_be(c)",
            Mnemonic::Ja | Mnemonic::Seta => "!cc_be(c)",
            Mnemonic::Js | Mnemonic::Sets => "cc_s(c)",
            Mnemonic::Jns | Mnemonic::Setns => "!cc_s(c)",
            Mnemonic::Jp | Mnemonic::Setp => "cc_p(c)",
            Mnemonic::Jnp | Mnemonic::Setnp => "!cc_p(c)",
            Mnemonic::Jl | Mnemonic::Setl => "cc_l(c)",
            Mnemonic::Jge | Mnemonic::Setge => "!cc_l(c)",
            Mnemonic::Jle | Mnemonic::Setle => "cc_le(c)",
            Mnemonic::Jg | Mnemonic::Setg => "!cc_le(c)",
            _ => return None,
        })
    }

    fn emit_insn(&mut self, ins: &Instruction) {
        let a = ins.ip32();
        let next = ins.next_ip32();
        let m = ins.mnemonic();
        let mut text = String::new();
        self.fmt.format(ins, &mut text);
        let _ = writeln!(self.out, "    /* {a:08x}: {} */", text.replace("*/", "* /"));
        self.stats.instructions += 1;

        // String instructions use implicit operands; handle them first.
        if matches!(ins.op_kind(0), OpKind::MemoryESEDI | OpKind::MemorySegESI)
            || (ins.op_count() > 1 && matches!(ins.op_kind(1), OpKind::MemoryESEDI | OpKind::MemorySegESI))
        {
            self.emit_string(ins);
            return;
        }

        let mem = Self::has_mem(ins) && m != Mnemonic::Lea && m != Mnemonic::Pop;
        if mem {
            let ea = self.ea(ins);
            self.line(&format!("{{ uint32_t ea = {ea}; (void)ea;"));
        } else {
            self.line("{");
        }
        if !self.emit_body(ins, a, next) {
            self.unimpl(ins, "");
        }
        self.line("}");
    }

    fn emit_body(&mut self, ins: &Instruction, a: u32, next: u32) -> bool {
        let m = ins.mnemonic();
        let sz = Self::op_size(ins, 0);
        macro_rules! rd { ($i:expr) => { match self.read(ins, $i) { Some(v) => v, None => return false } }; }
        macro_rules! wr { ($i:expr, $v:expr) => { match self.write(ins, $i, $v) { Some(v) => v, None => return false } }; }
        match m {
            Mnemonic::Nop | Mnemonic::Wait => {}
            Mnemonic::Mov | Mnemonic::Movzx => {
                let v = rd!(1);
                let s = wr!(0, &v);
                self.line(&s);
            }
            Mnemonic::Movsx => {
                let v = rd!(1);
                let ext = if Self::op_size(ins, 1) == 8 { format!("sx8({v})") } else { format!("sx16({v})") };
                let s = wr!(0, &format!("({ext} & {})", mask(sz)));
                self.line(&s);
            }
            Mnemonic::Lea => {
                let ea = self.ea(ins);
                let s = wr!(0, &format!("({ea} & {})", mask(sz)));
                self.line(&s);
            }
            Mnemonic::Push => {
                if sz == 16 && ins.op_kind(0) != OpKind::Immediate8to32 { return false; }
                let v = rd!(0);
                self.line(&format!("uint32_t v = {v}; push32(c, v);"));
            }
            Mnemonic::Pop => {
                if sz == 16 { return false; }
                if ins.op_kind(0) == OpKind::Memory {
                    let ea = self.ea(ins);
                    self.line(&format!("uint32_t v = pop32(c); uint32_t ea = {ea}; wr32(ea, v);"));
                } else {
                    let s = wr!(0, "v");
                    self.line(&format!("uint32_t v = pop32(c); {s}"));
                }
            }
            Mnemonic::Add | Mnemonic::Adc | Mnemonic::Sub | Mnemonic::Sbb | Mnemonic::Cmp => {
                let (x, y) = (rd!(0), rd!(1));
                let f = match m { Mnemonic::Add => "op_add", Mnemonic::Adc => "op_adc", Mnemonic::Sbb => "op_sbb", _ => "op_sub" };
                if m == Mnemonic::Cmp {
                    self.line(&format!("(void){f}(c, {sz}u, {x}, {y});"));
                } else {
                    let s = wr!(0, "r");
                    self.line(&format!("uint32_t r = {f}(c, {sz}u, {x}, {y}); {s}"));
                }
            }
            Mnemonic::And | Mnemonic::Or | Mnemonic::Xor | Mnemonic::Test => {
                let (x, y) = (rd!(0), rd!(1));
                let op = match m { Mnemonic::And | Mnemonic::Test => "&", Mnemonic::Or => "|", _ => "^" };
                if m == Mnemonic::Test {
                    self.line(&format!("(void)op_logic(c, {sz}u, {x} {op} {y});"));
                } else {
                    let s = wr!(0, "r");
                    self.line(&format!("uint32_t r = op_logic(c, {sz}u, {x} {op} {y}); {s}"));
                }
            }
            Mnemonic::Inc | Mnemonic::Dec | Mnemonic::Neg => {
                let x = rd!(0);
                let f = match m { Mnemonic::Inc => "op_inc", Mnemonic::Dec => "op_dec", _ => "op_neg" };
                let s = wr!(0, "r");
                self.line(&format!("uint32_t r = {f}(c, {sz}u, {x}); {s}"));
            }
            Mnemonic::Not => {
                let x = rd!(0);
                let s = wr!(0, &format!("(~{x} & {})", mask(sz)));
                self.line(&s);
            }
            Mnemonic::Shl | Mnemonic::Sal | Mnemonic::Shr | Mnemonic::Sar | Mnemonic::Rol | Mnemonic::Ror => {
                let x = rd!(0);
                let n = if ins.op_count() > 1 { rd!(1) } else { "1u".into() };
                let f = match m { Mnemonic::Shl | Mnemonic::Sal => "op_shl", Mnemonic::Shr => "op_shr", Mnemonic::Sar => "op_sar", Mnemonic::Rol => "op_rol", _ => "op_ror" };
                let s = wr!(0, "r");
                self.line(&format!("uint32_t r = {f}(c, {sz}u, {x}, ({n}) & 31u); {s}"));
            }
            Mnemonic::Shld | Mnemonic::Shrd => {
                if sz != 32 { return false; }
                let (x, y, n) = (rd!(0), rd!(1), rd!(2));
                let f = if m == Mnemonic::Shld { "op_shld" } else { "op_shrd" };
                let s = wr!(0, "r");
                self.line(&format!("uint32_t r = {f}(c, {x}, {y}, ({n}) & 31u); {s}"));
            }
            Mnemonic::Xchg => {
                let (x, y) = (rd!(0), rd!(1));
                let (s0, s1) = (wr!(0, "t1"), wr!(1, "t0"));
                self.line(&format!("uint32_t t0 = {x}, t1 = {y}; {s0} {s1}"));
            }
            Mnemonic::Imul => self.emit_imul(ins, sz),
            Mnemonic::Mul => {
                let x = rd!(0);
                match sz {
                    32 => self.line(&format!("uint64_t p = (uint64_t)c->eax * (uint64_t)({x}); c->eax = (uint32_t)p; c->edx = (uint32_t)(p >> 32); setfl(c, FK_MUL, 32u, 0, c->edx != 0, c->eax);")),
                    16 => self.line(&format!("uint32_t p = (c->eax & 0xffffu) * ({x}); c->eax = set16(c->eax, p); c->edx = set16(c->edx, p >> 16); setfl(c, FK_MUL, 16u, 0, (p >> 16) != 0, p & 0xffffu);")),
                    _ => self.line(&format!("uint32_t p = (c->eax & 0xffu) * ({x}); c->eax = set16(c->eax, p); setfl(c, FK_MUL, 8u, 0, (p >> 8) != 0, p & 0xffu);")),
                }
            }
            Mnemonic::Div | Mnemonic::Idiv => {
                let x = rd!(0);
                let signed = m == Mnemonic::Idiv;
                match (sz, signed) {
                    (32, false) => self.line(&format!(
                        "uint32_t d = {x}; uint64_t n = ((uint64_t)c->edx << 32) | c->eax; if (d == 0 || n / d > 0xffffffffull) {{ rt_div_error(c, 0x{a:08x}u); return; }} c->eax = (uint32_t)(n / d); c->edx = (uint32_t)(n % d);")),
                    (32, true) => self.line(&format!(
                        "int64_t d = (int64_t)s32({x}); int64_t n = s64(((uint64_t)c->edx << 32) | c->eax); if (d == 0 || (n == INT64_MIN && d == -1)) {{ rt_div_error(c, 0x{a:08x}u); return; }} {{ int64_t q = n / d, r = n % d; if (q > 2147483647LL || q < -2147483648LL) {{ rt_div_error(c, 0x{a:08x}u); return; }} c->eax = (uint32_t)(uint64_t)q; c->edx = (uint32_t)(uint64_t)r; }}")),
                    (16, false) => self.line(&format!(
                        "uint32_t d = {x}; uint32_t n = ((c->edx & 0xffffu) << 16) | (c->eax & 0xffffu); if (d == 0 || n / d > 0xffffu) {{ rt_div_error(c, 0x{a:08x}u); return; }} c->eax = set16(c->eax, n / d); c->edx = set16(c->edx, n % d);")),
                    (8, false) => self.line(&format!(
                        "uint32_t d = {x}; uint32_t n = c->eax & 0xffffu; if (d == 0 || n / d > 0xffu) {{ rt_div_error(c, 0x{a:08x}u); return; }} c->eax = set16(c->eax, ((n % d) << 8) | (n / d));")),
                    _ => return false,
                }
            }
            Mnemonic::Cdq => self.line("c->edx = (c->eax & 0x80000000u) ? 0xffffffffu : 0u;"),
            Mnemonic::Cwde => self.line("c->eax = sx16(c->eax);"),
            Mnemonic::Cbw => self.line("c->eax = set16(c->eax, sx8(c->eax));"),
            Mnemonic::Cld => self.line("c->df = 0;"),
            Mnemonic::Std => self.line("c->df = 1;"),
            Mnemonic::Clc => self.line("flags_set_explicit(c, flags_get(c) & ~F_CF);"),
            Mnemonic::Stc => self.line("flags_set_explicit(c, flags_get(c) | F_CF);"),
            Mnemonic::Cmc => self.line("flags_set_explicit(c, flags_get(c) ^ F_CF);"),
            Mnemonic::Sahf => self.line("op_sahf(c);"),
            Mnemonic::Lahf => self.line("op_lahf(c);"),
            Mnemonic::Leave => self.line("c->esp = c->ebp; c->ebp = pop32(c);"),
            Mnemonic::Bt => {
                let (x, y) = (rd!(0), rd!(1));
                self.line(&format!("uint32_t f = flags_get(c) & ~F_CF; if (({x} >> (({y}) & {}u)) & 1u) f |= F_CF; flags_set_explicit(c, f);", sz - 1));
            }
            _ if Self::cond(m).is_some() && ins.flow_control() == FlowControl::ConditionalBranch => {
                let t = ins.near_branch32();
                let br = self.branch_to(a, t);
                self.line(&format!("if ({}) {br}", Self::cond(m).unwrap()));
            }
            _ if Self::cond(m).is_some() => {
                let s = wr!(0, &format!("(uint32_t)({})", Self::cond(m).unwrap()));
                self.line(&s);
            }
            Mnemonic::Jecxz => {
                let br = self.branch_to(a, ins.near_branch32());
                self.line(&format!("if (c->ecx == 0) {br}"));
            }
            Mnemonic::Loop => {
                let br = self.branch_to(a, ins.near_branch32());
                self.line(&format!("c->ecx -= 1u; if (c->ecx != 0) {br}"));
            }
            Mnemonic::Jmp => {
                if ins.flow_control() == FlowControl::UnconditionalBranch {
                    let br = self.branch_to(a, ins.near_branch32());
                    self.line(&br);
                } else if let Some(ts) = self.f.tables.get(&a).cloned() {
                    self.stats.jump_tables += 1;
                    let t = rd!(0);
                    let mut s = format!("switch ({t}) {{ ");
                    for x in &ts {
                        let _ = write!(s, "case 0x{x:08x}u: goto L_{x:08x}; ");
                    }
                    let _ = write!(s, "default: rt_bad_jump(c, 0x{a:08x}u, {t}); return; }}");
                    self.line(&s);
                } else {
                    // Indirect jump with no resolved table: tail call through dispatch.
                    self.stats.tail_calls += 1;
                    let t = rd!(0);
                    self.line(&format!("recomp_call_indirect(c, {t}, 0x{a:08x}u); return;"));
                }
            }
            Mnemonic::Call => {
                if ins.flow_control() == FlowControl::Call {
                    let t = ins.near_branch32();
                    self.line(&format!("push32(c, 0x{next:08x}u); f_{t:08x}(c);"));
                } else {
                    self.stats.indirect_calls += 1;
                    let t = rd!(0);
                    self.line(&format!("uint32_t t = {t}; push32(c, 0x{next:08x}u); recomp_call_indirect(c, t, 0x{a:08x}u);"));
                }
            }
            Mnemonic::Ret => {
                let n = if ins.op_count() > 0 { ins.immediate(0) as u32 } else { 0 };
                self.line(&format!("c->esp += {}u; return;", 4 + n));
            }
            Mnemonic::Int3 | Mnemonic::Hlt => return false,
            _ if self.emit_x87(ins) => {}
            _ => return false,
        }
        true
    }

    fn emit_imul(&mut self, ins: &Instruction, sz: u32) {
        let ok = (|| -> Option<()> {
            match ins.op_count() {
                1 => {
                    let x = self.read(ins, 0)?;
                    match sz {
                        32 => self.line(&format!("int64_t p = (int64_t)s32(c->eax) * (int64_t)s32({x}); uint64_t up = (uint64_t)p; c->eax = (uint32_t)up; c->edx = (uint32_t)(up >> 32); setfl(c, FK_MUL, 32u, 0, p != (int64_t)s32(c->eax), c->eax);")),
                        16 => self.line(&format!("int32_t p = s32(sx16(c->eax)) * s32(sx16({x})); uint32_t up = (uint32_t)p; c->eax = set16(c->eax, up); c->edx = set16(c->edx, up >> 16); setfl(c, FK_MUL, 16u, 0, p != s32(sx16(up)), up & 0xffffu);")),
                        _ => self.line(&format!("int32_t p = s32(sx8(c->eax)) * s32(sx8({x})); uint32_t up = (uint32_t)p; c->eax = set16(c->eax, up); setfl(c, FK_MUL, 8u, 0, p != s32(sx8(up)), up & 0xffu);")),
                    }
                }
                2 | 3 => {
                    if sz != 32 { return None; }
                    let (x, y) = if ins.op_count() == 2 { (self.read(ins, 0)?, self.read(ins, 1)?) } else { (self.read(ins, 1)?, self.read(ins, 2)?) };
                    let s = self.write(ins, 0, "(uint32_t)(uint64_t)p")?;
                    self.line(&format!("int64_t p = (int64_t)s32({x}) * (int64_t)s32({y}); {s} setfl(c, FK_MUL, 32u, 0, p != (int64_t)s32((uint32_t)(uint64_t)p), (uint32_t)(uint64_t)p);"));
                }
                _ => return None,
            }
            Some(())
        })();
        if ok.is_none() {
            self.unimpl(ins, "imul form");
        }
    }

    fn emit_string(&mut self, ins: &Instruction) {
        let m = ins.mnemonic();
        let (kind, sz) = match m {
            Mnemonic::Movsb => ("movs", 1), Mnemonic::Movsw => ("movs", 2), Mnemonic::Movsd => ("movs", 4),
            Mnemonic::Stosb => ("stos", 1), Mnemonic::Stosw => ("stos", 2), Mnemonic::Stosd => ("stos", 4),
            Mnemonic::Lodsb => ("lods", 1), Mnemonic::Lodsw => ("lods", 2), Mnemonic::Lodsd => ("lods", 4),
            Mnemonic::Scasb => ("scas", 1), Mnemonic::Scasw => ("scas", 2), Mnemonic::Scasd => ("scas", 4),
            Mnemonic::Cmpsb => ("cmps", 1), Mnemonic::Cmpsw => ("cmps", 2), Mnemonic::Cmpsd => ("cmps", 4),
            _ => { self.line("{"); self.unimpl(ins, "string"); self.line("}"); return; }
        };
        let bits = sz * 8;
        let (rd, wr) = match sz { 1 => ("rd8", "wr8"), 2 => ("rd16", "wr16"), _ => ("rd32", "wr32") };
        let acc = match sz { 1 => "(c->eax & 0xffu)", 2 => "(c->eax & 0xffffu)", _ => "c->eax" };
        let set_acc = match sz { 1 => "c->eax = set8lo(c->eax, v);", 2 => "c->eax = set16(c->eax, v);", _ => "c->eax = v;" };
        let step = format!("(c->df ? (uint32_t)(0u - {sz}u) : {sz}u)");
        let body = match kind {
            "movs" => format!("{wr}(c->edi, {rd}(c->esi)); c->esi += st; c->edi += st;"),
            "stos" => format!("{wr}(c->edi, ({})({acc})); c->edi += st;", match sz { 1 => "uint8_t", 2 => "uint16_t", _ => "uint32_t" }),
            "lods" => format!("uint32_t v = {rd}(c->esi); {set_acc} c->esi += st;"),
            "scas" => format!("(void)op_sub(c, {bits}u, {acc}, (uint32_t){rd}(c->edi)); c->edi += st;"),
            _ => format!("(void)op_sub(c, {bits}u, (uint32_t){rd}(c->esi), (uint32_t){rd}(c->edi)); c->esi += st; c->edi += st;"),
        };
        let compares = kind == "scas" || kind == "cmps";
        self.line(&format!("{{ uint32_t st = {step}; (void)st;"));
        if ins.has_rep_prefix() || ins.has_repe_prefix() || ins.has_repne_prefix() {
            let stop = if !compares {
                String::new()
            } else if ins.has_repne_prefix() {
                " if (cc_e(c)) break;".to_string()
            } else {
                " if (!cc_e(c)) break;".to_string()
            };
            self.line(&format!("    while (c->ecx != 0) {{ {body} c->ecx -= 1u;{stop} }}"));
        } else {
            self.line(&format!("    {body}"));
        }
        self.line("}");
    }

    /// x87. Returns false if the instruction is not an x87 form handled here.
    fn emit_x87(&mut self, ins: &Instruction) -> bool {
        let m = ins.mnemonic();
        let has_mem = Self::has_mem(ins);
        let msz = if has_mem { Self::mem_size(ins) } else { 0 };
        let st = |r: Register| st_index(r).map(|i| format!("ST({i})"));
        let memf = |msz: u32| -> Option<&'static str> { match msz { 32 => Some("fld_m32(ea)"), 64 => Some("fld_m64(ea)"), 80 => Some("rt_f80_load(c, ea)"), _ => None } };
        let memi = |msz: u32| -> Option<&'static str> { match msz { 16 => Some("(double)s32(sx16(rd16(ea)))"), 32 => Some("(double)s32(rd32(ea))"), 64 => Some("(double)s64(rd64(ea))"), _ => None } };
        let s: String = match m {
            Mnemonic::Fld => {
                if has_mem { match memf(msz) { Some(v) => format!("fpush(c, {v});"), None => return false } }
                else { match st(ins.op_register(0)) { Some(r) => format!("{{ double v = {r}; fpush(c, v); }}"), None => return false } }
            }
            Mnemonic::Fild => match memi(msz) { Some(v) => format!("fpush(c, {v});"), None => return false },
            Mnemonic::Fld1 => "fpush(c, 1.0);".into(),
            Mnemonic::Fldz => "fpush(c, 0.0);".into(),
            Mnemonic::Fldpi => "rt_x87_note(c, X87_NOTE_F80); fpush(c, 3.141592653589793115997963468544185161590576171875);".into(),
            Mnemonic::Fldl2e => "rt_x87_note(c, X87_NOTE_F80); fpush(c, 1.442695040888963387004650940070860087871551513671875);".into(),
            Mnemonic::Fldln2 => "rt_x87_note(c, X87_NOTE_F80); fpush(c, 0.69314718055994528622676398299518041312694549560546875);".into(),
            Mnemonic::Fldlg2 => "rt_x87_note(c, X87_NOTE_F80); fpush(c, 0.30102999566398119802345015280298001319169998168945312);".into(),
            Mnemonic::Fst | Mnemonic::Fstp => {
                let pop = if m == Mnemonic::Fstp { " fpop(c);" } else { "" };
                if has_mem {
                    match msz {
                        32 => format!("fst_m32(c, ea, ST(0));{pop}"),
                        64 => format!("fst_m64(ea, ST(0));{pop}"),
                        80 => format!("rt_f80_store(c, ea, ST(0));{pop}"),
                        _ => return false,
                    }
                } else {
                    match st(ins.op_register(0)) { Some(r) => format!("{r} = ST(0);{pop}"), None => return false }
                }
            }
            Mnemonic::Fist | Mnemonic::Fistp => {
                let pop = if m == Mnemonic::Fistp { " fpop(c);" } else { "" };
                match msz {
                    16 => format!("wr16(ea, (uint16_t)(uint64_t)fist_value(c, ST(0), 16));{pop}"),
                    32 => format!("wr32(ea, (uint32_t)(uint64_t)fist_value(c, ST(0), 32));{pop}"),
                    64 => format!("wr64(ea, (uint64_t)fist_value(c, ST(0), 64));{pop}"),
                    _ => return false,
                }
            }
            Mnemonic::Fadd | Mnemonic::Faddp | Mnemonic::Fsub | Mnemonic::Fsubp | Mnemonic::Fsubr | Mnemonic::Fsubrp
            | Mnemonic::Fmul | Mnemonic::Fmulp | Mnemonic::Fdiv | Mnemonic::Fdivp | Mnemonic::Fdivr | Mnemonic::Fdivrp
            | Mnemonic::Fiadd | Mnemonic::Fisub | Mnemonic::Fisubr | Mnemonic::Fimul | Mnemonic::Fidiv | Mnemonic::Fidivr => {
                let (op, rev) = match m {
                    Mnemonic::Fadd | Mnemonic::Faddp | Mnemonic::Fiadd => ("+", false),
                    Mnemonic::Fsub | Mnemonic::Fsubp | Mnemonic::Fisub => ("-", false),
                    Mnemonic::Fsubr | Mnemonic::Fsubrp | Mnemonic::Fisubr => ("-", true),
                    Mnemonic::Fmul | Mnemonic::Fmulp | Mnemonic::Fimul => ("*", false),
                    Mnemonic::Fdiv | Mnemonic::Fdivp | Mnemonic::Fidiv => ("/", false),
                    _ => ("/", true),
                };
                let int = matches!(m, Mnemonic::Fiadd | Mnemonic::Fisub | Mnemonic::Fisubr | Mnemonic::Fimul | Mnemonic::Fidiv | Mnemonic::Fidivr);
                let pop = matches!(m, Mnemonic::Faddp | Mnemonic::Fsubp | Mnemonic::Fsubrp | Mnemonic::Fmulp | Mnemonic::Fdivp | Mnemonic::Fdivrp);
                let (dst, src) = if has_mem {
                    let v = if int { memi(msz) } else { memf(msz) };
                    match v { Some(v) => ("ST(0)".to_string(), v.to_string()), None => return false }
                } else if ins.op_count() == 2 {
                    match (st(ins.op_register(0)), st(ins.op_register(1))) { (Some(d), Some(s)) => (d, s), _ => return false }
                } else if ins.op_count() == 1 {
                    // e.g. "fsubp st(1)" forms decoded with a single register
                    match st(ins.op_register(0)) { Some(d) => (d, "ST(0)".to_string()), None => return false }
                } else {
                    ("ST(1)".to_string(), "ST(0)".to_string())
                };
                let expr = if rev { format!("s_ {op} {dst}") } else { format!("{dst} {op} s_") };
                format!("{{ double s_ = {src}; {dst} = fprnd(c, {expr}); }}{}", if pop { " fpop(c);" } else { "" })
            }
            Mnemonic::Fcom | Mnemonic::Fcomp | Mnemonic::Fucom | Mnemonic::Fucomp | Mnemonic::Ficom | Mnemonic::Ficomp => {
                let src = if has_mem {
                    let v = if matches!(m, Mnemonic::Ficom | Mnemonic::Ficomp) { memi(msz) } else { memf(msz) };
                    match v { Some(v) => v.to_string(), None => return false }
                } else {
                    let r = if ins.op_count() == 2 { ins.op_register(1) } else if ins.op_count() == 1 { ins.op_register(0) } else { Register::ST1 };
                    match st(r) { Some(s) => s, None => return false }
                };
                let pop = matches!(m, Mnemonic::Fcomp | Mnemonic::Fucomp | Mnemonic::Ficomp);
                format!("fcom_set(c, ST(0), {src});{}", if pop { " fpop(c);" } else { "" })
            }
            Mnemonic::Fcompp | Mnemonic::Fucompp => "fcom_set(c, ST(0), ST(1)); fpop(c); fpop(c);".into(),
            Mnemonic::Ftst => "fcom_set(c, ST(0), 0.0);".into(),
            Mnemonic::Fxch => {
                let r = if ins.op_count() == 2 { ins.op_register(1) } else if ins.op_count() == 1 { ins.op_register(0) } else { Register::ST1 };
                match st(r) { Some(s) => format!("{{ double t = ST(0); ST(0) = {s}; {s} = t; }}"), None => return false }
            }
            Mnemonic::Fchs => "ST(0) = -ST(0);".into(),
            Mnemonic::Fabs => "ST(0) = fabs(ST(0));".into(),
            Mnemonic::Fsqrt => "ST(0) = fprnd(c, sqrt(ST(0)));".into(),
            Mnemonic::Fsin => "rt_x87_note(c, X87_NOTE_TRANSCENDENTAL); ST(0) = sin(ST(0)); c->fsw &= ~0x0400u;".into(),
            Mnemonic::Fcos => "rt_x87_note(c, X87_NOTE_TRANSCENDENTAL); ST(0) = cos(ST(0)); c->fsw &= ~0x0400u;".into(),
            Mnemonic::Fptan => "rt_x87_note(c, X87_NOTE_TRANSCENDENTAL); ST(0) = tan(ST(0)); c->fsw &= ~0x0400u; fpush(c, 1.0);".into(),
            Mnemonic::Fpatan => "rt_x87_note(c, X87_NOTE_TRANSCENDENTAL); ST(1) = atan2(ST(1), ST(0)); fpop(c);".into(),
            Mnemonic::Frndint => "ST(0) = frnd_rc(c, ST(0));".into(),
            Mnemonic::Fnstsw | Mnemonic::Fstsw => {
                if has_mem { "wr16(ea, (uint16_t)fnstsw(c));".into() } else { "c->eax = set16(c->eax, fnstsw(c));".into() }
            }
            Mnemonic::Fnstcw | Mnemonic::Fstcw => "wr16(ea, (uint16_t)c->fcw);".into(),
            // Bit 6 of the control word is reserved and reads back as 1 on all x87 FPUs.
            Mnemonic::Fldcw => "c->fcw = ((uint32_t)rd16(ea) & 0x1f7fu) | 0x40u;".into(),
            Mnemonic::Fninit | Mnemonic::Finit => "fninit(c);".into(),
            Mnemonic::Fnclex | Mnemonic::Fclex => "c->fsw &= ~0x80ffu;".into(),
            Mnemonic::Ffree => match st(ins.op_register(0)) {
                Some(_) => { let i = st_index(ins.op_register(0)).unwrap(); format!("c->ftag |= 3u << (2u * ((c->ftop + {i}u) & 7u));") }
                None => return false,
            },
            _ => return false,
        };
        self.line(&s);
        true
    }
}

pub(crate) fn emit_function(f: &Func, is_override: bool, stats: &mut Stats) -> String {
    let mut fmt = IntelFormatter::new();
    fmt.options_mut().set_hex_prefix("0x");
    fmt.options_mut().set_hex_suffix("");
    let mut cx = Ctx { f, out: String::new(), stats, fmt };
    let _ = writeln!(cx.out, "/* {} */\nvoid f_{:08x}(Cpu *c) {{", f.name, f.entry);
    if is_override {
        let _ = writeln!(cx.out, "    rt_override(c, 0x{:08x}u);\n}}", f.entry);
        return cx.out;
    }
    if !f.decode_errors.is_empty() {
        cx.stats.decode_errors += f.decode_errors.len();
    }
    let insns: Vec<(u32, Instruction)> = f.insns.iter().map(|(a, i)| (*a, *i)).collect();
    // The entry may not be the lowest address; jump to it first.
    if insns.first().map(|x| x.0) != Some(f.entry) {
        let _ = writeln!(cx.out, "    goto L_{:08x};", f.entry);
    }
    for (idx, (a, ins)) in insns.iter().enumerate() {
        if f.labels.contains(a) || (*a == f.entry && idx != 0) {
            let _ = writeln!(cx.out, "L_{a:08x}: ;");
        }
        cx.emit_insn(ins);
        // Continuation: fallthrough into another function, an undecodable
        // successor, or a gap before the next emitted instruction.
        let fc = ins.flow_control();
        let falls = matches!(fc, FlowControl::Next | FlowControl::Call | FlowControl::IndirectCall | FlowControl::ConditionalBranch);
        if falls {
            let nxt = ins.next_ip32();
            if let Some(t) = f.fall_into.get(a) {
                let _ = writeln!(cx.out, "    f_{t:08x}(c); return; /* falls into next function */");
            } else if f.decode_errors.contains(&nxt) {
                let _ = writeln!(cx.out, "    rt_unimpl(c, 0x{nxt:08x}u, \"undecodable fallthrough\"); return;");
            } else if insns.get(idx + 1).map(|x| x.0) != Some(nxt) {
                let _ = writeln!(cx.out, "    goto L_{nxt:08x};");
            }
        }
    }
    // Labels referenced by gotos for fallthrough gaps must exist: every decoded
    // instruction that is the target of a gap goto is a decoded address, so make
    // sure it has a label.
    let mut need: Vec<u32> = Vec::new();
    for (idx, (a, ins)) in insns.iter().enumerate() {
        let fc = ins.flow_control();
        if matches!(fc, FlowControl::Next | FlowControl::Call | FlowControl::IndirectCall | FlowControl::ConditionalBranch)
            && !f.fall_into.contains_key(a)
        {
            let nxt = ins.next_ip32();
            if insns.get(idx + 1).map(|x| x.0) != Some(nxt) && !f.labels.contains(&nxt) && f.insns.contains_key(&nxt) {
                need.push(nxt);
            }
        }
    }
    let mut out = cx.out;
    for n in need {
        let marker = format!("    /* {n:08x}: ");
        if let Some(pos) = out.find(&marker) {
            out.insert_str(pos, &format!("L_{n:08x}: ;\n"));
        }
    }
    if insns.first().map(|x| x.0) != Some(f.entry) && !f.labels.contains(&f.entry) {
        // entry label was emitted in the loop via the (*a == f.entry && idx != 0) rule
    }
    out.push_str("}\n");
    out
}
