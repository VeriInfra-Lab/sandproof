//! HTTP header adapter for fixed file encoding selection, without I/O/routes.
//! Callers bound the complete header map BEFORE HTTP allocation/this function.
//! Upload selection/preflight precedes account/metadata work in fixed envd;
//! download selection follows successful stat/directory checks but precedes
//! open. This adapter alone cannot establish those request-stage guarantees.
use axum::http::HeaderMap;
use sandproof_guest_runtime::file_encoding as codec;
pub use sandproof_guest_runtime::file_encoding::{Encoding, Error, Limits};

fn first<'a>(headers: &'a HeaderMap, name: &str) -> &'a [u8] {
    headers.get(name).map_or(&[], |value| value.as_bytes())
}

/// Uses Go `Header.Get`'s first value, including an exactly empty first value.
/// It does not concatenate duplicate fields or reject opaque non-UTF8 bytes.
/// # Errors
/// Component budgets/configuration or unsupported content coding. The pinned
/// upload endpoint reports unsupported coding as HTTP 400 (not 415).
pub fn content(headers: &HeaderMap, limits: Limits) -> Result<Encoding, Error> {
    codec::content(first(headers, "content-encoding"), limits)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DownloadError {
    pub cause: Error,
    /// Fixed envd sets Vary only AFTER parsing Accept-Encoding successfully.
    /// A later conditional-identity rejection retains Vary; parse rejection
    /// does not. Component-budget failures here occur before that stage.
    pub vary_accept_encoding: bool,
}

/// On success caller sets Vary: Accept-Encoding. For errors caller preserves
/// the returned Vary stage. `NotAcceptable` maps to 406; component budgets need
/// the service's separately frozen resource-error mapping.
/// # Errors
/// Accept-Encoding parsing/budgets or identity forbidden for Range/conditions.
/// Successful identity selection still requires real `ServeContent` semantics.
pub fn download(headers: &HeaderMap, limits: Limits) -> Result<Encoding, DownloadError> {
    let accepted = codec::accept(first(headers, "accept-encoding"), limits).map_err(|cause| {
        DownloadError {
            cause,
            vary_accept_encoding: false,
        }
    })?;
    let conditional = ["range", "if-modified-since", "if-none-match", "if-range"]
        .iter()
        .any(|name| !first(headers, name).is_empty());
    if conditional {
        if !accepted.identity_acceptable {
            return Err(DownloadError {
                cause: Error::NotAcceptable,
                vary_accept_encoding: true,
            });
        }
        Ok(Encoding::Identity)
    } else {
        Ok(accepted.encoding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderName, HeaderValue};
    fn limits() -> Limits {
        Limits {
            header_bytes: 4096,
            items: 64,
        }
    }

    #[test]
    fn duplicate_values_use_the_first_even_when_empty_and_opaque_values_are_not_lost() {
        let mut headers = HeaderMap::new();
        headers.append("content-encoding", HeaderValue::from_static(""));
        headers.append("content-encoding", HeaderValue::from_static("gzip"));
        headers.append("accept-encoding", HeaderValue::from_static(""));
        headers.append("accept-encoding", HeaderValue::from_static("identity;q=0"));
        assert_eq!(content(&headers, limits()), Ok(Encoding::Identity));
        assert_eq!(download(&headers, limits()), Ok(Encoding::Identity));
        headers.insert(
            "content-encoding",
            HeaderValue::from_bytes(b"gzip\xff").unwrap(),
        );
        headers.insert("accept-encoding", HeaderValue::from_bytes(b"\xff").unwrap());
        assert_eq!(content(&headers, limits()), Err(Error::Unsupported));
        assert_eq!(download(&headers, limits()), Ok(Encoding::Identity));
        headers.insert(
            "content-encoding",
            HeaderValue::from_bytes("\u{a0}gzİp\u{3000};q=0".as_bytes()).unwrap(),
        );
        assert_eq!(content(&headers, limits()), Ok(Encoding::Gzip));
    }

    #[test]
    fn only_the_original_four_nonempty_condition_fields_force_identity() {
        for name in [
            "range",
            "if-modified-since",
            "if-none-match",
            "if-range",
            "if-match",
            "if-unmodified-since",
        ] {
            for value in ["", " ", "invalid", "bytes=0-1"] {
                let mut headers = HeaderMap::new();
                headers.insert("accept-encoding", HeaderValue::from_static("gzip"));
                headers.insert(
                    HeaderName::from_static(name),
                    HeaderValue::from_static(value),
                );
                let forced =
                    !value.is_empty() && !["if-match", "if-unmodified-since"].contains(&name);
                assert_eq!(
                    download(&headers, limits()),
                    Ok(if forced {
                        Encoding::Identity
                    } else {
                        Encoding::Gzip
                    }),
                    "{name}: {value}"
                );
            }
        }
        let mut headers = HeaderMap::new();
        headers.insert("accept-encoding", HeaderValue::from_static("gzip"));
        headers.append("range", HeaderValue::from_static(""));
        headers.append("range", HeaderValue::from_static("bytes=0-1"));
        assert_eq!(download(&headers, limits()), Ok(Encoding::Gzip));
    }

    #[test]
    fn vary_tracks_parse_versus_conditional_rejection_without_hiding_budget_failures() {
        let mut headers = HeaderMap::new();
        headers.insert("accept-encoding", HeaderValue::from_static("identity;q=0"));
        headers.insert("range", HeaderValue::from_static("bytes=0-1"));
        assert_eq!(
            download(&headers, limits()),
            Err(DownloadError {
                cause: Error::NotAcceptable,
                vary_accept_encoding: false
            })
        );
        headers.insert(
            "accept-encoding",
            HeaderValue::from_static("gzip,identity;q=0"),
        );
        assert_eq!(
            download(&headers, limits()),
            Err(DownloadError {
                cause: Error::NotAcceptable,
                vary_accept_encoding: true
            })
        );
        assert_eq!(
            download(
                &headers,
                Limits {
                    items: 1,
                    ..limits()
                }
            ),
            Err(DownloadError {
                cause: Error::ItemLimit,
                vary_accept_encoding: false
            })
        );
    }
}
