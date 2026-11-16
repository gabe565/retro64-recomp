// Capture the ORIGINAL game's final FMOD mix (16-bit stereo) from inside FMOD
// 3.61, for comparison with the native build (`bugatron --record-audio`).
//
// After FSOUND_Init, a DSP unit is added with priority 1100, above FMOD's
// clip-and-copy unit (1000), so it receives the final 16-bit stereo output
// buffer. RECORD_SECONDS of audio from the first PlaySong onwards are sent as
// {type:'audio', rate, b64} (raw little-endian 16-bit stereo).
// Load together with capture_deterministic_frames.js for a matching timeline.

'use strict';

const RECORD_SECONDS = 12;
let e, dspCreate, dspActive;

let rate = 44100, recording = false, chunks = [], bytes = 0, sent = false;

function b64(u8) {
  const A = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  let out = '';
  for (let i = 0; i < u8.length; i += 3) {
    const n = (u8[i] << 16) | ((u8[i + 1] || 0) << 8) | (u8[i + 2] || 0);
    out += A[(n >> 18) & 63] + A[(n >> 12) & 63] + (i + 1 < u8.length ? A[(n >> 6) & 63] : '=') + (i + 2 < u8.length ? A[n & 63] : '=');
  }
  return out;
}

const tap = new NativeCallback((orig, buf, length, param) => {
  if (recording && !sent) {
    const n = length * 4;
    chunks.push(new Uint8Array(buf.readByteArray(n)));
    bytes += n;
    if (bytes >= RECORD_SECONDS * rate * 4) {
      sent = true;
      const all = new Uint8Array(bytes);
      let o = 0;
      for (const c of chunks) { all.set(c, o); o += c.length; }
      send({ type: 'audio', rate, frames: bytes / 4, b64: b64(all) });
    }
  }
  return buf;
}, 'pointer', ['pointer', 'pointer', 'int', 'int'], 'stdcall');

// fmod.dll may not be mapped yet when the process is spawned: set up when the
// game's sound initialisation (0x4172b0) starts.
let setUp = false;
Interceptor.attach(ptr(0x4172b0), { onEnter() {
  if (setUp) return;
  setUp = true;
  const fmod = Process.getModuleByName('fmod.dll');
  e = (n) => fmod.getExportByName(n);
  dspCreate = new NativeFunction(e('_FSOUND_DSP_Create@12'), 'pointer', ['pointer', 'int', 'int'], 'stdcall');
  dspActive = new NativeFunction(e('_FSOUND_DSP_SetActive@8'), 'void', ['pointer', 'int'], 'stdcall');
  hookFmod();
} });

function hookFmod() {
Interceptor.attach(e('_FSOUND_Init@12'), {
  onEnter(args) { rate = args[0].toInt32(); },
  onLeave(ret) {
    if ((ret.toInt32() & 0xff) === 0) return;
    const unit = dspCreate(tap, 1100, 0);
    dspActive(unit, 1);
    send({ type: 'dsp', unit: unit.toString(), rate });
  },
});
Interceptor.attach(e('_FMUSIC_PlaySong@4'), { onEnter() { recording = true; } });
}
send({ type: 'ready' });
