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
//          driver keep real time).
//   input  GetAsyncKeyState = 0; GetCursorPos returns the last SetCursorPos
//          (initially 0, 0); joyGetPos = JOYERR_UNPLUGGED.
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
// Flips at which to report the timer state, to align runs with the native build
// (`BUGATRON_TRACE=delta`): {type:'tick', flip, qpc_reads, delta, tick_now, vt_ms}.
const TRACE_FLIPS = new Set([1, 2, 3, 4, 5, 6, 25, 50, 75, 100, 125, 150, 175, 200]);
// The game opens its files relative to the current directory; Frida's spawn
// does not set it, so set it to the game directory before the game runs.
const GAME_DIR = 'C:\\Program Files (x86)\\Bugatron Gold';
const QPC_FREQ = 24000000n;
const NS = 1000000000n;
const UNITS = 60n;                 // virtual time unit: 1/60 ns; one refresh = NS units
const REFRESH = NS;
const QPC_TICK = (NS * UNITS + QPC_FREQ - 1n) / QPC_FREQ;

const game = Process.getModuleByName('Bug3D.exe');
const inGame = (ra) => ra.compare(game.base) >= 0 && ra.compare(game.base.add(game.size)) < 0;

let t = 0n;        // virtual time
let flips = 0;
let qpcReads = 0;
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
replaceForGame(k32, 'QueryPerformanceCounter', 'int', ['pointer'], (p) => {
  const ticks = t * QPC_FREQ / (NS * UNITS);
  p.writeU64(uint64(ticks.toString()));
  t += QPC_TICK;
  qpcReads += 1;
  return 1;
});
replaceForGame(winmm, 'timeGetTime', 'uint32', [], () => Number(t * 1000n / (NS * UNITS)) >>> 0);
const realSleep = replaceForGame(k32, 'Sleep', 'void', ['uint32'], (ms) => {
  t += BigInt(ms) * 1000000n * UNITS;
  if (ms > 0) realSleep(ms);
});
replaceForGame(u32, 'GetAsyncKeyState', 'int16', ['int'], () => 0);
replaceForGame(u32, 'GetCursorPos', 'int', ['pointer'], (p) => { p.writeS32(cursor[0]); p.add(4).writeS32(cursor[1]); return 1; });
replaceForGame(u32, 'SetCursorPos', 'int', ['int', 'int'], (x, y) => { cursor = [x, y]; return 1; });
replaceForGame(winmm, 'joyGetPos', 'uint32', ['uint32', 'pointer'], () => 167);
replaceForGame(u32, 'DialogBoxParamA', 'int', ['pointer', 'pointer', 'pointer', 'pointer', 'pointer'], () => 1);

{
  const set = new NativeFunction(exp(k32, 'SetCurrentDirectoryA'), 'int', ['pointer'], 'stdcall');
  if (!set(Memory.allocAnsiString(GAME_DIR))) send({ type: 'error', what: 'SetCurrentDirectoryA', dir: GAME_DIR });
}

const vblank = () => { t = (t / REFRESH + 1n) * REFRESH; };

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
  send({ type: 'frame', flip, w, h, pitch, bpp, vt_ms: Number(t * 1000n / (NS * UNITS)), file });
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
    send({ type: 'hooked', vt_ms: Number(t * 1000n / (NS * UNITS)) });
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
  // Reported after the flip, as the native trace is (BUGATRON_TRACE=delta).
  if (TRACE_FLIPS.has(flips)) {
    send({ type: 'tick', flip: flips, qpc_reads: qpcReads, delta: ptr(0x8429e0).readU32(), tick_now: ptr(0x8429dc).readU32(), vt_ms: Number(t * 1000n / (NS * UNITS)) });
  }
});

{
  const mk = new NativeFunction(exp(k32, 'CreateDirectoryA'), 'int', ['pointer', 'pointer'], 'stdcall');
  mk(Memory.allocAnsiString(CAPTURE_DIR), NULL);
}

send({ type: 'ready', base: game.base.toString() });
