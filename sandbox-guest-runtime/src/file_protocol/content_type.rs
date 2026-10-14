//! Select the two JSON codecs registered by fixed Connect v1.18.1.
//! Reuse the bounded Go MIME parameter parser, including RFC 2231 continuations.
use crate::{file_mime, multipart, protocol::ProtocolError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsonContentType {
    Json,
    Utf8,
}
impl JsonContentType {
    #[must_use]
    pub const fn canonical(self) -> &'static str {
        match self {
            Self::Json => "application/json",
            Self::Utf8 => "application/json; charset=utf-8",
        }
    }
}

/// Select a JSON codec from an already parsed UTF-8 HTTP field. The total
/// request-header budget must be enforced separately, including duplicate and
/// ignored fields. None means this component cannot select a JSON codec, not
/// that the original handler would reject every other protocol with HTTP 415.
/// # Errors
/// Invalid or exceeded header budget, before MIME parameter allocation.
pub fn json_content_type(
    value: &str,
    limit: usize,
) -> Result<Option<JsonContentType>, ProtocolError> {
    if limit == 0 || limit > isize::MAX as usize / 16 || value.len() > limit {
        return Err(ProtocolError::ResourceExhausted);
    }
    let Ok((kind, params)) = multipart::media_type(value) else {
        return Ok(None);
    };
    if kind != "application/json" {
        return Ok(None);
    }
    Ok(if params.is_empty() {
        Some(JsonContentType::Json)
    } else if params.len() == 1
        && params
            .get("charset")
            .is_some_and(|v| file_mime::lower(v) == "utf-8")
    {
        Some(JsonContentType::Utf8)
    } else {
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn json_aliases_parameters_and_boundaries() {
        for (value, expected) in [
            ("application/json", Some(JsonContentType::Json)),
            (" APPLİCATİON/JSON ; ", Some(JsonContentType::Json)),
            (
                "application/json;ignored*=unknown''ignored",
                Some(JsonContentType::Json),
            ),
            (
                "application/json; CHARSET=\"UTF-8\"",
                Some(JsonContentType::Utf8),
            ),
            (
                "application/json;charset*=utf-8''UTF%2D8",
                Some(JsonContentType::Utf8),
            ),
            (
                "application/json;charset*0=ut;charset*1=f-8",
                Some(JsonContentType::Utf8),
            ),
            (
                "application/json;charset=utf-8;charset=utf-8",
                Some(JsonContentType::Utf8),
            ),
            ("application/json;charset=UTF-8;charset=utf-8", None),
            ("application/json;charset=utf-8;extra=x", None),
            ("application/json;charset=utf-8;bad", None),
            ("application/json;charset=utf-8;;", None),
            ("application/json;charset=latin1", None),
            ("application/proto", None),
            ("application/grpc+json", None),
            ("", None),
        ] {
            assert_eq!(json_content_type(value, 256).unwrap(), expected, "{value}");
        }
        let value = "application/json";
        assert_eq!(
            json_content_type(value, value.len()).unwrap(),
            Some(JsonContentType::Json)
        );
        assert_eq!(
            json_content_type(value, value.len() - 1),
            Err(ProtocolError::ResourceExhausted)
        );
        assert!(json_content_type(value, 0).is_err());
        assert!(json_content_type(value, usize::MAX).is_err());
    }
}
