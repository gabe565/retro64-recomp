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
// just after flip N), for each N in CAPTURE_FLIPS, sent as
// {type:'frame', flip, w, h, pitch, bpp, b64} with the raw guest pixels.

'use strict';

const CAPTURE_FLIPS = [139, 450, 700];
const QPC_FREQ = 24000000n;
const NS = 1000000000n;
const UNITS = 60n;                 // virtual time unit: 1/60 ns; one refresh = NS units
const REFRESH = NS;
const QPC_TICK = (NS * UNITS + QPC_FREQ - 1n) / QPC_FREQ;

const game = Process.getModuleByName('Bug3D.exe');
const inGame = (ra) => ra.compare(game.base) >= 0 && ra.compare(game.base.add(game.size)) < 0;

let t = 0n;        // virtual time
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
replaceForGame(k32, 'QueryPerformanceCounter', 'int', ['pointer'], (p) => {
  const ticks = t * QPC_FREQ / (NS * UNITS);
  p.writeU64(uint64(ticks.toString()));
  t += QPC_TICK;
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

const vblank = () => { t = (t / REFRESH + 1n) * REFRESH; };

function b64(bytes) {
  const A = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  let out = '';
  for (let i = 0; i < bytes.length; i += 3) {
    const n = (bytes[i] << 16) | ((bytes[i + 1] || 0) << 8) | (bytes[i + 2] || 0);
    out += A[(n >> 18) & 63] + A[(n >> 12) & 63] + (i + 1 < bytes.length ? A[(n >> 6) & 63] : '=') + (i + 2 < bytes.length ? A[n & 63] : '=');
  }
  return out;
}

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
  const bytes = new Uint8Array(bits.readByteArray(pitch * h));
  unlock(back, NULL);
  send({ type: 'frame', flip, w, h, pitch, bpp, vt_ms: Number(t * 1000n / (NS * UNITS)), b64: b64(bytes) });
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
    const front = ptr(0xa06b44).readPointer();
    Interceptor.attach(front.readPointer().add(11 * 4).readPointer(), {
      onEnter() {
        if (!inGame(this.returnAddress)) return;
        flips += 1;
        if (CAPTURE_FLIPS.includes(flips)) captureBackBuffer(flips);
      },
      onLeave() { if (inGame(this.returnAddress)) vblank(); },
    });
    send({ type: 'hooked', vt_ms: Number(t * 1000n / (NS * UNITS)) });
  },
});
send({ type: 'ready', base: game.base.toString() });
