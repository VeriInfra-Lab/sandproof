//! Validate the fixed Connect end-stream JSON without retaining metadata or
//! applying client errors to the command. Go's handler treats a valid final
//! envelope as EOF after its one request, even when it contains an error.
use super::ProtocolError;
use base64::{
    Engine, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};
use serde::{
    Deserializer,
    de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor},
};
use std::fmt;

mod diagnosis;
mod normalize;
pub use diagnosis::{EndStreamError, EndStreamReason};
#[cfg(test)]
mod tests;

pub(super) fn validate(payload: &[u8]) -> Result<(), ProtocolError> {
    let normalized = normalize::json(payload)?;
    let mut decoder = serde_json::Deserializer::from_slice(&normalized);
    Node::End
        .deserialize(&mut decoder)
        .map_err(|_| ProtocolError::InvalidArgument)?;
    decoder.end().map_err(|_| ProtocolError::InvalidArgument)
}

pub(super) fn validate_observed<E: From<ProtocolError> + From<EndStreamError>>(
    payload: &[u8],
) -> Result<(), E> {
    validate(payload).map_err(|error| {
        if error == ProtocolError::InvalidArgument
            && let Some(observed) = diagnosis::observe(payload)
        {
            return E::from(observed);
        }
        E::from(error)
    })
}

#[derive(Clone, Copy)]
enum Node {
    End,
    Error,
    Detail,
    Metadata,
    Details,
    Strings,
}
impl<'de> DeserializeSeed<'de> for Node {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        decoder.deserialize_option(Nullable(self))
    }
}
struct Nullable(Node);
impl<'de> Visitor<'de> for Nullable {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("nullable Connect metadata")
    }
    fn visit_none<E: de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_unit<E: de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_some<D: Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        match self.0 {
            Node::Details | Node::Strings => decoder.deserialize_seq(Fields(self.0)),
            _ => decoder.deserialize_map(Fields(self.0)),
        }
    }
}
struct Fields(Node);
impl<'de> Visitor<'de> for Fields {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Connect metadata fields")
    }
    fn visit_seq<S: SeqAccess<'de>>(self, mut values: S) -> Result<(), S::Error> {
        if matches!(self.0, Node::Details) {
            while values.next_element_seed(Node::Detail)?.is_some() {}
        } else {
            while values.next_element::<Option<String>>()?.is_some() {}
        }
        Ok(())
    }
    fn visit_map<M: MapAccess<'de>>(self, mut fields: M) -> Result<(), M::Error> {
        let mut binary = String::new();
        while let Some(key) = fields.next_key::<String>()? {
            match self.0 {
                Node::Metadata => fields.next_value_seed(Node::Strings)?,
                Node::End if field(&key, "error") => fields.next_value_seed(Node::Error)?,
                Node::End if field(&key, "metadata") => fields.next_value_seed(Node::Metadata)?,
                Node::Error if field(&key, "details") => fields.next_value_seed(Node::Details)?,
                Node::Error if field(&key, "code") || field(&key, "message") => {
                    fields.next_value::<Option<String>>()?;
                }
                Node::Detail if field(&key, "type") => {
                    fields.next_value::<Option<String>>()?;
                }
                Node::Detail if field(&key, "value") => {
                    // encoding/json ignores null for a string. The custom
                    // detail decoder validates only the final non-null value.
                    if let Some(value) = fields.next_value::<Option<String>>()? {
                        binary = value;
                    }
                }
                _ => {
                    fields.next_value::<IgnoredAny>()?;
                }
            }
        }
        if matches!(self.0, Node::Detail) {
            validate_binary(&binary).map_err(de::Error::custom)?;
        }
        Ok(())
    }
}
fn field(key: &str, name: &str) -> bool {
    // Go's Unicode simple folding has two non-ASCII members in ASCII letter
    // classes: long s and Kelvin sign. All schema names here are ASCII.
    let mut chars = key.chars();
    for expected in name.chars() {
        let Some(actual) = chars.next() else {
            return false;
        };
        let folded = match actual {
            '\u{017f}' => 's',
            '\u{212a}' => 'k',
            c => c.to_ascii_lowercase(),
        };
        if folded != expected {
            return false;
        }
    }
    chars.next().is_none()
}
fn validate_binary(value: &str) -> Result<(), &'static str> {
    let padding = if value.len().is_multiple_of(4) {
        DecodePaddingMode::RequireCanonical
    } else {
        DecodePaddingMode::RequireNone
    };
    let engine = GeneralPurpose::new(
        &alphabet::STANDARD,
        GeneralPurposeConfig::new()
            .with_decode_padding_mode(padding)
            .with_decode_allow_trailing_bits(true),
    );
    let clean;
    let bytes = if value.contains(['\r', '\n']) {
        clean = value
            .bytes()
            .filter(|b| !matches!(b, b'\r' | b'\n'))
            .collect::<Vec<_>>();
        clean.as_slice()
    } else {
        value.as_bytes()
    };
    engine
        .decode(bytes)
        .map(|_| ())
        .map_err(|_| "invalid detail value")
}
