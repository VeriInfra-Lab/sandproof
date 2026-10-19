//! Observe keys only up to the first non-duplicate failure. Exact sorted
//! duplicates within that prefix may precede it; select the earliest original
//! token, never the lexicographically first duplicate or a later syntax error.
use super::super::tokens::string_length;
use super::{Decoder, F, Failure, Kind, R, scalar};
use crate::file_protocol::request_json::key_arena::{Arena, Key};
use serde::de::DeserializeSeed;
use std::{cmp::Reverse, collections::BinaryHeap};

const CHUNK_ENTRIES: usize = 2048;

struct Entry<'a> {
    key: Key<'a>,
    position: usize,
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
    if let Some(position) = duplicate(&mut chunks, &arena) {
        return Err(failure(body, position));
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
            position: key.range.start,
        });
        // Include this key even if its value fails: a duplicate key precedes
        // that value's error in the original streaming decoder.
        scalar(decoder, F::MapValue, body)?;
        if chunks.len().is_power_of_two()
            && chunks
                .last()
                .is_some_and(|chunk| chunk.len() == CHUNK_ENTRIES)
            && let Some(position) = duplicate(chunks, arena)
        {
            return Err(failure(body, position));
        }
    }
}

fn failure(body: &[u8], position: usize) -> Failure {
    // Every stored position came from a validated Name token in this same
    // immutable body. Recover only the selected token's end with the original
    // tokenizer, instead of retaining two machine words for every key span.
    let length = string_length(&body[position..], position)
        .unwrap_or_else(|_| panic!("stored key was already validated by the original tokenizer"));
    Failure {
        reason: R::DuplicateMap,
        position,
        raw: position..position + length,
    }
}

fn duplicate(chunks: &mut [Vec<Entry<'_>>], arena: &Arena) -> Option<usize> {
    for chunk in &mut *chunks {
        chunk.sort_unstable_by(|left, right| {
            left.key
                .as_str(arena)
                .cmp(right.key.as_str(arena))
                .then_with(|| left.position.cmp(&right.position))
        });
    }
    let mut heads: BinaryHeap<_> = chunks
        .iter()
        .enumerate()
        .filter_map(|(index, chunk)| {
            chunk
                .first()
                .map(|entry| Reverse((entry.key.as_str(arena), entry.position, index, 0)))
        })
        .collect();
    let mut previous = None;
    let mut first = None;
    while let Some(Reverse((key, position, chunk, offset))) = heads.pop() {
        if previous == Some(key) && first.is_none_or(|earlier| position < earlier) {
            first = Some(position);
        }
        previous = Some(key);
        if let Some(next) = chunks[chunk].get(offset + 1) {
            heads.push(Reverse((
                next.key.as_str(arena),
                next.position,
                chunk,
                offset + 1,
            )));
        }
    }
    first
}
