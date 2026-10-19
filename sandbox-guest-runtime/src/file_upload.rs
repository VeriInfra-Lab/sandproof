//! Fixed envd upload dispatch decisions; no I/O or HTTP admission.
// Adapted from Go 1.26.8 mime/mediatype.go, Copyright 2010 The Go Authors.
// BSD license: multipart/GO-LICENSE. Metadata contract: fixed envd upload.go
// and shared/pkg/filesystem/xattr.go, Apache-2.0: file_encoding/E2B-LICENSE.
use std::collections::BTreeMap;

mod fault;
pub use fault::{Fault, Stage};

/// Completed body/metadata work, not a durability or whole-batch transaction.
#[derive(Debug, PartialEq, Eq)]
pub struct UploadResult {
    pub path: String,
    pub written: u64,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Raw,
    Multipart,
    Unsupported,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidConfiguration,
    HeaderLimit,
    InvalidMetadata,
}

/// A validation observation without retaining an unbounded key or value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataViolation {
    EmptyKey,
    KeyLength,
    KeyByte { byte: u8, offset: usize },
    ValueByte { byte: u8, offset: usize },
}

/// Keep `ParseMediaType`'s returned type even on invalid optional parameters;
/// conflicting duplicate parameters instead discard the type. Parameter bytes
/// are opaque and never decoded just to choose raw versus multipart.
/// # Errors
/// Invalid or exceeded explicit header budget, before copying parameter bytes.
pub fn kind(value: &[u8], limit: usize) -> Result<Kind, Error> {
    if limit == 0 || limit > isize::MAX as usize / 16 {
        return Err(Error::InvalidConfiguration);
    }
    if value.len() > limit {
        return Err(Error::HeaderLimit);
    }
    let split = value.iter().position(|b| *b == b';').unwrap_or(value.len());
    let Ok(base) = std::str::from_utf8(&value[..split]) else {
        return Ok(Kind::Unsupported);
    };
    let base = crate::file_mime::lower(base.trim());
    let mut pieces = base.split('/');
    if !pieces.next().is_some_and(valid_token)
        || pieces.next().is_some_and(|p| !valid_token(p))
        || pieces.next().is_some()
    {
        return Ok(Kind::Unsupported);
    }
    let selected = if base.eq_ignore_ascii_case("application/octet-stream") {
        Kind::Raw
    } else if base
        .get(..10)
        .is_some_and(|p| p.eq_ignore_ascii_case("multipart/"))
    {
        Kind::Multipart
    } else {
        Kind::Unsupported
    };
    let mut rest = &value[split..];
    let mut parameters = BTreeMap::new();
    while let Some((key, value, after)) = parameter(rest) {
        let key = key.to_ascii_lowercase();
        if parameters.get(&key).is_some_and(|prior| *prior != value) {
            return Ok(Kind::Unsupported);
        }
        parameters.insert(key, value);
        rest = after;
    }
    Ok(selected)
}
fn valid_token(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(token)
}
fn special(byte: u8) -> bool {
    b"()<>@,;:\\\"/[]?=".contains(&byte)
}
fn token(byte: u8) -> bool {
    byte > b' ' && byte < 127 && !special(byte)
}
fn trim_start(mut bytes: &[u8]) -> &[u8] {
    while let Some(&first) = bytes.first() {
        let width = if first < 128 {
            1
        } else if first & 0xe0 == 0xc0 {
            2
        } else if first & 0xf0 == 0xe0 {
            3
        } else if first & 0xf8 == 0xf0 {
            4
        } else {
            break;
        };
        let Some(prefix) = bytes.get(..width) else {
            break;
        };
        if !std::str::from_utf8(prefix)
            .ok()
            .and_then(|s| s.chars().next())
            .is_some_and(char::is_whitespace)
        {
            break;
        }
        bytes = &bytes[width..];
    }
    bytes
}
fn parameter(input: &[u8]) -> Option<(&[u8], Vec<u8>, &[u8])> {
    let rest = trim_start(trim_start(input).strip_prefix(b";")?);
    let end = rest.iter().take_while(|b| token(**b)).count();
    if end == 0 {
        return None;
    }
    let key = &rest[..end];
    let rest = trim_start(trim_start(&rest[end..]).strip_prefix(b"=")?);
    if !rest.starts_with(b"\"") {
        let end = rest.iter().take_while(|b| token(**b)).count();
        return (end > 0).then(|| (key, rest[..end].to_vec(), &rest[end..]));
    }
    let mut value = Vec::new();
    let mut i = 1;
    while let Some(&byte) = rest.get(i) {
        match byte {
            b'"' => return Some((key, value, &rest[i + 1..])),
            b'\r' | b'\n' => return None,
            b'\\' if rest.get(i + 1).is_some_and(|b| special(*b)) => {
                i += 1;
                value.push(rest[i]);
            }
            byte => value.push(byte),
        }
        i += 1;
    }
    None
}

/// Shared validation for HTTP metadata and actual upload I/O. The fixed total
/// includes the stored user.e2b. prefix; it is not an ext4 allocation guarantee.
/// # Errors
/// Empty/oversized keys, non-printable ASCII or more than 4096 stored bytes.
pub fn validate_metadata(values: &BTreeMap<String, String>) -> Result<(), Error> {
    let mut total = 0usize;
    for (key, value) in values {
        validate_entry(key.as_bytes(), value.as_bytes())?;
        total = total
            .checked_add(9 + key.len())
            .and_then(|n| n.checked_add(value.len()))
            .filter(|n| *n <= 4096)
            .ok_or(Error::InvalidMetadata)?;
    }
    Ok(())
}
/// # Errors
/// One invalid metadata key/value; aggregate validation is separate.
pub fn validate_entry(key: &[u8], value: &[u8]) -> Result<(), Error> {
    validate_entry_detailed(key, value).map_err(|_| Error::InvalidMetadata)
}

/// Preserve the fixed key-length, key-byte, then value-byte validation order.
/// # Errors
/// The first invalid byte is an offset into the original bytes, not Unicode.
pub fn validate_entry_detailed(key: &[u8], value: &[u8]) -> Result<(), MetadataViolation> {
    if key.is_empty() {
        return Err(MetadataViolation::EmptyKey);
    }
    if key.len() > 246 {
        return Err(MetadataViolation::KeyLength);
    }
    for (bytes, key) in [(key, true), (value, false)] {
        if let Some((offset, &byte)) = bytes
            .iter()
            .enumerate()
            .find(|(_, b)| !(32..=126).contains(*b))
        {
            return Err(if key {
                MetadataViolation::KeyByte { byte, offset }
            } else {
                MetadataViolation::ValueByte { byte, offset }
            });
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests;
