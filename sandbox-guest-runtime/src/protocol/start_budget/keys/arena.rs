//! Long escaped keys share byte chunks owned by this one map validation.
//! Indices, rather than references into mutable storage, survive chunk growth.
use serde::{Deserializer, de};

const CHUNK_BYTES: usize = 64 * 1024;

#[derive(Default)]
pub(in super::super) struct Arena {
    chunks: Vec<String>,
}

pub(in super::super) enum Key<'a> {
    Borrowed(&'a str),
    Inline { bytes: [u8; 15], length: u8 },
    Pooled { chunk: usize, start: u32, end: u32 },
    // A single unusually large key is kept intact, never split or refused by
    // the allocation granularity. There can only be as many as fit the input.
    Owned(Box<str>),
}

impl Key<'_> {
    pub(in super::super) fn as_str<'a>(&'a self, arena: &'a Arena) -> &'a str {
        match self {
            Self::Borrowed(value) => value,
            Self::Inline { bytes, length } => std::str::from_utf8(&bytes[..usize::from(*length)])
                .expect("stored complete UTF-8 string"),
            Self::Pooled { chunk, start, end } => arena.chunks[*chunk]
                .get(*start as usize..*end as usize)
                .expect("stored complete UTF-8 slice"),
            Self::Owned(value) => value,
        }
    }
}

impl Arena {
    pub(in super::super) fn seed(&mut self) -> Seed<'_> {
        Seed(self)
    }

    fn transient(&mut self, value: &str) -> Key<'static> {
        if value.len() <= 15 {
            let mut bytes = [0; 15];
            bytes[..value.len()].copy_from_slice(value.as_bytes());
            return Key::Inline {
                bytes,
                length: u8::try_from(value.len()).expect("inline length"),
            };
        }
        if value.len() > CHUNK_BYTES {
            return Key::Owned(value.into());
        }
        if self
            .chunks
            .last()
            .is_none_or(|chunk| chunk.len() + value.len() > CHUNK_BYTES)
        {
            self.chunks.push(String::new());
        }
        let index = self.chunks.len() - 1;
        let chunk = &mut self.chunks[index];
        let start = chunk.len();
        let end = start + value.len();
        if end > chunk.capacity() {
            let capacity = chunk.capacity().saturating_mul(2).max(end).min(CHUNK_BYTES);
            chunk.reserve_exact(capacity - chunk.len());
        }
        chunk.push_str(value);
        Key::Pooled {
            chunk: index,
            start: u32::try_from(start).expect("chunk offset"),
            end: u32::try_from(end).expect("chunk offset"),
        }
    }
}

pub(in super::super) struct Seed<'a>(&'a mut Arena);

impl<'de> de::DeserializeSeed<'de> for Seed<'_> {
    type Value = Key<'de>;
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        struct Visitor<'a>(&'a mut Arena);
        impl<'de> de::Visitor<'de> for Visitor<'_> {
            type Value = Key<'de>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a string key")
            }
            fn visit_borrowed_str<E: de::Error>(self, value: &'de str) -> Result<Self::Value, E> {
                Ok(Key::Borrowed(value))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(self.0.transient(value))
            }
        }
        deserializer.deserialize_str(Visitor(self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pooled_offsets_survive_growth_rollover_and_large_complete_keys() {
        let mut arena = Arena::default();
        let values = [
            "🙂".repeat(4),
            "中".repeat(21_845),
            "x".repeat(65_536),
            "雪".repeat(21_846),
        ];
        let mut keys = Vec::new();
        for _ in 0..3 {
            keys.extend(values.iter().map(|value| arena.transient(value)));
        }
        for (index, key) in keys.iter().enumerate() {
            assert_eq!(key.as_str(&arena), values[index % values.len()]);
        }
        assert!(arena.chunks.iter().all(|chunk| chunk.len() <= CHUNK_BYTES));
        assert!(std::mem::size_of::<Key<'_>>() <= 3 * std::mem::size_of::<usize>());
    }
}
