//! Observe keys only up to the first non-duplicate failure. Exact sorted
//! duplicates within that prefix may precede it; select the earliest original
//! token, never the lexicographically first duplicate or a later syntax error.
use super::{Decoder, F, Failure, Kind, R, scalar};
use crate::file_protocol::request_json::key_arena::{Arena, Key};
use serde::de::DeserializeSeed;
use std::{cmp::Reverse, collections::BinaryHeap, ops::Range};

const CHUNK_ENTRIES: usize = 4096;

struct Entry<'a> {
    key: Key<'a>,
    range: Range<usize>,
}

pub(super) fn map(decoder: &mut Decoder<'_>, body: &[u8]) -> Result<(), Failure> {
    let first = decoder.read()?;
    if first.kind != Kind::ObjectOpen {
        return Err(Failure::token(R::UnexpectedToken, &first));
    }
    let mut arena = Arena::default();
    let mut chunks = Vec::new();
    let result = prefix(decoder, body, &mut arena, &mut chunks);
    if matches!(&result, Err(failure) if failure.reason == R::DuplicateMap) {
        return result;
    }
    if let Some(range) = duplicate(&mut chunks, &arena) {
        return Err(Failure {
            reason: R::DuplicateMap,
            position: range.start,
            raw: range,
        });
    }
    result
}

fn prefix<'a>(
    decoder: &mut Decoder<'_>,
    body: &'a [u8],
    arena: &mut Arena,
    chunks: &mut Vec<Vec<Entry<'a>>>,
) -> Result<(), Failure> {
    loop {
        let key = decoder.read()?;
        if key.kind == Kind::ObjectClose {
            return Ok(());
        }
        if key.kind != Kind::Name {
            return Err(Failure::token(R::UnexpectedToken, &key));
        }
        let value = arena
            .seed()
            .deserialize(&mut serde_json::Deserializer::from_slice(
                &body[key.range.clone()],
            ))
            .map_err(|_| Failure::token(R::InvalidValue, &key))?;
        if chunks
            .last()
            .is_none_or(|chunk| chunk.len() == CHUNK_ENTRIES)
        {
            chunks.push(Vec::new());
        }
        chunks.last_mut().expect("chunk exists").push(Entry {
            key: value,
            range: key.range,
        });
        // Include this key even if its value fails: a duplicate key precedes
        // that value's error in the original streaming decoder.
        scalar(decoder, F::MapValue, body)?;
        if chunks.len().is_power_of_two()
            && chunks
                .last()
                .is_some_and(|chunk| chunk.len() == CHUNK_ENTRIES)
            && let Some(range) = duplicate(chunks, arena)
        {
            return Err(Failure {
                reason: R::DuplicateMap,
                position: range.start,
                raw: range,
            });
        }
    }
}

fn duplicate(chunks: &mut [Vec<Entry<'_>>], arena: &Arena) -> Option<Range<usize>> {
    for chunk in &mut *chunks {
        chunk.sort_unstable_by(|left, right| {
            left.key
                .as_str(arena)
                .cmp(right.key.as_str(arena))
                .then_with(|| left.range.start.cmp(&right.range.start))
        });
    }
    let mut heads: BinaryHeap<_> = chunks
        .iter()
        .enumerate()
        .filter_map(|(index, chunk)| {
            chunk
                .first()
                .map(|entry| Reverse((entry.key.as_str(arena), entry.range.start, index, 0)))
        })
        .collect();
    let mut previous = None;
    let mut first: Option<Range<usize>> = None;
    while let Some(Reverse((key, position, chunk, offset))) = heads.pop() {
        if previous == Some(key) && first.as_ref().is_none_or(|range| position < range.start) {
            first = Some(chunks[chunk][offset].range.clone());
        }
        previous = Some(key);
        if let Some(next) = chunks[chunk].get(offset + 1) {
            heads.push(Reverse((
                next.key.as_str(arena),
                next.range.start,
                chunk,
                offset + 1,
            )));
        }
    }
    first
}
