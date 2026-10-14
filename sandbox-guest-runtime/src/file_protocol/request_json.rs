//! Fixed Connect `DiscardUnknown` / protobuf uint32 input semantics. Keep this
//! separate from command decoding, whose complete protobuf contract is pending.
use crate::protocol::ProtocolError;
use serde::{Deserialize, Deserializer, de};
use serde_json::value::RawValue;

/// Serde's iterative ignored-value parser checks JSON syntax, but deliberately
/// skips UTF-8/surrogate validation and does not enforce a nesting limit. Check
/// every string and container first, including discarded fields. This scan is
/// iterative; its only temporary allocation is the current escaped string's
/// decoder scratch, bounded by the already admitted body. No Value tree is built.
/// The fixed protobuf default allows 10,000 containers including the message.
pub(super) fn validate(body: &[u8]) -> Result<(), ProtocolError> {
    let mut offset = 0;
    let mut depth = 0_usize;
    while let Some(byte) = body.get(offset) {
        match byte {
            b'"' => {
                let mut tokens = serde_json::Deserializer::from_slice(&body[offset..])
                    .into_iter::<CheckedString>();
                tokens
                    .next()
                    .ok_or(ProtocolError::InvalidArgument)?
                    .map_err(|_| ProtocolError::InvalidArgument)?;
                offset += tokens.byte_offset();
                continue;
            }
            b'[' | b'{' => {
                depth += 1;
                if depth > 10_000 {
                    return Err(ProtocolError::InvalidArgument);
                }
            }
            b']' | b'}' => {
                depth = depth.checked_sub(1).ok_or(ProtocolError::InvalidArgument)?;
            }
            _ => {}
        }
        offset += 1;
    }
    // Object shape, matching brackets, numeric syntax, duplicates and trailing
    // input are checked by the typed decoder immediately after this scan.
    Ok(())
}

struct CheckedString;
impl<'de> Deserialize<'de> for CheckedString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl de::Visitor<'_> for Visitor {
            type Value = CheckedString;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a valid Unicode string")
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
                Ok(CheckedString)
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}

pub(super) fn uint32<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    // Borrow the numeric spelling: conversion via f64 would round fractions
    // and would reject a large mantissa scaled back into range by its exponent.
    let raw = <&RawValue>::deserialize(deserializer)?.get();
    if raw == "null" {
        return Ok(0);
    }
    let decoded;
    let number = if raw.starts_with('"') {
        decoded = serde_json::from_str::<String>(raw).map_err(de::Error::custom)?;
        decoded.as_str()
    } else {
        raw
    };
    exact_uint32(number).ok_or_else(|| de::Error::custom("invalid protobuf uint32"))
}

fn exact_uint32(number: &str) -> Option<u32> {
    // RawValue checks JSON number grammar without floating point conversion.
    // Equality rejects all surrounding JSON whitespace in a quoted number.
    if !matches!(number.as_bytes().first(), Some(b'-' | b'0'..=b'9')) {
        return None;
    }
    let raw: &RawValue = serde_json::from_str(number).ok()?;
    if raw.get() != number {
        return None;
    }
    let unsigned = number.strip_prefix('-').unwrap_or(number);
    let (mantissa, exponent) = unsigned.split_once(['e', 'E']).unwrap_or((unsigned, "0"));
    let (integer, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let integer = if integer == "0" { "" } else { integer };
    let fraction = fraction.trim_end_matches('0');
    // Protobuf normalizes negative zero and zero with any syntactically valid
    // exponent before attempting its signed 32-bit exponent conversion.
    if integer.is_empty() && fraction.is_empty() {
        return Some(0);
    }
    if number.starts_with('-') {
        return None;
    }
    let exponent: i32 = exponent.parse().ok()?;
    if let Ok(shift) = usize::try_from(exponent) {
        // Match protobuf's 20-digit normalization guard, including leading
        // fractional zeros, before applying the final uint32 range check.
        if fraction.len() > shift || integer.len().checked_add(shift)? > 20 {
            return None;
        }
        integer
            .bytes()
            .chain(fraction.bytes())
            .chain(std::iter::repeat_n(b'0', shift - fraction.len()))
            .try_fold(0_u32, |value, digit| {
                value.checked_mul(10)?.checked_add(u32::from(digit - b'0'))
            })
    } else {
        if !fraction.is_empty() {
            return None;
        }
        let end = integer
            .len()
            .checked_sub(usize::try_from(exponent.unsigned_abs()).ok()?)?;
        if !integer.as_bytes()[end..].iter().all(|digit| *digit == b'0') {
            return None;
        }
        integer[..end].parse().ok()
    }
}
