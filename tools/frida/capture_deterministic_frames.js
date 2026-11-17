// Deterministic frame capture from the ORIGINAL game, for side-by-side
// comparison with the native build (`bugatron --dump-frames`).
//
// Target: Bugatron Gold v1.20a Bug3D.exe in the Windows VM, SPAWNED under
// Frida (the timer is calibrated at start-up). The script gives the original
// exactly the inputs the native runtime gives the recompiled code, so frame N
// shows the same game state in both and remaining differences are rendering:
//
//   time   QueryPerformanceFrequency = 24 MHz; each QPC read returns now and
//          then advances one tick; timeGetTime = virtual ms; Sleep(n) advances
//          n ms (Sleep(0) none); WaitForVerticalBlank and Flip advance to the
//          next 1/60 s refresh boundary (runtime crate, clock.rs).
//          Only calls from Bug3D.exe are virtualised (FMOD and the DirectDraw
//          driver keep real time). Time is counted in 24 MHz QPC ticks, in which
//          every step is a whole number (a refresh is 400000 ticks, 1 ms 24000),
//          so the counter is exact. QPC and Sleep are compiled C (a CModule): in
//          gameplay the frame limiter calls each about 96,000 times per frame.
//   input  INPUT_SPEC, the native `--input` syntax ("600:click@70x120,
//          800:click@70x120,1000:LEFT+,1045:LEFT-", rel@DXxDY), applied after flip N exactly as the
//          runtime does (crates/runtime/src/user.rs, input_script.rs): async key
//          state ("down", "pressed since last query"), WM_KEYDOWN/UP (SYS for
//          Alt/F10) with the same lParam, WM_MOUSEMOVE and button messages posted
//          to the game window; GetCursorPos returns the virtual cursor, moved by
//          the script and by SetCursorPos (initially 0, 0); joyGetPos = unplugged.
//          Start the game with a click: an injected F1 on the title screen
//          reaches the window procedure but does not start the original
//          (docs/open-questions.md, R10).
//   video  DialogBoxParamA returns IDOK without showing the dialog, so the
//          selection from `prefs` is used (must be 640×480×32 on device 1).
//          GetMonitorFrequency reports 60.
//
// Captures: the back buffer just before flip N (= the native front buffer
// just after flip N), for each N in CAPTURE_FLIPS, written raw to
// CAPTURE_DIR\flip-N.raw (encoding megabytes in the script stalls the game);
// {type:'frame', flip, w, h, pitch, bpp, file} describes each.
//
// Flips are counted at the game's single Flip call site (0x426493, in its
// page-flip wrapper 0x426420; see the hooks below), from process start: r3d_Initialize
// flips twice before any surface exists to hook, and missing those two
// refreshes shifted the virtual clock's phase against the 1/1024 s game timer.

'use strict';

const CAPTURE_FLIPS = [139, 450, 700];
const CAPTURE_DIR = 'C:\\Users\\Public\\bugatron-frames';
// Scripted input (empty: none). Runs replace this line with their script.
const INPUT_SPEC = '';
// Flips at which to report the timer state, to align runs with the native build
// (`BUGATRON_TRACE=delta`): {type:'tick', flip, qpc_reads, delta, tick_now, vt_ms}.
const TRACE_FLIPS = new Set([1, 2, 3, 4, 5, 6, 25, 50, 75, 100, 125, 150, 175, 200]);
// The game opens its files relative to the current directory; Frida's spawn
// does not set it, so set it to the game directory before the game runs.
const GAME_DIR = 'C:\\Program Files (x86)\\Bugatron Gold';
const QPC_FREQ = 24000000n;
const TICKS_PER_REFRESH = 400000n;   // 24 MHz / 60 Hz
const TICKS_PER_MS = 24000n;

const game = Process.getModuleByName('Bug3D.exe');
const inGame = (ra) => ra.compare(game.base) >= 0 && ra.compare(game.base.add(game.size)) < 0;

let flips = 0;
let cursor = [0, 0];

const k32 = Process.getModuleByName('kernel32.dll');
const u32 = Process.getModuleByName('user32.dll');
const winmm = Process.getModuleByName('winmm.dll');
const exp = (m, n) => m.getExportByName(n);

function replaceForGame(mod, name, ret, args, impl) {
  const addr = exp(mod, name);
  const orig = new NativeFunction(addr, ret, args, 'stdcall');
  Interceptor.replace(addr, new NativeCallback(function (...a) {
    return inGame(this.returnAddress) ? impl(...a) : orig(...a);
  }, ret, args, 'stdcall'));
  return orig;
}

