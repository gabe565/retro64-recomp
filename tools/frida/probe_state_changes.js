// Read-only probe: log game-state and current-config changes in Bugatron Gold v1.20a.
//
// Samples once per frame (after FUN_0040b720) and sends a message only when the
// game state (0x4370a0) or the current config name (0x81d548) changes.
// Rows: {frame, tick, state, config}. Nothing in the target is modified.

'use strict';

const EXPECT_40B720 = '83ec0856c7442404c8000000e89f7e01';
const fn = ptr(0x40b720);
const bytes = Array.from(new Uint8Array(fn.readByteArray(16)))
  .map((b) => b.toString(16).padStart(2, '0')).join('');

if (bytes !== EXPECT_40B720) {
  send({ type: 'error', reason: 'prologue mismatch at 0x40b720', bytes });
} else {
  const G_TICK = ptr(0x8429dc);
  const G_STATE = ptr(0x4370a0);
  const G_CONFIG = ptr(0x81d548);
  let frame = 0;
  let last = null;
  const read = () => ({ state: G_STATE.readS32(), config: G_CONFIG.readCString() });
  const initial = read();
  send({ type: 'change', frame: 0, tick: G_TICK.readU32(), ...initial, initial: true });
  last = initial;
  Interceptor.attach(fn, {
    onLeave() {
      frame++;
      const now = read();
      if (now.state !== last.state || now.config !== last.config) {
        send({ type: 'change', frame, tick: G_TICK.readU32(), ...now, from_state: last.state });
        last = now;
      }
    },
  });
}
