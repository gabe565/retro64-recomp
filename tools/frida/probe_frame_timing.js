// Read-only probe: frame timing and game state in Bugatron Gold v1.20a (Bug3D.exe).
//
// Verifies the static findings for FUN_0040b720 (frame delta in 1/1024-s ticks,
// minimum 4, over-512 reuses the previous delta) by sampling its globals after
// each call. Nothing in the target is modified.
//
// Messages: {type:'probe', ...} once at load, then {type:'frames', rows:[...]}
// batches. Each row is [frame, tick_now, delta_ticks, delta_f32, game_state].

'use strict';

const BATCH = 120;          // frames per message
const MAX_FRAMES = 1200;    // stop sampling after this many frames (~20 s at 60 fps)

const mod = Process.getModuleByName('Bug3D.exe');
const base = mod.base;
const IMAGE_BASE = 0x400000;
const va = (addr) => base.add(addr - IMAGE_BASE);

// Guard: refuse to hook unless the prologues match the analysed build.
const EXPECT = {
  0x40b720: '83ec0856c7442404c8000000e89f7e01', // FUN_0040b720 frame timing
  0x40ba70: 'a12cdf820083ec0c5733ff3bc7750d68', // FUN_0040ba70 frame body
};
const hex = (p, n) => Array.from(new Uint8Array(p.readByteArray(n)))
  .map((b) => b.toString(16).padStart(2, '0')).join('');
const mismatches = Object.entries(EXPECT)
  .filter(([a, bytes]) => hex(va(Number(a)), 16) !== bytes)
  .map(([a]) => '0x' + Number(a).toString(16));

send({
  type: 'probe',
  module: mod.name, base: base.toString(), size: mod.size, path: mod.path,
  arch: Process.arch, platform: Process.platform,
  prologue_mismatches: mismatches,
});

if (mismatches.length === 0) {
  const G_TICK_NOW = va(0x8429dc);   // u32 current timer tick
  const G_DELTA = va(0x8429e0);      // u32 frame delta (ticks)
  const G_DELTA_F = va(0x81d67c);    // f32 delta / 1024
  const G_STATE = va(0x4370a0);      // i32 game state

  let frame = 0;
  let rows = [];
  const hook = Interceptor.attach(va(0x40b720), {
    onLeave() {
      rows.push([
        frame++,
        G_TICK_NOW.readU32(),
        G_DELTA.readU32(),
        G_DELTA_F.readFloat(),
        G_STATE.readS32(),
      ]);
      if (rows.length >= BATCH || frame >= MAX_FRAMES) {
        send({ type: 'frames', rows });
        rows = [];
      }
      if (frame >= MAX_FRAMES) {
        hook.detach();
        send({ type: 'done', frames: frame });
      }
    },
  });
}