replaceForGame(k32, 'QueryPerformanceFrequency', 'int', ['pointer'], (p) => {
  p.writeU64(uint64(QPC_FREQ.toString()));
  return 1;
});

// The hot clock calls, compiled. CModule code and globals are not writable, so
// the mutable state lives in JS-allocated memory passed in as extern symbols:
// t (u64, virtual time in QPC ticks), qpc_reads (u32), and the original QPC and
// Sleep, stored just after each replacement is installed.
const clockState = Memory.alloc(32);
for (let i = 0; i < 32; i += 4) clockState.add(i).writeU32(0);
const clock = new CModule(`
typedef int (__attribute__((__stdcall__)) *qpc_fn)(unsigned long long *p);
typedef void (__attribute__((__stdcall__)) *sleep_fn)(unsigned int ms);
extern unsigned char game_lo[];
extern unsigned char game_hi[];
extern unsigned long long t;
extern unsigned int qpc_reads;
extern qpc_fn real_qpc;
extern sleep_fn real_sleep;

#define FROM_GAME() ((unsigned char *) __builtin_return_address(0) >= game_lo && \\
                     (unsigned char *) __builtin_return_address(0) < game_hi)

/* A call from another thread before the originals are stored (Frida's own
   threads sleep) finds them still null and must not jump there. */
int __attribute__((__stdcall__)) fast_qpc(unsigned long long *p) {
  if (!FROM_GAME()) return real_qpc ? real_qpc(p) : 0;
  *p = t;   /* QueryPerformanceCounter: returns now, then one tick passes (clock.rs) */
  t += 1;
  qpc_reads += 1;
  return 1;
}

/* Sleep(n) advances n ms; Sleep(0) is a pure yield and takes no virtual time. */
void __attribute__((__stdcall__)) fast_sleep(unsigned int ms) {
  if (!FROM_GAME()) { if (real_sleep) real_sleep(ms); return; }
  if (ms != 0) {
    t += (unsigned long long) (ms * 24000u);
    if (real_sleep) real_sleep(ms);
  }
}
`, { game_lo: game.base, game_hi: game.base.add(game.size), t: clockState, qpc_reads: clockState.add(8),
     real_qpc: clockState.add(16), real_sleep: clockState.add(20) });
// __stdcall functions are exported decorated on Win32 (`_fast_qpc@4`).
const cfn = (name) => clock[Object.keys(clock).find((k) => k === name || k.startsWith(`_${name}@`))];
clockState.add(16).writePointer(Interceptor.replaceFast(exp(k32, 'QueryPerformanceCounter'), cfn('fast_qpc')));
clockState.add(20).writePointer(Interceptor.replaceFast(exp(k32, 'Sleep'), cfn('fast_sleep')));
const getT = () => BigInt(clockState.readU64().toString());
const setT = (v) => clockState.writeU64(uint64(v.toString()));
const qpcReads = () => clockState.add(8).readU32();

replaceForGame(winmm, 'timeGetTime', 'uint32', [], () => Number(getT() / TICKS_PER_MS) >>> 0);
// Virtual keyboard: vk -> [down, pressed since the last GetAsyncKeyState].
const keys = new Map();
replaceForGame(u32, 'GetAsyncKeyState', 'int16', ['int'], (vk) => {
  const e = keys.get(vk & 0xff) || [false, false];
  const r = (e[0] ? 0x8000 : 0) | (e[1] ? 1 : 0);
  e[1] = false;
  keys.set(vk & 0xff, e);
  return r;
});
replaceForGame(u32, 'GetCursorPos', 'int', ['pointer'], (p) => { p.writeS32(cursor[0]); p.add(4).writeS32(cursor[1]); return 1; });
replaceForGame(u32, 'SetCursorPos', 'int', ['int', 'int'], (x, y) => { cursor = [x, y]; return 1; });
replaceForGame(winmm, 'joyGetPos', 'uint32', ['uint32', 'pointer'], () => 167);
replaceForGame(u32, 'DialogBoxParamA', 'int', ['pointer', 'pointer', 'pointer', 'pointer', 'pointer'], () => 1);

{
  const set = new NativeFunction(exp(k32, 'SetCurrentDirectoryA'), 'int', ['pointer'], 'stdcall');
  if (!set(Memory.allocAnsiString(GAME_DIR))) send({ type: 'error', what: 'SetCurrentDirectoryA', dir: GAME_DIR });
}

const vblank = () => { setT((getT() / TICKS_PER_REFRESH + 1n) * TICKS_PER_REFRESH); };
const vtMs = () => Number(getT() / TICKS_PER_MS);

