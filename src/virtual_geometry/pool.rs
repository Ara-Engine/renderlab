//! Shared fixed-size page slots. Uploads reuse chunk memory instead of creating buffers, and every
//! page of a chunk draws through the chunk's single `Mesh`, so consecutive draws rarely rebind.
use super::PAGE_BYTES;
use crate::Mesh;
use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    sync::{Arc, Mutex},
};

/// Vertices per slot: the fewest 48-byte vertices covering one charged page.
pub(crate) const SLOT_VERTICES: u32 = PAGE_BYTES.div_ceil(48) as u32;
/// Slot stride, a whole number of vertices so each slot's first vertex has an integral index.
pub(crate) const SLOT_BYTES: u64 = SLOT_VERTICES as u64 * 48;
/// Largest chunk buffer, below the device allocator's 128 MiB block so chunks are suballocated
/// like ordinary buffers. Dedicated 134 MB chunks let upload flights outlive frames under memory
/// pressure, which denied uploads.
pub(crate) const CHUNK_BYTES: u64 = 32 << 20;

type FreeSlots = Arc<Mutex<BinaryHeap<Reverse<u32>>>>;

/// Page slots in chunk buffers bound as both vertex and index data. A slot holds one tangent-free
/// page's vertices followed by its indices, rebased to the chunk's vertex numbering. There is one
/// slot per page the geometry budget can charge, so physical capacity tracks the logical budget.
/// The renderer creates chunks on first use; the asset retains them.
pub(crate) struct PagePool {
    pub(crate) chunks: Vec<Arc<Mesh>>,
    pub(crate) capacity: u32,
    next: u32,
    free: FreeSlots,
}

/// A leased slot. Dropping it returns the slot, so its page must drop only after its last GPU use.
pub(crate) struct PageSlot {
    pub(crate) index: u32,
    free: FreeSlots,
}

impl Drop for PageSlot {
    fn drop(&mut self) {
        if let Ok(mut free) = self.free.lock() {
            free.push(Reverse(self.index));
        }
    }
}

impl PagePool {
    pub(crate) fn new(geometry_bytes: usize) -> Self {
        Self {
            chunks: vec![],
            capacity: (geometry_bytes / PAGE_BYTES).min(u32::MAX as usize) as u32,
            next: 0,
            free: Arc::default(),
        }
    }
    /// The lowest free slot, keeping occupancy dense in the first chunks. None when all are leased.
    pub(crate) fn acquire(&mut self) -> Option<PageSlot> {
        let index = match self.free.lock().unwrap().pop() {
            Some(Reverse(index)) => index,
            None if self.next < self.capacity => {
                self.next += 1;
                self.next - 1
            }
            None => return None,
        };
        Some(PageSlot {
            index,
            free: self.free.clone(),
        })
    }
}
