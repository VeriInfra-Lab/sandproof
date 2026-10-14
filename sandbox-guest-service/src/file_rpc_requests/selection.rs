use axum::http::HeaderMap;
#[cfg(any(target_os = "linux", test))]
use axum::http::Method;
#[cfg(any(target_os = "linux", test))]
use sandproof_guest_runtime::file_protocol::JsonContentType;

/// No public status is inferred here: other formats and the fixed unsupported
/// compression error response have not yet been connected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionError {
    Method,
    ContentType,
    RequestCompression,
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
    if !matches!(first("content-encoding"), b"" | b"identity" | b"gzip") {
        return Err(SelectionError::RequestCompression);
    }
    Ok(codec)
}

pub(super) fn is_gzip_request(headers: &HeaderMap) -> bool {
    headers.get("content-encoding").is_some_and(|v| v == "gzip")
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn response_encoding(headers: &HeaderMap) -> crate::file_rpc_response::Encoding {
    // Original Connect splits only on comma or ASCII space, not tabs or Unicode
    // whitespace. It compares exact names and does not interpret HTTP q weights.
    if is_gzip_request(headers)
        || headers
            .get("accept-encoding")
            .map_or(b"".as_slice(), |v| v.as_bytes())
            .split(|c| matches!(c, b',' | b' '))
            .any(|name| name == b"gzip")
    {
        crate::file_rpc_response::Encoding::Gzip
    } else {
        crate::file_rpc_response::Encoding::Identity
    }
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
            Ok(JsonContentType::Json)
        );
        for value in ["identity", "br", "gzip;q=0", ""] {
            headers.insert("accept-encoding", value.parse().unwrap());
            assert_eq!(
                response_encoding(&headers),
                crate::file_rpc_response::Encoding::Gzip
            );
        }
        for value in ["GZIP", "gzip,identity", "gzip;q=1", "br"] {
            headers.insert("content-encoding", value.parse().unwrap());
            headers.append("content-encoding", "gzip".parse().unwrap());
            assert_eq!(
                select(&Method::POST, &headers, Some(JsonContentType::Json)),
                Err(SelectionError::RequestCompression)
            );
            assert!(!is_gzip_request(&headers));
        }
        headers.insert("content-encoding", "identity".parse().unwrap());
        headers.append("content-encoding", "gzip".parse().unwrap());
        assert_eq!(
            select(&Method::POST, &headers, Some(JsonContentType::Json)),
            Ok(JsonContentType::Json)
        );
        for value in [
            "identity",
            "",
            "br",
            "gzip;q=1",
            "GZIP",
            "br\tgzip",
            "\u{85}gzip",
            "\u{a0}gzip",
        ] {
            headers.insert(
                "accept-encoding",
                axum::http::HeaderValue::from_bytes(value.as_bytes()).unwrap(),
            );
            headers.append("accept-encoding", "gzip".parse().unwrap());
            assert_eq!(
                select(&Method::POST, &headers, Some(JsonContentType::Utf8)),
                Ok(JsonContentType::Utf8)
            );
            assert_eq!(
                response_encoding(&headers),
                crate::file_rpc_response::Encoding::Identity
            );
        }
        for value in ["identity,gzip", "br gzip", "gzip, br"] {
            headers.insert(
                "accept-encoding",
                axum::http::HeaderValue::from_bytes(value.as_bytes()).unwrap(),
            );
            assert_eq!(
                select(&Method::POST, &headers, Some(JsonContentType::Json)),
                Ok(JsonContentType::Json)
            );
            assert_eq!(
                response_encoding(&headers),
                crate::file_rpc_response::Encoding::Gzip
            );
        }
    }
}