function captureBackBuffer(flip) {
  const back = ptr(0xa06b58).readPointer();
  const vt = back.readPointer();
  const lock = new NativeFunction(vt.add(25 * 4).readPointer(), 'int', ['pointer', 'pointer', 'pointer', 'uint32', 'pointer'], 'stdcall');
  const unlock = new NativeFunction(vt.add(32 * 4).readPointer(), 'int', ['pointer', 'pointer'], 'stdcall');
  const desc = Memory.alloc(0x7c);
  desc.writeU32(0x7c);
  const hr = lock(back, NULL, desc, 1, NULL); // DDLOCK_WAIT
  if (hr !== 0) { send({ type: 'error', flip, hr }); return; }
  const h = desc.add(8).readU32(), w = desc.add(12).readU32(), pitch = desc.add(16).readU32();
  const bits = desc.add(0x24).readPointer(), bpp = desc.add(0x54).readU32();
  const bytes = bits.readByteArray(pitch * h);
  unlock(back, NULL);
  const file = `${CAPTURE_DIR}\\flip-${flip}.raw`;
  File.writeAllBytes(file, bytes);
  send({ type: 'frame', flip, w, h, pitch, bpp, vt_ms: vtMs(), file });
}

// After r3d_Initialize (0x424b90): hook WaitForVerticalBlank, GetMonitorFrequency and Flip.
let hooked = false;
Interceptor.attach(ptr(0x424b90), {
  onLeave() {
    if (hooked) return;
    hooked = true;
    const dd4 = ptr(0xa06afc).readPointer();
    const ddvt = dd4.readPointer();
    Interceptor.attach(ddvt.add(22 * 4).readPointer(), { onLeave() { if (inGame(this.returnAddress)) vblank(); } });
    Interceptor.replace(ddvt.add(15 * 4).readPointer(), new NativeCallback((self, p) => { p.writeU32(60); return 0; }, 'int', ['pointer', 'pointer'], 'stdcall'));
    send({ type: 'hooked', vt_ms: vtMs() });
  },
});
// The game's only Flip call is IDirectDrawSurface4::Flip(front, NULL, DDFLIP_WAIT)
// at 0x426493, a 3-byte instruction Frida cannot patch. So: capture at the entry
// of its page-flip wrapper 0x426420 (the back buffer holds the frame about to be
// shown), and count the flip and advance the clock at 0x426496, the instruction
// after Flip returns.
Interceptor.attach(ptr(0x426420), function () {
  if (CAPTURE_FLIPS.includes(flips + 1)) captureBackBuffer(flips + 1);
});
Interceptor.attach(ptr(0x426496), function () {
  flips += 1;
  vblank();
  applyInput(flips);
  // Reported after the flip, as the native trace is (BUGATRON_TRACE=delta).
  if (TRACE_FLIPS.has(flips)) {
    send({ type: 'tick', flip: flips, qpc_reads: qpcReads(), delta: ptr(0x8429e0).readU32(), tick_now: ptr(0x8429dc).readU32(), vt_ms: vtMs() });
  }
});

// ---- scripted input (mirrors the runtime's input_script / key_input_rt) ----
// name -> [vk, scan, side vk]
function keyCode(name) {
  const n = name.toUpperCase();
  const letters = [0x1e, 0x30, 0x2e, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24, 0x25, 0x26, 0x32, 0x31, 0x18, 0x19, 0x10, 0x13, 0x1f, 0x14, 0x16, 0x2f, 0x11, 0x2d, 0x15, 0x2c];
  if (/^[A-Z]$/.test(n)) return [n.charCodeAt(0), letters[n.charCodeAt(0) - 65], 0];
  if (n === '0') return [0x30, 0x0b, 0];
  if (/^[1-9]$/.test(n)) return [0x30 + Number(n), 0x01 + Number(n), 0];
  const f = /^F([1-9]|1[0-2])$/.exec(n);
  if (f) { const k = Number(f[1]); return [0x6f + k, k <= 10 ? 0x3a + k : 0x4c + k, 0]; }
  const table = { ENTER: [0x0d, 0x1c, 0], ESC: [0x1b, 0x01, 0], TAB: [0x09, 0x0f, 0], SPACE: [0x20, 0x39, 0],
    HOME: [0x24, 0x147, 0], PGUP: [0x21, 0x149, 0], END: [0x23, 0x14f, 0], PGDN: [0x22, 0x151, 0],
    RIGHT: [0x27, 0x14d, 0], LEFT: [0x25, 0x14b, 0], DOWN: [0x28, 0x150, 0], UP: [0x26, 0x148, 0],
    CTRL: [0x11, 0x1d, 0xa2], SHIFT: [0x10, 0x2a, 0xa0], ALT: [0x12, 0x38, 0xa4] };
  if (n in table) return table[n];
  throw new Error(`unknown key ${name}`);
}

