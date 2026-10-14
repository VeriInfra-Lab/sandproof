//! Fixed Connect canonicalization for outer middleware, before codec selection.
use super::ProtocolError;
use crate::{
    file_mime,
    multipart::{self, MediaError},
};
use std::borrow::Cow;

/// Canonicalize an already bounded UTF-8 first Content-Type. This does not select
/// a codec or authorize a request. Invalid MIME syntax preserves the raw field,
/// as in Connect v1.18.1; decoded non-UTF-8 MIME values remain unsupported.
/// # Errors
/// Input/output budgets or unrepresentable decoded MIME parameter text.
pub fn canonical_content_type(
    value: &str,
    input_bytes: usize,
    output_bytes: usize,
) -> Result<Cow<'_, str>, ProtocolError> {
    if input_bytes == 0
        || output_bytes == 0
        || input_bytes > isize::MAX as usize / 16
        || output_bytes > isize::MAX as usize / 16
        || value.len() > input_bytes
    {
        return Err(ProtocolError::ResourceExhausted);
    }
    let raw = || {
        if value.len() > output_bytes {
            Err(ProtocolError::ResourceExhausted)
        } else {
            Ok(Cow::Borrowed(value))
        }
    };
    if value
        .bytes()
        .all(|b| b.is_ascii_lowercase() || matches!(b, b'.' | b'+' | b'-' | b'/'))
        && value.bytes().filter(|b| *b == b'/').count() == 1
    {
        return raw();
    }
    let (kind, mut params) = match multipart::media_type(value) {
        Ok(parsed) => parsed,
        Err(MediaError::NonUtf8) => return Err(ProtocolError::Unimplemented),
        Err(MediaError::Invalid) => return raw(),
    };
    if let Some(charset) = params.get_mut("charset") {
        *charset = file_mime::lower(charset);
    }
    file_mime::media_type(&kind, &params, output_bytes)
        .map(Cow::Owned)
        .map_err(|_| ProtocolError::ResourceExhausted)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_middleware_type_preserves_unknown_parameters_and_bad_syntax() {
        for (raw, expected) in [
            ("application/connect+json", "application/connect+json"),
            (
                " APPLICATION/CONNECT+JSON ; CHARSET=\"UTF-8\"",
                "application/connect+json; charset=utf-8",
            ),
            (
                "application/connect+json;z=\"a b\";a=x",
                "application/connect+json; a=x; z=\"a b\"",
            ),
            (
                "application/connect+json;x*=utf-8''%E9%9B%AA",
                "application/connect+json; x*=utf-8''%E9%9B%AA",
            ),
            (
                "application/connect+json;charset*0=UT;charset*1=F-8",
                "application/connect+json; charset=utf-8",
            ),
            (
                "application/connect+json;bad",
                "application/connect+json;bad",
            ),
            ("", ""),
        ] {
            assert_eq!(canonical_content_type(raw, 512, 512).unwrap(), expected);
            assert_eq!(
                canonical_content_type(raw, 512, expected.len().max(1)).unwrap(),
                expected
            );
            if !expected.is_empty() {
                assert!(canonical_content_type(raw, 512, expected.len() - 1).is_err());
            }
        }
        assert!(canonical_content_type("application/json", 1, 512).is_err());
        assert_eq!(
            canonical_content_type("application/connect+json;x*=utf-8''%FF", 512, 512),
            Err(ProtocolError::Unimplemented)
        );
    }
}
