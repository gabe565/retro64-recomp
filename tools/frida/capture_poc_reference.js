// Reference capture for the static-recompilation proof of concept.
// Target: Bugatron Gold v1.20a Bug3D.exe (image base 0x400000, verified by prologue checks).
//
// Everything that executes original code runs on the GAME THREAD, inside an
// Interceptor callback on FUN_0040b720, because the MSVC 6 CRT is single-threaded
// and the x87 control word is per-thread.
//
// Phases (messages carry {type: ...}):
//   'hello'      module/base/prologue verification
//   'x87_probe'  control word at hook time + Prism precision-control tests
//   'rand'       original srand/_rand outputs for fixed seeds (thread RNG state restored)
//   'lev'        original FUN_004076a0 output (length, CRC-32) for each .lev file
//   'timing'     raw QueryPerformanceCounter inputs and FUN_0040b720 outputs per frame
//   'cw'         x87 control-word histogram by sampling point (sent periodically)
//
// Side effects on the game: brief stalls while the decoder reference runs; the
// game's RNG state is saved and restored. No game memory other than CRT heap
// churn (malloc/free of our own buffers) is changed.

'use strict';

const LEV_FILES = [
  'BUG1.XML.lev', 'BUG10.XML.lev', 'BUG11.XML.lev', 'BUG12.XML.lev', 'BUG13.XML.lev',
  'bug14.xml.lev', 'bug15.xml.lev', 'bug16.xml.lev', 'bug17.xml.lev', 'bug18.xml.lev',
  'bug19.xml.lev', 'BUG2.XML.lev', 'bug20.xml.lev', 'bug21.xml.lev', 'BUG3.XML.lev',
  'BUG4.XML.lev', 'BUG5.XML.lev', 'BUG6.XML.lev', 'BUG7.XML.lev', 'BUG8.XML.lev',
  'BUG9.XML.lev', 'config.xml.lev', 'CONFIG2.XML.lev', 'highscores.xml.lev',
  'PACK1.XML.lev', 'PACK2.XML.lev', 'PACK3.XML.lev', 'pack4.xml.lev', 'pack5.xml.lev',
  'pack6.xml.lev',
];
const RAND_SEEDS = [0, 1, 12345, 0xdeadbeef];
const TIMING_FRAMES = 600;

const EXPECT = {
  0x40b720: '83ec0856c7442404c8000000e89f7e01',
  0x424440: '83ec088d4424005650ff152c504300a1',
  0x4076a0: '568b7424085756e86457010050e86472',
  0x42ab66: 'e8182a00008b4c2404894814c3e80b2a',
  0x42ab73: 'e80b2a00008b481469c9fd43030081c1',
  0x42d583: '5657ff1504514300ff35b0b943008bf8',
  0x42a6f8: '568b74240885f6743d6a09e8f8190000',
  0x40df90: '83ec0c538b5c2418558bab9c08000056',
};
const hex = (p, n) => Array.from(new Uint8Array(p.readByteArray(n)))
  .map((b) => b.toString(16).padStart(2, '0')).join('');
const mod = Process.getModuleByName('Bug3D.exe');
const mismatches = Object.entries(EXPECT)
  .filter(([a, b]) => hex(ptr(Number(a)), 16) !== b).map(([a]) => '0x' + Number(a).toString(16));
send({ type: 'hello', base: mod.base.toString(), path: mod.path, arch: Process.arch, mismatches });
if (mismatches.length) throw new Error('prologue mismatch; refusing to run');