// flip -> list of events
const events = new Map();
function addEvent(flip, ev) { if (!events.has(flip)) events.set(flip, []); events.get(flip).push(ev); }
for (const item of INPUT_SPEC.split(',').map((x) => x.trim()).filter((x) => x)) {
  const [frame, key] = [item.slice(0, item.indexOf(':')), item.slice(item.indexOf(':') + 1)];
  const f = Number(frame);
  const pt = (s) => s.split('x').map(Number);
  if (key === 'QUIT') addEvent(f, { quit: true });
  else if (key.startsWith('rel@')) addEvent(f, { rel: pt(key.slice(4)) });
  else if (key.startsWith('move@')) addEvent(f, { move: pt(key.slice(5)) });
  else if (key.startsWith('click@')) { const p = pt(key.slice(6)); addEvent(f, { move: p }); addEvent(f, { button: true, p }); addEvent(f + 2, { button: false, p }); }
  else if (key.endsWith('+')) addEvent(f, { key: keyCode(key.slice(0, -1)), down: true });
  else if (key.endsWith('-')) addEvent(f, { key: keyCode(key.slice(0, -1)), down: false });
  else { addEvent(f, { key: keyCode(key), down: true }); addEvent(f + 2, { key: keyCode(key), down: false }); }
}

// The game's main window, as its WM_CREATE handler stores it (0x407d41).
const gameWindow = () => ptr(0x832034).readPointer();
const postMessage = new NativeFunction(exp(u32, 'PostMessageA'), 'int', ['pointer', 'uint32', 'uint32', 'uint32'], 'stdcall');
const mkLparam = (x, y) => ((x & 0xffff) | ((y & 0xffff) << 16)) >>> 0;
let cursorFrac = [0, 0];

function applyInput(flip) {
  for (const ev of events.get(flip) || []) {
    if (ev.key) {
      const [vk, scan, side] = ev.key;
      for (const v of side ? [vk, side] : [vk]) {
        const e = keys.get(v) || [false, false];
        if (ev.down && !e[0]) e[1] = true;
        e[0] = ev.down;
        keys.set(v, e);
      }
      const sys = vk === 0x79 || vk === 0x12;
      const msg = ev.down ? (sys ? 0x104 : 0x100) : (sys ? 0x105 : 0x101);
      let lp = 1 | ((scan & 0xff) << 16) | (((scan >> 8) & 1) << 24);
      if (!ev.down) lp |= (1 << 30) | (1 << 31);
      postMessage(gameWindow(), msg, vk, lp >>> 0);
    } else if (ev.move) {
      cursor = ev.move;
      postMessage(gameWindow(), 0x200, 0, mkLparam(ev.move[0], ev.move[1]));
    } else if (ev.rel) {
      const fx = cursorFrac[0] + ev.rel[0], fy = cursorFrac[1] + ev.rel[1];
      const ix = Math.trunc(fx), iy = Math.trunc(fy);
      cursorFrac = [fx - ix, fy - iy];
      if (ix !== 0 || iy !== 0) {
        cursor = [Math.min(Math.max(cursor[0] + ix, 0), 639), Math.min(Math.max(cursor[1] + iy, 0), 479)];
        postMessage(gameWindow(), 0x200, 0, mkLparam(cursor[0], cursor[1]));
      }
    } else if (ev.button !== undefined) {
      cursor = ev.p;
      const e = keys.get(0x01) || [false, false];
      if (ev.button && !e[0]) e[1] = true;
      e[0] = ev.button;
      keys.set(0x01, e);
      postMessage(gameWindow(), ev.button ? 0x201 : 0x202, ev.button ? 1 : 0, mkLparam(ev.p[0], ev.p[1]));
    } else if (ev.quit) {
      postMessage(gameWindow(), 0x112, 0xf060, 0); // WM_SYSCOMMAND(SC_CLOSE)
    }
  }
}

{
  const mk = new NativeFunction(exp(k32, 'CreateDirectoryA'), 'int', ['pointer', 'pointer'], 'stdcall');
  mk(Memory.allocAnsiString(CAPTURE_DIR), NULL);
}

send({ type: 'ready', base: game.base.toString() });
