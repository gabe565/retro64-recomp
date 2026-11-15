//! Guest heap (HeapAlloc family) and VirtualAlloc regions inside guest memory.
//! Bookkeeping lives on the host side, so guest code cannot corrupt it.

use std::collections::BTreeMap;

pub struct Heap {
    next: u32,
    end: u32,
    /// addr -> (requested size, capacity)
    live: BTreeMap<u32, (u32, u32)>,
    /// capacity -> free blocks
    free: BTreeMap<u32, Vec<u32>>,
}

fn align8(n: u32) -> u32 {
    (n + 7) & !7
}

impl Heap {
    pub fn new(low: u32, high: u32) -> Self {
        Heap { next: low, end: high, live: BTreeMap::new(), free: BTreeMap::new() }
    }

    pub fn alloc(&mut self, size: u32) -> Option<u32> {
        let cap = align8(size.max(8));
        if let Some(list) = self.free.get_mut(&cap) {
            if let Some(a) = list.pop() {
                self.live.insert(a, (size, cap));
                return Some(a);
            }
        }
        // 8-byte header gap between blocks catches small overruns in tests.
        let a = self.next;
        let n = a.checked_add(cap + 8)?;
        if n > self.end {
            return None;
        }
        self.next = n;
        self.live.insert(a, (size, cap));
        Some(a)
    }

    pub fn free(&mut self, a: u32) -> bool {
        match self.live.remove(&a) {
            Some((_, cap)) => {
                self.free.entry(cap).or_default().push(a);
                true
            }
            None => false,
        }
    }

    pub fn size(&self, a: u32) -> Option<u32> {
        self.live.get(&a).map(|x| x.0)
    }

    pub fn resize_in_place(&mut self, a: u32, size: u32) -> bool {
        match self.live.get_mut(&a) {
            Some(e) if size <= e.1 => {
                e.0 = size;
                true
            }
            _ => false,
        }
    }
}

/// VirtualAlloc emulation: 64 KiB-aligned reservations, page-granular commits.
pub struct VirtualRegions {
    next: u32,
    end: u32,
    /// base -> size
    regions: BTreeMap<u32, u32>,
}

impl VirtualRegions {
    pub fn new(low: u32, high: u32) -> Self {
        VirtualRegions { next: low, end: high, regions: BTreeMap::new() }
    }

    pub fn reserve(&mut self, size: u32) -> Option<u32> {
        let sz = (size + 0xffff) & !0xffff;
        let a = self.next;
        let n = a.checked_add(sz)?;
        if n > self.end {
            return None;
        }
        self.next = n;
        self.regions.insert(a, sz);
        Some(a)
    }

    pub fn contains(&self, addr: u32, size: u32) -> bool {
        self.regions
            .range(..=addr)
            .next_back()
            .is_some_and(|(&b, &s)| addr >= b && (addr as u64 + size as u64) <= b as u64 + s as u64)
    }
}
