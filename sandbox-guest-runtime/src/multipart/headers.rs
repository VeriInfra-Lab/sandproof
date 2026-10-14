// MIME parameter/header behavior adapted from Go 1.26.8 mime and net/textproto.
// Copyright 2010 The Go Authors. All rights reserved. BSD terms are in GO-LICENSE.
use super::Error;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, PartialEq, Eq)]
pub struct Part {
    pub name: Option<String>,
    /// Full filename, deliberately not basename; None differs from empty.
    pub filename: Option<String>,
    pub quoted_printable: bool,
}

/// Fixed `Request.MultipartReader` / empty-boundary `NextPart` rejections. None of
/// these reads the request body. Resource and text limitations stay separate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryRejection {
    NotMultipart,
    MissingBoundary,
    EmptyBoundary,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryError {
    Rejected(BoundaryRejection),
    Decode(Error),
}

/// Parse a bounded Content-Type value. `Decoder::new` separately limits boundary
/// bytes. Parameter quoting/continuations follow the fixed Go MIME contract.
/// # Errors
/// Oversized header, non-multipart type, bad parameters or missing boundary.
pub fn boundary(content_type: &str, header_bytes: usize) -> Result<String, Error> {
    boundary_detailed(content_type, header_bytes).map_err(|error| match error {
        BoundaryError::Rejected(_) => Error::Malformed,
        BoundaryError::Decode(error) => error,
    })
}

/// Preserve the original public failure stage before body polling. An explicit
/// empty boundary passes Go `MultipartReader` but fails its first `NextPart`; both
/// happen before any body read. This component still rejects non-UTF-8 decoded
/// parameters without inventing a corresponding Go public error.
/// # Errors
/// Oversized header, unsupported text, invalid type/parameters, absent/empty boundary.
pub fn boundary_detailed(content_type: &str, header_bytes: usize) -> Result<String, BoundaryError> {
    if content_type.len() > header_bytes {
        return Err(BoundaryError::Decode(Error::ResourceExhausted));
    }
    let (kind, mut params) = media_type(content_type).map_err(|error| match error {
        MediaError::Invalid => BoundaryError::Rejected(BoundaryRejection::NotMultipart),
        MediaError::NonUtf8 => BoundaryError::Decode(Error::Malformed),
    })?;
    // Request.MultipartReader in the fixed Go HTTP handler accepts both.
    if kind != "multipart/form-data" && kind != "multipart/mixed" {
        return Err(BoundaryError::Rejected(BoundaryRejection::NotMultipart));
    }
    let value = params
        .remove("boundary")
        .ok_or(BoundaryError::Rejected(BoundaryRejection::MissingBoundary))?;
    if value.is_empty() {
        return Err(BoundaryError::Rejected(BoundaryRejection::EmptyBoundary));
    }
    Ok(value)
}

// Construct semantic fields only after header_error::inspect validates this
// complete, byte/count-bounded block. UTF-8 parameter limitations remain local.
pub(super) fn parse(raw: &[u8]) -> Result<Part, Error> {
    let mut headers: Vec<(String, Vec<u8>)> = Vec::new();
    for line in raw.split_inclusive(|b| *b == b'\n') {
        let line = line.strip_suffix(b"\n").ok_or(Error::Truncated)?;
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            break;
        }
        if matches!(line[0], b' ' | b'\t') {
            let (_, value) = headers.last_mut().ok_or(Error::Malformed)?;
            value.push(b' ');
            value.extend_from_slice(trim_space(line));
            continue;
        }
        let colon = line
            .iter()
            .position(|b| *b == b':')
            .ok_or(Error::Malformed)?;
        let key =
            String::from_utf8(line[..colon].to_ascii_lowercase()).map_err(|_| Error::Malformed)?;
        headers.push((key, trim_space(&line[colon + 1..]).to_vec()));
    }
    let first = |name: &str| {
        headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_slice())
    };
    let quoted_printable = first("content-transfer-encoding")
        .is_some_and(|v| v.eq_ignore_ascii_case(b"quoted-printable"));
    let mut part = Part {
        name: None,
        filename: None,
        quoted_printable,
    };
    if let Some(value) = first("content-disposition") {
        let value = std::str::from_utf8(value).map_err(|_| Error::Malformed)?;
        match media_type(value) {
            Ok((kind, mut params)) => {
                if kind == "form-data" {
                    part.name = params.remove("name");
                }
                part.filename = params.remove("filename");
            }
            // FormName ignores malformed dispositions in the fixed Go parser.
            Err(MediaError::Invalid) => {}
            Err(MediaError::NonUtf8) => return Err(Error::Malformed),
        }
    }
    Ok(part)
}

pub(super) fn trim_space(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t'))
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| !matches!(b, b' ' | b'\t'))
        .map_or(start, |n| n + 1);
    &bytes[start..end]
}
pub(super) fn header_name(b: u8) -> bool {
    // Go's MIME reader also accepts spaces in unknown field names. Such a key
    // cannot match either recognized name, so it remains an ignored field.
    b == b' ' || b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}
