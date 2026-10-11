//! The one original envd route allowlist, shared on both authenticated hops.
use super::{Body, Method, Request, StatusCode};
use percent_encoding::percent_decode_str;
use sandproof_protocol::DataKind;

fn operation(method: &Method, path: &str) -> Option<(DataKind, bool)> {
    if path == "/health" && method == Method::GET {
        return Some((DataKind::Exec, false));
    }
    if path.starts_with("/process.Process/") && method == Method::POST {
        return Some((DataKind::Exec, false));
    }
    if path == "/files" {
        return match *method {
            Method::GET => Some((DataKind::FileRead, true)),
            Method::POST | Method::DELETE => Some((DataKind::FileWrite, true)),
            _ => None,
        };
    }
    if let Some(name) = path.strip_prefix("/filesystem.Filesystem/") {
        if method != Method::POST {
            return None;
        }
        let kind = match name {
            "ListDir" | "Stat" | "ReadFile" | "GetInfo" => DataKind::FileRead,
            "MakeDir" | "Remove" | "Move" | "WriteFile" | "Copy" => DataKind::FileWrite,
            _ => return None,
        };
        return Some((kind, true));
    }
    None
}

/// Pure parsing of the original SDK method/URI, never resource authorization.
/// # Errors
/// Rejects unsupported routes, oversized URIs, file NULs and invalid UTF-8.
pub fn request_kind(request: &Request<Body>) -> Result<(DataKind, bool), StatusCode> {
    let (kind, file_request) =
        operation(request.method(), request.uri().path()).ok_or(StatusCode::NOT_FOUND)?;
    let raw_url = request
        .uri()
        .path_and_query()
        .map_or("", |value| value.as_str());
    if raw_url.len() > 4096 {
        return Err(StatusCode::URI_TOO_LONG);
    }
    if file_request
        && (percent_decode_str(raw_url).decode_utf8().is_err()
            || percent_decode_str(raw_url)
                .decode_utf8()
                .is_ok_and(|decoded| decoded.contains('\0')))
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok((kind, file_request))
}

/// One bearer header only; duplicate headers cannot select differing tenants.
/// # Errors
/// Missing, duplicated or non-text credentials return 401.
pub fn access_token(headers: &axum::http::HeaderMap) -> Result<String, StatusCode> {
    let mut tokens = headers.get_all("x-access-token").iter();
    let token = tokens.next().ok_or(StatusCode::UNAUTHORIZED)?;
    if tokens.next().is_some() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    token
        .to_str()
        .map(str::to_owned)
        .map_err(|_| StatusCode::UNAUTHORIZED)
}