// ---- helpers -------------------------------------------------------------
const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = (c & 1) ? (0xedb88320 ^ (c >>> 1)) : (c >>> 1);
    t[n] = c >>> 0;
  }
  return t;
})();
function crc32(u8) {
  let c = 0xffffffff;
  for (let i = 0; i < u8.length; i++) c = CRC_TABLE[(c ^ u8[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

// Hand-assembled x86 helpers (see docs/recompilation-feasibility.md for the listing).
const code = Memory.alloc(Process.pageSize);
const GETCW = [0x83, 0xEC, 0x04, 0xD9, 0x3C, 0x24, 0x0F, 0xB7, 0x04, 0x24, 0x83, 0xC4, 0x04, 0xC3];
const PRISM = [
  0x8B, 0x4C, 0x24, 0x04,             // mov ecx,[esp+4]   out
  0x8B, 0x54, 0x24, 0x08,             // mov edx,[esp+8]   consts
  0xD9, 0x7A, 0x16,                   // fnstcw [edx+0x16] save
  0xDF, 0x2A, 0xD9, 0xE8, 0xDE, 0xC1, 0xDF, 0x39,              // 2^53+1 at current CW -> [ecx]
  0xD9, 0x6A, 0x10, 0xDF, 0x2A, 0xD9, 0xE8, 0xDE, 0xC1, 0xDF, 0x79, 0x08, // PC64 -> [ecx+8]
  0xD9, 0x6A, 0x12, 0xDF, 0x2A, 0xD9, 0xE8, 0xDE, 0xC1, 0xDF, 0x79, 0x10, // PC53 -> [ecx+16]
  0xD9, 0x6A, 0x14, 0xD9, 0xE8, 0xDC, 0x72, 0x08, 0xDD, 0x59, 0x18,       // PC24 1/3 -> [ecx+24]
  0xD9, 0x6A, 0x12, 0xD9, 0xE8, 0xDC, 0x72, 0x08, 0xDD, 0x59, 0x20,       // PC53 1/3 -> [ecx+32]
  0xD9, 0x6A, 0x10, 0xD9, 0xE8, 0xDC, 0x72, 0x08, 0xDD, 0x59, 0x28,       // PC64 1/3 -> [ecx+40]
  0xD9, 0x6A, 0x16,                   // fldcw [edx+0x16] restore
  0xC3,
];
Memory.patchCode(code, 256, (p) => {
  p.writeByteArray(GETCW);
  p.add(32).writeByteArray(PRISM);
});
const getcw = new NativeFunction(code, 'uint32', [], 'mscdecl');
const prismTest = new NativeFunction(code.add(32), 'void', ['pointer', 'pointer'], 'mscdecl');

const crtMalloc = new NativeFunction(ptr(0x42ab95), 'pointer', ['uint32'], 'mscdecl');
const crtFree = new NativeFunction(ptr(0x42a6f8), 'void', ['pointer'], 'mscdecl');
const levDecode = new NativeFunction(ptr(0x4076a0), 'pointer', ['pointer', 'pointer'], 'mscdecl');
const crtSrand = new NativeFunction(ptr(0x42ab66), 'void', ['uint32'], 'mscdecl');
const crtRand = new NativeFunction(ptr(0x42ab73), 'int', [], 'mscdecl');
const getptd = new NativeFunction(ptr(0x42d583), 'pointer', [], 'mscdecl');

// ---- control-word histogram ----------------------------------------------
const cwHist = {};
function sampleCw(where) {
  const k = where + ':0x' + getcw().toString(16);
  cwHist[k] = (cwHist[k] || 0) + 1;
}
Interceptor.attach(ptr(0x424440), { onEnter() { sampleCw('timer_0x424440'); } });
Interceptor.attach(ptr(0x40df90), { onEnter() { sampleCw('alien_0x40df90'); } });

// ---- phases run on the game thread from FUN_0040b720 ------------------------
let phase = 0;
let frame = 0;
let inTiming = false;
let gameThread = -1;   // QPC calls from other threads (FMOD/DirectSound) are ignored
let qpcLog = [];
let timingRows = [];

function runX87Probe() {
  const consts = Memory.alloc(32);
  consts.writeS64(int64('9007199254740992'));     // 2^53
  consts.add(8).writeDouble(3.0);
  consts.add(0x10).writeU16(0x037f);                // PC=64, RC=nearest, masked
  consts.add(0x12).writeU16(0x027f);                // PC=53
  consts.add(0x14).writeU16(0x007f);                // PC=24
  const out = Memory.alloc(48);
  const cwBefore = getcw();
  prismTest(out, consts);
  send({
    type: 'x87_probe',
    cw_at_frame_hook: '0x' + cwBefore.toString(16),
    sum_2p53_plus_1_current_cw: out.readS64().toString(),
    sum_2p53_plus_1_pc64: out.add(8).readS64().toString(),
    sum_2p53_plus_1_pc53: out.add(16).readS64().toString(),
    third_pc24_bits: hex(out.add(24), 8),
    third_pc53_bits: hex(out.add(32), 8),
    third_pc64_bits: hex(out.add(40), 8),
    cw_after: '0x' + getcw().toString(16),
  });
}

function runRand() {
  const ptd = getptd();
  const saved = ptd.add(0x14).readU32();
  const results = [];
  for (const seed of RAND_SEEDS) {
    crtSrand(seed >>> 0);
    const first = [];
    let acc = 0;
    for (let i = 0; i < 10000; i++) {
      const v = crtRand();
      if (i < 16) first.push(v);
      acc = (Math.imul(acc, 31) + v) >>> 0;
    }
    results.push({ seed: seed >>> 0, first16: first, hash10000: acc, state_after: ptd.add(0x14).readU32() });
  }
  ptd.add(0x14).writeU32(saved);
  send({ type: 'rand', ptd: ptd.toString(), saved_state: saved, results });
}

function runLev() {
  const dir = mod.path.substring(0, mod.path.lastIndexOf('\\') + 1);
  for (const name of LEV_FILES) {
    let row = { type: 'lev', name };
    try {
      const raw = new Uint8Array(File.readAllBytes(dir + name));
      const buf = crtMalloc(raw.length);
      const inv = new Uint8Array(raw.length);
      for (let i = 0; i < raw.length; i++) inv[i] = (~raw[i]) & 0xff;
      buf.writeByteArray(inv.buffer);
      const plen = Memory.alloc(4);
      const out = levDecode(buf, plen);            // frees buf
      const n = plen.readU32();
      const bytes = new Uint8Array(out.readByteArray(n));
      row = { ...row, in_len: raw.length, out_len: n, crc32: crc32(bytes) };
      crtFree(out);
    } catch (e) {
      row.error = String(e);
    }
    send(row);
  }
}

const TIMER_GLOBALS = {
  qpc_base_lo: 0x9fa928, qpc_base_hi: 0x9fa92c, tick_base: 0x9f679c, tick_now_r3d: 0xa02ad0,
  tick_saved: 0xa02a4c, rebase_flag: 0x9f5788, freq_lo: 0xa06b48, freq_hi: 0xa06b4c,
  tick_now: 0x8429dc, delta: 0x8429e0, tick_prev: 0x8429e4, delta_copy: 0x8429e8,
};
function snapGlobals() {
  const o = {};
  for (const [k, a] of Object.entries(TIMER_GLOBALS)) o[k] = ptr(a).readU32();
  o.delta_f32_bits = ptr(0x81d67c).readU32();
  return o;
}

const qpcExport = Module.getGlobalExportByName('QueryPerformanceCounter');
Interceptor.attach(qpcExport, {
  onEnter(args) { this.p = (inTiming && this.threadId === gameThread) ? args[0] : null; },
  onLeave() {
    if (this.p) qpcLog.push([this.p.readU32(), this.p.add(4).readU32()]);
  },
});

Interceptor.attach(ptr(0x40b720), {
  onEnter() {
    gameThread = this.threadId;
    sampleCw('frame_0x40b720');
    if (phase === 0) {
      phase = 1;
      runX87Probe();
      runRand();
      runLev();
      send({ type: 'phase', done: 'x87_rand_lev' });
      return;
    }
    if (phase === 1 && frame < TIMING_FRAMES) {
      this.before = snapGlobals();
      inTiming = true;
      qpcLog = [];
    }
  },
  onLeave() {
    if (phase !== 1 || !this.before) return;
    inTiming = false;
    timingRows.push({ frame, before: this.before, qpc: qpcLog, after: snapGlobals() });
    frame++;
    if (timingRows.length >= 25 || frame >= TIMING_FRAMES) {
      send({ type: 'timing', rows: timingRows });
      timingRows = [];
    }
    if (frame % 120 === 0 || frame >= TIMING_FRAMES) send({ type: 'cw', frame, hist: cwHist });
    if (frame >= TIMING_FRAMES) { phase = 2; send({ type: 'phase', done: 'timing' }); }
  },
});
