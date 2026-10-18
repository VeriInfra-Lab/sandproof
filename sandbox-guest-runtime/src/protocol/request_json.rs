//! Scalar behavior of the fixed default Connect protobuf JSON codec.
use super::Signal;
use crate::file_protocol::request_json::exact_uint32;
use base64::{
    Engine, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};
use serde::{Deserialize, Deserializer, de};
use serde_json::value::RawValue;
use std::collections::{BTreeMap, btree_map::Entry};

pub(super) fn bytes<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    let encoded = String::deserialize(deserializer)?;
    decode_bytes(&encoded).map_err(|_| de::Error::custom("invalid bytes"))
}

pub(crate) fn decode_bytes(encoded: &str) -> Result<Vec<u8>, base64::DecodeError> {
    // The original chooses padding using the parsed string's byte length BEFORE
    // DecodeString ignores CR/LF. Trying four decoders would accept other input.
    let padding = if encoded.len().is_multiple_of(4) {
        DecodePaddingMode::RequireCanonical
    } else {
        DecodePaddingMode::RequireNone
    };
    let alphabet = if encoded.contains(['-', '_']) {
        &alphabet::URL_SAFE
    } else {
        &alphabet::STANDARD
    };
    let engine = GeneralPurpose::new(
        alphabet,
        GeneralPurposeConfig::new()
            .with_decode_padding_mode(padding)
            .with_decode_allow_trailing_bits(true),
    );
    let clean;
    let input = if encoded.contains(['\r', '\n']) {
        clean = encoded
            .bytes()
            .filter(|b| !matches!(b, b'\r' | b'\n'))
            .collect::<Vec<_>>();
        clean.as_slice()
    } else {
        encoded.as_bytes()
    };
    engine.decode(input)
}

pub(super) fn signal<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Signal>, D::Error> {
    let raw = <&RawValue>::deserialize(deserializer)?.get();
    if raw == "null" {
        return Ok(None);
    }
    if raw.starts_with('"') {
        let name: String = serde_json::from_str(raw).map_err(de::Error::custom)?;
        return Ok(match name.as_str() {
            "SIGNAL_UNSPECIFIED" => Some(Signal::Unspecified),
            "SIGNAL_SIGTERM" => Some(Signal::Terminate),
            "SIGNAL_SIGKILL" => Some(Signal::Kill),
            _ => None, // default Connect DiscardUnknown also covers enum names
        });
    }
    let negative = raw.starts_with('-');
    let magnitude = exact_uint32(raw.strip_prefix('-').unwrap_or(raw))
        .ok_or_else(|| de::Error::custom("invalid enum integer"))?;
    let signed = if negative {
        -i64::from(magnitude)
    } else {
        i64::from(magnitude)
    };
    let value = i32::try_from(signed).map_err(|_| de::Error::custom("enum out of range"))?;
    Ok(Some(match value {
        0 => Signal::Unspecified,
        9 => Signal::Kill,
        15 => Signal::Terminate,
        value => Signal::Unknown(value),
    }))
}

pub(super) fn envs<'de, D, K, V>(deserializer: D) -> Result<BTreeMap<K, V>, D::Error>
where
    D: Deserializer<'de>,
    K: Deserialize<'de> + Ord,
    V: Deserialize<'de>,
{
    struct Map<K, V>(BTreeMap<K, V>);
    impl<'de, K: Deserialize<'de> + Ord, V: Deserialize<'de>> Deserialize<'de> for Map<K, V> {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct Visitor<K, V>(std::marker::PhantomData<(K, V)>);
            impl<'de, K: Deserialize<'de> + Ord, V: Deserialize<'de>> de::Visitor<'de> for Visitor<K, V> {
                type Value = Map<K, V>;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str("a string map")
                }
                fn visit_map<M: de::MapAccess<'de>>(
                    self,
                    mut map: M,
                ) -> Result<Self::Value, M::Error> {
                    let mut values = BTreeMap::new();
                    while let Some(key) = map.next_key::<K>()? {
                        let Entry::Vacant(entry) = values.entry(key) else {
                            return Err(de::Error::custom("duplicate environment key"));
                        };
                        entry.insert(map.next_value::<V>()?);
                    }
                    Ok(Map(values))
                }
            }
            deserializer.deserialize_map(Visitor(std::marker::PhantomData))
        }
    }
    Ok(Option::<Map<K, V>>::deserialize(deserializer)?.map_or_else(BTreeMap::new, |map| map.0))
}
