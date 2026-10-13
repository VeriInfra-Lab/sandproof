// Adapted from Go 1.26.8 mime/mediatype.go and internal/filepathlite/path.go.
// Copyright 2010 The Go Authors. BSD terms: ../multipart/GO-LICENSE.
use super::Error;
use std::collections::BTreeMap;

/// Linux filepath.Base, including empty, root and trailing slashes. Backslash
/// is an ordinary filename byte; this function does not clean dot components.
#[must_use]
pub fn basename(path: &[u8]) -> &[u8] {
    if path.is_empty() {
        return b".";
    }
    let end = path.iter().rposition(|b| *b != b'/').map_or(0, |n| n + 1);
    let path = &path[..end];
    if path.is_empty() {
        return b"/";
    }
    path.rsplit(|b| *b == b'/').next().unwrap_or(path)
}
/// Linux filepath.Ext of the original name, not a cleaned or resolved path.
#[must_use]
pub fn extension(path: &str) -> &str {
    let last = path.rsplit('/').next().unwrap_or(path);
    last.rfind('.').map_or("", |index| &last[index..])
}
fn special(b: u8) -> bool {
    b"()<>@,;:\\\"/[]?=".contains(&b)
}
fn token(value: &[u8]) -> bool {
    !value.is_empty() && value.iter().all(|b| *b > b' ' && *b < 127 && !special(*b))
}
fn encoded(value: &[u8]) -> bool {
    value
        .iter()
        .any(|b| (*b < b' ' || *b > b'~') && *b != b'\t')
}
fn escape(b: u8) -> bool {
    b <= b' ' || b >= 127 || matches!(b, b'*' | b'\'' | b'%') || special(b)
}
fn value_length(value: &[u8]) -> Option<usize> {
    if encoded(value) {
        value.iter().try_fold(9_usize, |n, b| {
            n.checked_add(if escape(*b) { 3 } else { 1 })
        })
    } else if token(value) {
        Some(value.len())
    } else {
        value.iter().try_fold(2_usize, |n, b| {
            n.checked_add(if matches!(b, b'"' | b'\\') { 2 } else { 1 })
        })
    }
}
fn append_value(out: &mut Vec<u8>, value: &[u8]) {
    if encoded(value) {
        out.extend(b"*=utf-8''");
        for b in value {
            if escape(*b) {
                out.extend([
                    b'%',
                    b"0123456789ABCDEF"[usize::from(b >> 4)],
                    b"0123456789ABCDEF"[usize::from(b & 15)],
                ]);
            } else {
                out.push(*b);
            }
        }
    } else {
        out.push(b'=');
        if token(value) {
            out.extend(value);
            return;
        }
        out.push(b'"');
        for b in value {
            if matches!(b, b'"' | b'\\') {
                out.push(b'\\');
            }
            out.push(*b);
        }
        out.push(b'"');
    }
}
/// Exactly the endpoint's inline/filename formatter. Raw non-ASCII/control
/// bytes use uppercase percent encoding, not a lossy UTF-8 conversion.
/// # Errors
/// Invalid budgets or input/output limit, checked before allocating output.
pub fn disposition(path: &[u8], path_bytes: usize, header_bytes: usize) -> Result<Vec<u8>, Error> {
    if path_bytes == 0 || header_bytes == 0 || header_bytes > isize::MAX as usize / 16 {
        return Err(Error::InvalidConfiguration);
    }
    if path.len() > path_bytes {
        return Err(Error::InputLimit);
    }
    let value = basename(path);
    let length = 16_usize
        .checked_add(value_length(value).ok_or(Error::RetainedLimit)?)
        .and_then(|n| n.checked_add(usize::from(!encoded(value))))
        .filter(|n| *n <= header_bytes)
        .ok_or(Error::RetainedLimit)?;
    let mut out = Vec::with_capacity(length);
    out.extend(b"inline; filename");
    append_value(&mut out, value);
    Ok(out)
}
pub(super) fn media_type(
    kind: &str,
    params: &BTreeMap<String, String>,
    limit: usize,
) -> Result<String, Error> {
    if kind.split('/').count() > 2
        || kind.split('/').any(|part| !token(part.as_bytes()))
        || params.keys().any(|key| !token(key.as_bytes()))
    {
        return Ok(String::new());
    }
    let length = params
        .iter()
        .try_fold(kind.len(), |n, (key, value)| {
            n.checked_add(2)?
                .checked_add(key.len())?
                .checked_add(value_length(value.as_bytes())?)?
                .checked_add(usize::from(!encoded(value.as_bytes())))
        })
        .filter(|n| *n <= limit)
        .ok_or(Error::RetainedLimit)?;
    let mut out = Vec::with_capacity(length);
    out.extend(kind.to_ascii_lowercase().as_bytes());
    for (key, value) in params {
        out.extend(b"; ");
        out.extend(key.to_ascii_lowercase().as_bytes());
        append_value(&mut out, value.as_bytes());
    }
    String::from_utf8(out).map_err(|_| Error::UnsupportedText)
}
