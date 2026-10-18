//! Exact duplicate checking without one allocation per tree node or a single
//! key vector whose geometric growth can temporarily double all key storage.
use std::{cmp::Reverse, collections::BinaryHeap};

mod arena;
pub(super) use arena::Arena;
use arena::Key;

// Allocation granularity only, never an input count/length acceptance limit.
const CHUNK_ENTRIES: usize = 4096;

#[derive(Default)]
pub(super) struct Chunks<'a> {
    chunks: Vec<Vec<Key<'a>>>,
    count: usize,
}

impl<'a> Chunks<'a> {
    pub(super) fn push(&mut self, key: Key<'a>) {
        if self
            .chunks
            .last()
            .is_none_or(|chunk| chunk.len() == CHUNK_ENTRIES)
        {
            self.chunks.push(Vec::new());
        }
        self.chunks.last_mut().expect("chunk exists").push(key);
        self.count += 1;
    }

    pub(super) const fn len(&self) -> usize {
        self.count
    }

    pub(super) fn unique(&mut self, arena: &Arena) -> bool {
        for chunk in &mut self.chunks {
            chunk.sort_unstable_by(|left, right| left.as_str(arena).cmp(right.as_str(arena)));
        }
        // Merge sorted chunks using only one borrowed cursor per chunk.
        // Equality is decoded content, including between storage variants.
        let mut heads: BinaryHeap<_> = self
            .chunks
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
            if let Some(next) = self.chunks[chunk].get(offset + 1) {
                heads.push(Reverse((next.as_str(arena), chunk, offset + 1)));
            }
        }
        true
    }
}