fn special(b: u8) -> bool {
    b"()<>@,;:\\\"/[]?=".contains(&b)
}
fn token(b: u8) -> bool {
    b > b' ' && b < 127 && !special(b)
}

#[derive(Debug)]
pub(crate) enum MediaError {
    Invalid,
    NonUtf8,
}

fn consume_token(input: &str) -> Result<(&str, &str), MediaError> {
    let n = input.bytes().take_while(|b| token(*b)).count();
    if n == 0 {
        return Err(MediaError::Invalid);
    }
    Ok((&input[..n], &input[n..]))
}
fn consume_value(input: &str) -> Result<(String, &str), MediaError> {
    if !input.starts_with('"') {
        let (value, rest) = consume_token(input)?;
        return Ok((value.into(), rest));
    }
    let bytes = input.as_bytes();
    let mut value = Vec::new();
    let mut n = 1;
    while n < bytes.len() {
        match bytes[n] {
            b'"' => {
                return Ok((
                    String::from_utf8(value).map_err(|_| MediaError::NonUtf8)?,
                    &input[n + 1..],
                ));
            }
            b'\r' | b'\n' => return Err(MediaError::Invalid),
            b'\\' if bytes.get(n + 1).is_some_and(|b| special(*b)) => {
                n += 1;
                value.push(bytes[n]);
            }
            b => value.push(b),
        }
        n += 1;
    }
    Err(MediaError::Invalid)
}

pub(crate) fn media_type(value: &str) -> Result<(String, BTreeMap<String, String>), MediaError> {
    let (kind, mut rest) = value.find(';').map_or((value, ""), |n| value.split_at(n));
    let kind = kind.trim().to_ascii_lowercase();
    let pieces: Vec<_> = kind.split('/').collect();
    if pieces.len() > 2 || pieces.iter().any(|p| p.is_empty() || !p.bytes().all(token)) {
        return Err(MediaError::Invalid);
    }
    let mut params = BTreeMap::<String, String>::new();
    let mut extended = BTreeMap::<String, String>::new();
    while !rest.trim_start().is_empty() {
        rest = rest
            .trim_start()
            .strip_prefix(';')
            .ok_or(MediaError::Invalid)?
            .trim_start();
        if rest.is_empty() {
            break;
        }
        let (key, after) = consume_token(rest)?;
        let key = key.to_ascii_lowercase();
        rest = after
            .trim_start()
            .strip_prefix('=')
            .ok_or(MediaError::Invalid)?
            .trim_start();
        let (value, after) = consume_value(rest)?;
        let target = if key.contains('*') {
            &mut extended
        } else {
            &mut params
        };
        if target.get(&key).is_some_and(|old| *old != value) {
            return Err(MediaError::Invalid);
        }
        target.insert(key, value);
        rest = after;
    }
    let bases: BTreeSet<_> = extended
        .keys()
        .filter_map(|k| k.split_once('*').map(|(base, _)| base.to_owned()))
        .collect();
    for base in bases {
        if let Some(value) = extended.get(&format!("{base}*")) {
            if let Some(decoded) = extended_value(value) {
                params.insert(
                    base,
                    String::from_utf8(decoded).map_err(|_| MediaError::NonUtf8)?,
                );
            }
            continue;
        }
        let mut joined = Vec::new();
        let mut seen = false;
        // Every successful iteration consumes a distinct stored parameter.
        for index in 0..=extended.len() {
            if let Some(value) = extended.get(&format!("{base}*{index}")) {
                joined.extend_from_slice(value.as_bytes());
                seen = true;
                continue;
            }
            let Some(value) = extended.get(&format!("{base}*{index}*")) else {
                break;
            };
            seen = true;
            let decoded = if index == 0 {
                extended_value(value)
            } else {
                percent_decode(value)
            };
            if let Some(decoded) = decoded {
                joined.extend(decoded);
            }
        }
        if seen {
            params.insert(
                base,
                String::from_utf8(joined).map_err(|_| MediaError::NonUtf8)?,
            );
        }
    }
    Ok((kind, params))
}

fn extended_value(value: &str) -> Option<Vec<u8>> {
    let (charset, rest) = value.split_once('\'')?;
    if !charset.eq_ignore_ascii_case("utf-8") && !charset.eq_ignore_ascii_case("us-ascii") {
        return None;
    }
    let (_, content) = rest.split_once('\'')?;
    percent_decode(content)
}
fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
fn percent_decode(value: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(value.len());
    let mut input = value.as_bytes().iter().copied();
    while let Some(b) = input.next() {
        if b != b'%' {
            bytes.push(b);
            continue;
        }
        let pair = input.next().and_then(hex).zip(input.next().and_then(hex));
        let (high, low) = pair?;
        bytes.push(high * 16 + low);
    }
    Some(bytes)
}
