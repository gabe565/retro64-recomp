// Read-only probe: sample the angle argument of the r3d rotation-matrix builder
// 0x421680(float *m, float ax, float ay, float az, float angle), which computes
// fsin/fcos of angle*0.5. Reports range, magnitude histogram and distinct samples.
'use strict';
const EXPECT = '83ec70d9442478d944247cd984248000';
const bytes = Array.from(new Uint8Array(ptr(0x421680).readByteArray(16))).map((b) => b.toString(16).padStart(2, '0')).join('');
if (bytes !== EXPECT) throw new Error('prologue mismatch at 0x421680: ' + bytes);
let n = 0, min = Infinity, max = -Infinity, maxAbs = 0;
const hist = {};          // floor(log2(|angle|)) -> count
const distinct = new Set();
const samples = [];
let frame = 0;
Interceptor.attach(ptr(0x421680), {
  onEnter() {
    const a = this.context.esp.add(20).readFloat();
    n++;
    if (a < min) min = a;
    if (a > max) max = a;
    if (Math.abs(a) > maxAbs) maxAbs = Math.abs(a);
    const k = a === 0 ? 'zero' : String(Math.floor(Math.log2(Math.abs(a))));
    hist[k] = (hist[k] || 0) + 1;
    const bits = this.context.esp.add(20).readU32();
    if (!distinct.has(bits) && distinct.size < 4000) { distinct.add(bits); if (samples.length < 300) samples.push(bits); }
  },
});
Interceptor.attach(ptr(0x40b720), {
  onLeave() {
    frame++;
    if (frame % 600 === 0) send({ type: 'angles', frame, n, min, max, maxAbs, hist, distinct: distinct.size, samples });
  },
});
send({ type: 'hello' });
