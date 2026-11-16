// Capture the ORIGINAL game's final FMOD mix (16-bit stereo) from inside FMOD
// 3.61, for comparison with the native build (`bugatron --record-audio`).
//
// After FSOUND_Init, a DSP unit is added with priority 1100, above FMOD's
// clip-and-copy unit (1000), so it receives the final 16-bit stereo buffer.
//
// Safety rules learned the hard way (2026-11-15, the VM's audio and the game
// hung; see docs/audio.md):
// - DSP callbacks are cdecl in FMOD 3.61 (its own units end in a plain `ret`);
//   a stdcall callback corrupts the mixer thread's stack and crashes the game.
// - The callback runs on FMOD's real-time mixer thread: it only does one
//   native memcpy into a preallocated buffer. No JS arrays, no encoding, no
//   send() there; otherwise mixing stalls and DirectSound loops its buffer.
// - Encoding and sending happen later on a timer, in chunks.
// Spawn the game with only a dialog auto-OK hook: the timeline/clock script
// (capture_deterministic_frames.js) made FSOUND_Init fail.

'use strict';

const RECORD_SECONDS = 12;
const CHUNK_BYTES = 256 * 1024;
let rate = 44100, recording = false, done = false, filled = 0, capacity = 0, store = null;

// Callback: copy only, while there is room. Allocation happens before recording starts.
const tap = new NativeCallback((orig, buf, length, param) => {
  if (recording && !done) {
    const n = Math.min(length * 4, capacity - filled);
    if (n > 0) {
      Memory.copy(store.add(filled), buf, n);
      filled += n;
    }
    if (filled >= capacity) done = true;
  }
  return buf;
}, 'pointer', ['pointer', 'pointer', 'int', 'int'], 'mscdecl');

function b64(u8) {
  const A = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  const parts = [];
  for (let i = 0; i < u8.length; i += 3) {
    const n = (u8[i] << 16) | ((u8[i + 1] || 0) << 8) | (u8[i + 2] || 0);
    parts.push(A[(n >> 18) & 63] + A[(n >> 12) & 63] + (i + 1 < u8.length ? A[(n >> 6) & 63] : '=') + (i + 2 < u8.length ? A[n & 63] : '='));
  }
  return parts.join('');
}

// Off the audio thread: once full, send in chunks, one per timer tick.
let sentBytes = 0;
const pump = setInterval(() => {
  if (!done) return;
  if (sentBytes >= capacity) {
    send({ type: 'audio-end', rate, bytes: capacity });
    clearInterval(pump);
    return;
  }
  const n = Math.min(CHUNK_BYTES, capacity - sentBytes);
  send({ type: 'audio', rate, offset: sentBytes, b64: b64(new Uint8Array(store.add(sentBytes).readByteArray(n))) });
  sentBytes += n;
}, 200);

let setUp = false;
Interceptor.attach(ptr(0x4172b0), { onEnter() {
  if (setUp) return;
  setUp = true;
  const fmod = Process.getModuleByName('fmod.dll');
  const e = (n) => fmod.getExportByName(n);
  const dspCreate = new NativeFunction(e('_FSOUND_DSP_Create@12'), 'pointer', ['pointer', 'int', 'int'], 'stdcall');
  const dspActive = new NativeFunction(e('_FSOUND_DSP_SetActive@8'), 'void', ['pointer', 'int'], 'stdcall');
  Interceptor.attach(e('_FSOUND_Init@12'), {
    onEnter(args) { rate = args[0].toInt32(); },
    onLeave(ret) {
      if ((ret.toInt32() & 0xff) === 0) { send({ type: 'init-failed', rate }); return; }
      capacity = RECORD_SECONDS * rate * 4;
      store = Memory.alloc(capacity);
      const unit = dspCreate(tap, 1100, 0);
      dspActive(unit, 1);
      send({ type: 'dsp', rate });
    },
  });
  Interceptor.attach(e('_FMUSIC_PlaySong@4'), { onEnter() { recording = true; } });
} });
send({ type: 'ready' });
