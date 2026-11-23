//! A HashMap with a fast hasher for the runtime's integer keys (guest pointers,
//! handles). The standard SipHash defends against keys chosen to collide, which
//! these are not, and costs several times more per lookup; the render path looks
//! objects up for every draw call.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

pub type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FastHasher>>;

/// Multiply-rotate mixing of each integer written (keys are often aligned
/// addresses, so the low bits alone would cluster).
#[derive(Default)]
pub struct FastHasher(u64);

impl Hasher for FastHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(b as u64);
        }
    }
    fn write_u32(&mut self, v: u32) {
        self.write_u64(v as u64);
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
    fn write_usize(&mut self, v: usize) {
        self.write_u64(v as u64);
    }
}
