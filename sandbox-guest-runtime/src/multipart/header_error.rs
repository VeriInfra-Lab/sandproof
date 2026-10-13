//! Fixed Go MIME header diagnostics, observed before constructing a Part.
// Adapted from Go 1.26.8 net/textproto (Copyright 2010 The Go Authors).
// All rights reserved. BSD terms in GO-LICENSE.
use super::headers::{header_name, trim_space};
use super::{Error, ReadError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeaderError {
    InitialLine(Vec<u8>),
    /// Go bounds its initial-indentation diagnostic to 80 source bytes.
    InitialLineTooLong,
    MissingColon(Vec<u8>),
    InvalidLine(Vec<u8>),
}

/// Inspect complete logical headers plus any available physical line. The
/// caller checks the raw header-byte budget first. No allocation on success or
/// incomplete input; a diagnostic retains at most the admitted raw byte count.
pub(super) fn inspect(raw: &[u8], count_limit: usize, eof: bool) -> Result<(), ReadError> {
    let mut offset = 0;
    let mut count = 0;
    while let Some((line, size)) = physical(&raw[offset..], eof) {
        if line.is_empty() {
            return Ok(());
        }
        if offset == 0 && matches!(line[0], b' ' | b'\t') {
            return Err(ReadError::Headers(if line.len() > 80 {
                HeaderError::InitialLineTooLong
            } else {
                HeaderError::InitialLine(line.to_vec())
            }));
        }
        let Some(colon) = line.iter().position(|b| *b == b':') else {
            return Err(ReadError::Headers(HeaderError::MissingColon(line.to_vec())));
        };
        let first = trim_space(line);
        let key = trim_space(&line[..colon]);
        let valid_key = !key.is_empty() && line[..colon].iter().all(|b| header_name(*b));
        let canonicalize = valid_key && !line[..colon].contains(&b' ');
        let mut invalid = !valid_key || !line[colon + 1..].iter().all(|b| value_byte(*b));
        let start = offset;
        offset += size;
        loop {
            let rest = &raw[offset..];
            if rest.is_empty() {
                if !eof {
                    return Ok(());
                }
                break;
            }
            if !matches!(rest[0], b' ' | b'\t') {
                break;
            }
            // skipSpace followed by ReadLine: an EOF after only spaces still
            // appends one folding space to the diagnostic.
            let Some((continued, size)) = physical(rest, eof) else {
                return Ok(());
            };
            invalid |= !continued.iter().all(|b| value_byte(*b));
            offset += size;
        }
        if invalid {
            return Err(ReadError::Headers(HeaderError::InvalidLine(normalized(
                &raw[start..offset],
                first,
                canonicalize,
            ))));
        }
        if count == count_limit {
            return Err(ReadError::Decode(Error::ResourceExhausted));
        }
        count += 1;
    }
    Ok(())
}
fn value_byte(byte: u8) -> bool {
    byte == b'\t' || byte >= b' ' && byte != 127
}
fn physical(bytes: &[u8], eof: bool) -> Option<(&[u8], usize)> {
    if let Some(n) = bytes.iter().position(|b| *b == b'\n') {
        let line = &bytes[..n];
        Some((line.strip_suffix(b"\r").unwrap_or(line), n + 1))
    } else if eof && !bytes.is_empty() {
        Some((bytes, bytes.len()))
    } else {
        None
    }
}
fn normalized(span: &[u8], first: &[u8], canonicalize: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(span.len());
    out.extend_from_slice(first);
    // Go canonicalizes a valid token key in-place before checking values.
    // A bad value therefore quotes the canonical key, but a key with spaces
    // or an invalid key remains exactly as received.
    if canonicalize {
        let colon = out
            .iter()
            .position(|b| *b == b':')
            .expect("validated colon");
        let mut upper = true;
        for byte in &mut out[..colon] {
            *byte = if upper {
                byte.to_ascii_uppercase()
            } else {
                byte.to_ascii_lowercase()
            };
            upper = *byte == b'-';
        }
    }
    for line in span.split_inclusive(|b| *b == b'\n').skip(1) {
        let line = if let Some(line) = line.strip_suffix(b"\n") {
            line.strip_suffix(b"\r").unwrap_or(line)
        } else {
            line
        };
        out.push(b' ');
        out.extend_from_slice(trim_space(line));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostic_waits_for_logical_line_and_retains_original_validation_order() {
        assert_eq!(inspect(b"bad<: value\r\n", 1, false), Ok(()));
        assert_eq!(inspect(b"bad<: value\r\n  fold", 1, false), Ok(()));
        assert_eq!(
            inspect(b"bad<: value\r\n  fold\r\nN", 1, false),
            Err(ReadError::Headers(HeaderError::InvalidLine(
                b"bad<: value fold".to_vec()
            )))
        );
        assert_eq!(
            inspect(b"NoColon \t\r\n", 1, false),
            Err(ReadError::Headers(HeaderError::MissingColon(
                b"NoColon \t".to_vec()
            )))
        );
        assert_eq!(
            inspect(b" x: value\r\n", 1, false),
            Err(ReadError::Headers(HeaderError::InitialLine(
                b" x: value".to_vec()
            )))
        );
        assert_eq!(
            inspect(b"x-key: bad\0\r\n \t", 1, true),
            Err(ReadError::Headers(HeaderError::InvalidLine(
                b"X-Key: bad\0 ".to_vec()
            )))
        );
        assert_eq!(
            inspect(b"x key: bad\0\r\n\r\n", 1, false),
            Err(ReadError::Headers(HeaderError::InvalidLine(
                b"x key: bad\0".to_vec()
            )))
        );
        assert_eq!(
            inspect(b"x-key: valid\r\n\r\n", 0, false),
            Err(ReadError::Decode(Error::ResourceExhausted))
        );
        assert!(matches!(
            inspect(b"x-key: bad\0\r\n\r\n", 0, false),
            Err(ReadError::Headers(_))
        ));
    }
}
