#[cfg(any(target_os = "linux", test))]
use axum::http::{HeaderMap, Method};
#[cfg(any(target_os = "linux", test))]
use sandproof_guest_runtime::file_protocol::JsonContentType;

/// No public status is inferred here: fixed Connect also supports formats and
/// compression that this JSON/identity component has not yet connected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionError {
    Method,
    ContentType,
    RequestCompression,
    ResponseCompression,
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn select(
    method: &Method,
    headers: &HeaderMap,
    codec: Option<JsonContentType>,
) -> Result<JsonContentType, SelectionError> {
    if method != Method::POST {
        return Err(SelectionError::Method);
    }
    let codec = codec.ok_or(SelectionError::ContentType)?;
    let first = |field| {
        headers
            .get(field)
            .map_or(b"".as_slice(), |value| value.as_bytes())
    };
    if !matches!(first("content-encoding"), b"" | b"identity") {
        return Err(SelectionError::RequestCompression);
    }
    // Original Connect selects an exact supported name split on comma or Go
    // Unicode whitespace. It does not interpret HTTP q weights. Replacing bad
    // UTF-8 with U+FFFD cannot create a separator or a literal gzip token.
    if String::from_utf8_lossy(first("accept-encoding"))
        .split(|c: char| c == ',' || c.is_whitespace())
        .any(|name| name == "gzip")
    {
        return Err(SelectionError::ResponseCompression);
    }
    Ok(codec)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_order_and_first_compression_values() {
        let mut headers = HeaderMap::new();
        headers.append("content-encoding", "gzip".parse().unwrap());
        headers.append("accept-encoding", "gzip".parse().unwrap());
        assert_eq!(
            select(&Method::GET, &headers, None),
            Err(SelectionError::Method)
        );
        assert_eq!(
            select(&Method::POST, &headers, None),
            Err(SelectionError::ContentType)
        );
        assert_eq!(
            select(&Method::POST, &headers, Some(JsonContentType::Json)),
            Err(SelectionError::RequestCompression)
        );
        headers.insert("content-encoding", "identity".parse().unwrap());
        headers.append("content-encoding", "gzip".parse().unwrap());
        assert_eq!(
            select(&Method::POST, &headers, Some(JsonContentType::Json)),
            Err(SelectionError::ResponseCompression)
        );
        for value in ["identity", "", "br", "gzip;q=1", "GZIP"] {
            headers.insert("accept-encoding", value.parse().unwrap());
            headers.append("accept-encoding", "gzip".parse().unwrap());
            assert_eq!(
                select(&Method::POST, &headers, Some(JsonContentType::Utf8)),
                Ok(JsonContentType::Utf8)
            );
        }
        for value in ["identity,gzip", "br\tgzip", "gzip, br", "\u{a0}gzip"] {
            headers.insert(
                "accept-encoding",
                axum::http::HeaderValue::from_bytes(value.as_bytes()).unwrap(),
            );
            assert_eq!(
                select(&Method::POST, &headers, Some(JsonContentType::Json)),
                Err(SelectionError::ResponseCompression)
            );
        }
    }
}
