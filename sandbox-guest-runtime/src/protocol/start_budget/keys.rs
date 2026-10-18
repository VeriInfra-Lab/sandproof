//! Exact duplicate checking without one allocation per tree node or a single
//! key vector whose geometric growth can temporarily double all key storage.
use std::{cmp::Reverse, collections::BinaryHeap};

pub(super) use crate::file_protocol::request_json::key_arena::Arena;
use crate::file_protocol::request_json::key_arena::Key;

// Allocation granularity only, never an input count/length acceptance limit.
const CHUNK_ENTRIES: usize = 4096;

#[derive(Default)]
pub(super) struct Chunks<'a> {
    blocks: Vec<Vec<Key<'a>>>,
    count: usize,
    sorted: usize,
    checked_count: usize,
}

impl<'a> Chunks<'a> {
    pub(super) fn push(&mut self, key: Key<'a>) {
        if self
            .blocks
            .last()
            .is_none_or(|chunk| chunk.len() == CHUNK_ENTRIES)
        {
            self.blocks.push(Vec::new());
        }
        self.sorted = self.sorted.min(self.blocks.len() - 1);
        self.blocks.last_mut().expect("chunk exists").push(key);
        self.count += 1;
    }

    pub(super) const fn len(&self) -> usize {
        self.count
    }

    pub(super) fn checkpoint(&self) -> bool {
        self.count.is_multiple_of(CHUNK_ENTRIES) && self.blocks.len().is_power_of_two()
    }

    pub(super) fn unique(&mut self, arena: &Arena) -> bool {
        if self.checked_count == self.count {
            return true;
        }
        for chunk in self.blocks.iter_mut().skip(self.sorted) {
            chunk.sort_unstable_by(|left, right| left.as_str(arena).cmp(right.as_str(arena)));
        }
        self.sorted = self.blocks.len();
        // Merge sorted chunks using only one borrowed cursor per chunk.
        // Equality is decoded content, including between storage variants.
        let mut heads: BinaryHeap<_> = self
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(index, chunk)| {
                chunk
                    .first()
                    .map(|key| Reverse((key.as_str(arena), index, 0)))
            })
            .collect();
        let mut previous = None;
        while let Some(Reverse((key, chunk, offset))) = heads.pop() {
            if previous == Some(key) {
                return false;
            }
            previous = Some(key);
            if let Some(next) = self.blocks[chunk].get(offset + 1) {
                heads.push(Reverse((next.as_str(arena), chunk, offset + 1)));
            }
        }
        self.checked_count = self.count;
        true
    }
}
