//! Exact decoded JSON map keys. Short escapes need no per-key allocation;
//! large escaped keys own their decoding and unescaped keys borrow input.
use std::cmp::Ordering;

use serde::{Deserialize, Deserializer, de};

pub(crate) struct StringKey<'a>(Storage<'a>);

enum Storage<'a> {
    Borrowed(&'a str),
    Inline { bytes: [u8; 15], length: u8 },
    Owned(Box<str>),
}

impl StringKey<'_> {
    pub(crate) fn as_str(&self) -> &str {
        match &self.0 {
            Storage::Borrowed(value) => value,
            Storage::Owned(value) => value,
            Storage::Inline { bytes, length } => {
                // Only complete validated UTF-8 strings enter this private
                // storage; length is their exact byte count, never a prefix.
                std::str::from_utf8(&bytes[..usize::from(*length)])
                    .expect("inline key originated from a complete str")
            }
        }
    }
}

impl<'de> Deserialize<'de> for StringKey<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = StringKey<'de>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a string key")
            }
            fn visit_borrowed_str<E: de::Error>(self, value: &'de str) -> Result<Self::Value, E> {
                Ok(StringKey(Storage::Borrowed(value)))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                if let Ok(length) = u8::try_from(value.len())
                    && usize::from(length) <= 15
                {
                    let mut bytes = [0; 15];
                    bytes[..value.len()].copy_from_slice(value.as_bytes());
                    Ok(StringKey(Storage::Inline { bytes, length }))
                } else {
                    Ok(StringKey(Storage::Owned(value.into())))
                }
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}

// Storage representation never participates in equality or ordering. In
// particular, an escaped key must collide with its unescaped counterpart.
impl PartialEq for StringKey<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}
impl Eq for StringKey<'_> {}
impl PartialOrd for StringKey<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for StringKey<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::fmt::Write;

    #[test]
    fn key_storage_boundaries_keep_exact_equality_order_and_dense_key_size() {
        let values = [
            "",
            "a",
            "a\0b",
            "a=b",
            "中文",
            "123456789012345",
            "1234567890123456",
            "123456789012中",
            "1234567890123中",
            "12345678901🙂",
            "123456789012🙂",
        ];
        let spellings: Vec<_> = values
            .iter()
            .flat_map(|value| {
                let mut escaped = String::from("\"");
                for unit in value.encode_utf16() {
                    write!(&mut escaped, "\\u{unit:04x}").unwrap();
                }
                escaped.push('"');
                [serde_json::to_string(value).unwrap(), escaped]
            })
            .collect();
        let keys: Vec<StringKey<'_>> = spellings
            .iter()
            .map(|text| serde_json::from_str(text).unwrap())
            .collect();
        for (pair, expected) in keys.chunks_exact(2).zip(values) {
            assert_eq!(pair[0].as_str(), expected);
            assert_eq!(pair[1].as_str(), expected);
            assert!(pair[0] == pair[1]);
        }
        let unique: BTreeSet<_> = keys.into_iter().collect();
        assert_eq!(unique.len(), values.len());
        let mut expected = values.to_vec();
        expected.sort_unstable();
        assert_eq!(
            unique.iter().map(StringKey::as_str).collect::<Vec<_>>(),
            expected
        );
        // Keep the inline optimization from increasing every dense key node.
        assert!(std::mem::size_of::<StringKey<'_>>() <= 3 * std::mem::size_of::<usize>());
    }
}
