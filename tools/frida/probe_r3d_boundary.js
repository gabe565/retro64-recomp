// Read-only probe: validate the r3d graphics-wrapper boundary at runtime.
// Target: Bugatron Gold v1.20a Bug3D.exe (base 0x400000).
//
// Hooks every function in the r3d range (0x41e950-0x42a3ff) and records, per
// function and per running config (menu, level files, high scores):
//   - call counts, split by caller region (return address in game / r3d / CRT),
//   - stack-cleanup check: Frida reports ESP in onLeave after its trampoline has
//     consumed the return address, so a stdcall function that pops N bytes shows
//     a delta of exactly N (calibrated 2026-11-15 on purge-0 functions; see
//     docs/graphics/r3d-interface.md §8). N is the purge recovered statically by
//     tools/ghidra/R3dSurvey.java,
//   - the first argument tuple seen from game code.
// Also counts the game-side direct COM call sites identified statically.
// Summaries are sent every 300 frames. Nothing in the target is modified.

'use strict';

const ENTRIES = [
  [0x41e950, 8],
  [0x41ebe0, 64],
  [0x41ed70, 8],
  [0x41f090, 0],
  [0x41f1e0, 0],
  [0x41f2b0, 0],
  [0x41f4c0, 8],
  [0x41ff20, 8],
  [0x420ed0, 4],
  [0x420f90, 4],
  [0x4210d0, 4],
  [0x421430, 32],
  [0x421600, 8],
  [0x421640, 4],
  [0x421680, 20],
  [0x421a30, 24],
  [0x421c20, 12],
  [0x421cc0, 4],
  [0x421f60, 4],
  [0x421fa0, 4],
  [0x421fd0, 28],
  [0x4220e0, 12],
  [0x422140, 0],
  [0x422160, 4],
  [0x4221e0, 12],
  [0x4222b0, 20],
  [0x422430, 16],
  [0x422750, 8],
  [0x4229a0, 8],
  [0x422bb0, 4],
  [0x422be0, 12],
  [0x422cb0, 0],
  [0x422d90, 4],
  [0x422dd0, 4],
  [0x422e10, 4],
  [0x4231b0, 8],
  [0x4235d0, 0],
  [0x4235e0, 0],
  [0x4235f0, 0],
  [0x423600, 0],
  [0x423610, 4],
  [0x423650, 4],
  [0x423b30, 0],
  [0x423b50, 4],
  [0x423bd0, 4],
  [0x424400, 12],
  [0x424430, 0],
  [0x424440, 0],
  [0x424500, 4],
  [0x424520, 0],
  [0x4245f0, 28],
  [0x424670, 8],
  [0x4246b0, 8],
  [0x4246f0, 0],
  [0x424730, 12],
  [0x424a10, 28],
  [0x424b90, 24],
  [0x425660, 4],
  [0x425690, 8],
  [0x425780, 0],
  [0x426200, 32],
  [0x426250, 8],
  [0x4262b0, 32],
  [0x426420, 8],
  [0x4264c0, 0],
  [0x4265b0, 0],
  [0x426650, 0],
  [0x4266e0, 68],
  [0x427480, 4],
  [0x4274b0, 4],
  [0x427700, 24],
  [0x4279c0, 8],
  [0x427a10, 12],
  [0x427ab0, 28],
  [0x427bd0, 20],
  [0x427d20, 4],
  [0x427db0, 52],
  [0x4288b0, 4],
  [0x4288c0, 8],
  [0x428950, 12],
  [0x428980, 12],
  [0x4289b0, 4],
  [0x4289d0, 12],
  [0x428a10, 0],
  [0x428a90, 12],
  [0x428b10, 20],
  [0x428ee0, 8],
  [0x4290d0, 0],
  [0x4291b0, 8],
  [0x429ef0, 8]
];
const COM_SITES = { 0x409e75: 'Device3::DrawPrimitive', 0x40b739: 'DDraw4::GetMonitorFrequency',
  0x40b893: 'BackBuffer::Blt', 0x40b8cd: 'BackBuffer::Blt', 0x40ba1a: 'BackBuffer::Blt',
  0x40baa1: 'FrontBuffer::IsLost', 0x41c1e7: 'DDraw4::WaitForVerticalBlank' };

const region = (a) => (a >= 0x41e950 && a < 0x42a400) ? 'r3d' : (a >= 0x401000 && a < 0x41e950) ? 'game' : 'crt';
const G_CONFIG = ptr(0x81d548);
const stats = {};
let frame = 0;

function bucket() {
  let cfg = '';
  try { cfg = G_CONFIG.readCString(); } catch (e) { cfg = '?'; }
  return (stats[cfg] = stats[cfg] || {});
}

for (const [addr, purge] of ENTRIES) {
  const key = '0x' + addr.toString(16);
  Interceptor.attach(ptr(addr), {
    onEnter(args) {
      this.esp0 = this.context.esp;
      this.ret = this.context.esp.readU32();
      this.from = region(this.ret);
      this.b = bucket();
      const s = (this.b[key] = this.b[key] || { game: 0, r3d: 0, crt: 0, purge_mismatch: 0, purge, sample: null, deltas: {} });
      s[this.from]++;
      if (this.from === 'game' && s.sample === null) {
        const n = Math.min(purge / 4, 8);
        const a = [];
        for (let i = 0; i < n; i++) a.push('0x' + this.context.esp.add(4 + 4 * i).readU32().toString(16));
        s.sample = { caller: '0x' + this.ret.toString(16), args: a };
      }
    },
    onLeave() {
      const delta = this.context.esp.sub(this.esp0).toInt32();
      const d = this.b[key].deltas;
      d[delta] = (d[delta] || 0) + 1;
      if (delta !== purge) this.b[key].purge_mismatch++;
    },
  });
}

// COM call sites are 3-byte `call [reg+disp8]` instructions that Frida cannot
// always relocate; hook where possible. The accessors that produce the COM
// objects (0x4235d0..0x423600) are hooked above with caller attribution either way.
const comHooked = [], comSkipped = [];
for (const [site, name] of Object.entries(COM_SITES)) {
  const tag = name + '@0x' + Number(site).toString(16);
  try {
    Interceptor.attach(ptr(Number(site)), {
      onEnter() {
        const b = bucket();
        b['com:' + tag] = (b['com:' + tag] || 0) + 1;
      },
    });
    comHooked.push(tag);
  } catch (e) {
    comSkipped.push(tag);
  }
}

Interceptor.attach(ptr(0x40b720), {
  onLeave() {
    frame++;
    if (frame % 300 === 0) send({ type: 'r3d_boundary', frame, stats });
    if (globalThis.MAX_FRAMES && frame >= globalThis.MAX_FRAMES) send({ type: 'done', frame });
  },
});
send({ type: 'hello', hooks: ENTRIES.length, com_hooked: comHooked, com_skipped: comSkipped });
