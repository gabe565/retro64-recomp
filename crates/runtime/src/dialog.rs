//! Headless emulation of the game's modal dialogs (USER32 `DialogBoxParamA`).
//!
//! The only dialog is the video-selection box (resource 0x66, dialog proc
//! 0x408280): three list boxes (driver 0x3eb, device 0x3ec, mode 0x3ed). The
//! original dialog proc runs unchanged against emulated list boxes: it receives
//! `WM_INITDIALOG`, fills and selects the lists, and is then sent `IDOK`, so its
//! own code commits the selection (from `prefs` or its defaults).
//!
//! Known deviation: the dialog is not shown, so the user cannot pick a
//! different driver, device or mode yet (docs/runtime-status.md).

use crate::{call_guest_stdcall, fatal, with_rt, Cpu};
use std::collections::BTreeMap;

const WM_INITDIALOG: u32 = 0x110;
const WM_COMMAND: u32 = 0x111;
const IDOK: u32 = 1;
pub const LB_ERR: u32 = 0xffff_ffff;

pub const DIALOG_HWND: u32 = 0x0002_0066;

/// An unsorted, single-selection Win32 list box.
#[derive(Default, Debug)]
pub struct ListBox {
    pub items: Vec<String>,
    pub sel: Option<usize>,
}

impl ListBox {
    /// LB_ADDSTRING: append; returns the new index.
    pub fn add(&mut self, s: String) -> u32 {
        self.items.push(s);
        self.items.len() as u32 - 1
    }

    /// LB_DELETESTRING: returns the remaining count, or LB_ERR.
    pub fn delete(&mut self, i: usize) -> u32 {
        if i >= self.items.len() {
            return LB_ERR;
        }
        self.items.remove(i);
        self.sel = match self.sel {
            Some(s) if s == i => None,
            Some(s) if s > i => Some(s - 1),
            s => s,
        };
        self.items.len() as u32
    }

    /// LB_GETCURSEL.
    pub fn cur_sel(&self) -> u32 {
        self.sel.map_or(LB_ERR, |s| s as u32)
    }

    /// LB_SELECTSTRING: the first item, searching after `after` (`u32::MAX` =
    /// from the start) and wrapping, that begins with `prefix` (case-insensitive).
    pub fn select_string(&mut self, after: u32, prefix: &str) -> u32 {
        let n = self.items.len();
        if n == 0 {
            return LB_ERR;
        }
        let start = if after == u32::MAX { 0 } else { (after as usize + 1) % n };
        let found = (0..n).map(|k| (start + k) % n).find(|&i| {
            let it = self.items[i].as_bytes();
            it.len() >= prefix.len() && it[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
        });
        match found {
            Some(i) => {
                self.sel = Some(i);
                i as u32
            }
            None => LB_ERR,
        }
    }
}

#[derive(Default)]
pub struct Dialog {
    lists: BTreeMap<u32, ListBox>,
    title: String,
    result: Option<u32>,
}

fn arg(cpu: &Cpu, i: u32) -> u32 {
    with_rt(|rt| rt.mem.r32(cpu.esp + 4 + 4 * i))
}

fn ret(cpu: &mut Cpu, value: u32, nargs: u32) {
    cpu.eax = value;
    cpu.esp += 4 + 4 * nargs;
}

/// USER32 imports that belong to dialogs. Returns false if not handled here.
pub(crate) fn import(cpu: &mut Cpu, name: &str) -> bool {
    match name {
        "DialogBoxParamA" => {
            // (hInstance, template, hWndParent, DLGPROC, dwInitParam)
            let (template, proc_, param) = (arg(cpu, 1), arg(cpu, 3), arg(cpu, 4));
            if template != 0x66 {
                fatal(&format!("DialogBoxParamA: only the video dialog (0x66) is emulated, got {template:#x}"));
            }
            with_rt(|rt| rt.dialog = Some(Dialog::default()));
            call_guest_stdcall(cpu, proc_, &[DIALOG_HWND, WM_INITDIALOG, 0, param]);
            if with_rt(|rt| rt.dialog.as_ref().unwrap().result.is_none()) {
                call_guest_stdcall(cpu, proc_, &[DIALOG_HWND, WM_COMMAND, IDOK, 0]);
            }
            let r = with_rt(|rt| rt.dialog.take().unwrap().result)
                .unwrap_or_else(|| fatal("video dialog did not call EndDialog after IDOK"));
            ret(cpu, r, 5);
        }
        "EndDialog" => {
            let r = arg(cpu, 1);
            with_rt(|rt| rt.dialog.as_mut().unwrap_or_else(|| fatal("EndDialog outside a dialog")).result = Some(r));
            ret(cpu, 1, 2);
        }
        "SendDlgItemMessageA" => {
            let (id, msg, wp, lp) = (arg(cpu, 1), arg(cpu, 2), arg(cpu, 3), arg(cpu, 4));
            let r = with_rt(|rt| {
                let text = |p: u32| rt.mem.cstr(p);
                let lb_text = if matches!(msg, 0x180 | 0x18c) { text(lp) } else { String::new() };
                let d = rt.dialog.as_mut().unwrap_or_else(|| fatal("SendDlgItemMessageA outside a dialog"));
                let lb = d.lists.entry(id).or_default();
                match msg {
                    0x180 => lb.add(lb_text),
                    0x182 => lb.delete(wp as usize),
                    0x188 => lb.cur_sel(),
                    0x189 => {
                        // LB_GETTEXT
                        let Some(s) = lb.items.get(wp as usize).cloned() else { return LB_ERR };
                        rt.mem.write(lp, s.as_bytes());
                        rt.mem.w8(lp + s.len() as u32, 0);
                        s.len() as u32
                    }
                    0x18c => lb.select_string(wp, &lb_text),
                    _ => fatal(&format!("SendDlgItemMessageA: message {msg:#x} to control {id:#x} not emulated")),
                }
            });
            ret(cpu, r, 5);
        }
        "SetWindowTextA" => {
            let (hwnd, text) = (arg(cpu, 0), arg(cpu, 1));
            let t = with_rt(|rt| rt.mem.cstr(text));
            if hwnd != DIALOG_HWND {
                fatal(&format!("SetWindowTextA({hwnd:#x}, {t:?}): only the dialog title is emulated"));
            }
            with_rt(|rt| rt.dialog.as_mut().unwrap_or_else(|| fatal("SetWindowTextA outside a dialog")).title = t);
            ret(cpu, 1, 2);
        }
        "ShowWindow" if arg(cpu, 0) == DIALOG_HWND => ret(cpu, 0, 2), // was hidden; headless
        _ => return false,
    }
    true
}
