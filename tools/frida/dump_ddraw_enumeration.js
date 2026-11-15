// Read-only dump of the r3d enumeration results held in the running game:
// driver records (0xa06b0c list, stride via +0x368) with HAL/HEL DDCAPS,
// device records (+0x364 list, +0x158) with D3DDEVICEDESC at +0x3c,
// display modes (+0x154 list, +0xa4) as raw DDSURFACEDESC2 + name,
// texture-format slots 0x9fa800 + 0x24*k, Z-buffer format 0x9d5750, and
// the selected-format globals. Bugatron Gold v1.20a only.
'use strict';
const hex = (a, n) => Array.from(new Uint8Array(ptr(a).readByteArray(n))).map(b => b.toString(16).padStart(2, '0')).join('');
const u32 = a => ptr(a).readU32();
const out = { drivers: [], texture_slots: [], globals: {} };
for (let d = u32(0xa06b0c); d !== 0; d = u32(d + 0x368)) {
  const drv = { addr: d, guid_ptr: u32(d + 0x10), guid: hex(d, 16), desc: ptr(d + 0x14).readCString(), name: ptr(d + 0x3c).readCString(),
    hal_caps: hex(d + 0x64, 0x17c), hel_caps: hex(d + 0x1e0, 0x17c), devices: [] };
  for (let v = u32(d + 0x364); v !== 0; v = u32(v + 0x158)) {
    const dev = { addr: v, guid: hex(v, 16), name: ptr(v + 0x14).readCString(), hw: u32(v + 0x138), windowed: u32(v + 0x13c),
      win_bpp_masks: [u32(v + 0x140), u32(v + 0x144), u32(v + 0x148), u32(v + 0x14c)], desc: hex(v + 0x3c, 0xfc), modes: [] };
    for (let m = u32(v + 0x154); m !== 0; m = u32(m + 0xa4)) dev.modes.push({ name: ptr(m + 0x7c).readCString(), sd: hex(m, 0x7c) });
    drv.devices.push(dev);
  }
  out.drivers.push(drv);
}
for (let k = 0; k < 6; k++) out.texture_slots.push(hex(0x9fa800 + 0x24 * k, 0x24));
out.zbuffer_format = hex(0x9d5750, 0x20);
for (const [n, a] of Object.entries({ bpp: 0x9fa8fc, backbuffer_code: 0xa02a50, alpha_tex_code: 0x9fa910, opaque_tex_code: 0xa06b08,
  sel_driver: 0x98d4a8, sel_device: 0x98d4ac, sel_mode: 0x98d4b0, windowed: 0x9d5774, max_tex_w: 0xa06ae8, max_tex_h: 0xa06b10, devcaps_bit10: 0xa06b14,
  ddenum_ex_used_flag: 0xa06b9c, saw_device: 0xa06ba0 })) out.globals[n] = u32(a);
send(JSON.stringify(out));
