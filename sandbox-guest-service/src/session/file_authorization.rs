use super::{AuthPermit, MetadataSource, Session, token_digest};
use crate::file_authorization::{Authorization, Denial, Error, Limits, Operation};
use axum::http::HeaderMap;
use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
use sandproof_guest_runtime::file_query::Query;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::sync::OwnedRwLockReadGuard;
use tokio::time::{Instant, timeout_at};
use zeroize::Zeroizing;

#[cfg(test)]
mod tests;

impl<M: MetadataSource> Session<M> {
    /// Bind the original decoded file query to one current authorized snapshot.
    /// Nonempty access token takes precedence; otherwise validate the fixed
    /// v1 signature and then its expiry. Pre-init and duplicate tokens fail
    /// closed under the existing protected Sandproof initialization policy.
    /// Caller owns request admission; this does not create a request queue.
    /// # Errors
    /// Invalid budgets/bytes, original timeout, unavailable clock or auth denial.
    pub async fn authorize_file(
        &self,
        headers: &HeaderMap,
        query: Query,
        operation: Operation,
        limits: Limits,
        deadline: Instant,
    ) -> Result<Authorization, Error> {
        self.authorize_file_clock(
            headers,
            query,
            operation,
            limits,
            (deadline, || {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()
                    .and_then(|v| i64::try_from(v.as_secs()).ok())
                    .ok_or(Error::ClockUnavailable)
            }),
        )
        .await
    }

    async fn authorize_file_clock(
        &self,
        headers: &HeaderMap,
        mut query: Query,
        operation: Operation,
        limits: Limits,
        time: (Instant, impl FnOnce() -> Result<i64, Error>),
    ) -> Result<Authorization, Error> {
        let (deadline, now) = time;
        if Instant::now() >= deadline {
            return Err(Error::DeadlineExceeded);
        }
        crate::file_upload::check_headers(headers, limits.headers).map_err(
            |error| match error {
                sandproof_guest_runtime::file_upload::Error::InvalidConfiguration => {
                    Error::InvalidConfiguration
                }
                _ => Error::ResourceExhausted,
            },
        )?;
        if limits.query_bytes == 0 || limits.query_bytes > isize::MAX as usize / 16 {
            return Err(Error::InvalidConfiguration);
        }
        [&query.path, &query.username, &query.signature]
            .into_iter()
            .try_fold(0usize, |n, value| {
                n.checked_add(value.as_ref().map_or(0, Vec::len))
                    .filter(|n| *n <= limits.query_bytes)
                    .ok_or(Error::ResourceExhausted)
            })?;
        let mut tokens = headers.get_all("x-access-token").iter();
        let token = tokens
            .next()
            .map(axum::http::HeaderValue::as_bytes)
            .unwrap_or_default();
        if tokens.next().is_some() {
            return Err(Error::Denied(Denial::DuplicateToken));
        }
        let guard = timeout_at(deadline, Arc::clone(&self.shared.current).read_owned())
            .await
            .map_err(|_| Error::DeadlineExceeded)?;
        let guard = OwnedRwLockReadGuard::try_map(guard, Option::as_ref)
            .map_err(|_| Error::Denied(Denial::Uninitialized))?;
        if token.is_empty() {
            let supplied = query
                .signature
                .as_deref()
                .ok_or(Error::Denied(Denial::MissingSignature))?;
            let expected = signature(guard.token.0.as_bytes(), &query, operation);
            if !bool::from(expected.as_slice().ct_eq(supplied)) {
                return Err(Error::Denied(Denial::InvalidSignature));
            }
            if let Some(expiration) = query.signature_expiration {
                // Read wall time only after acquiring the current snapshot and
                // validating the signature; time spent waiting is not refunded.
                if expiration < now()? {
                    return Err(Error::Denied(Denial::ExpiredSignature));
                }
            }
        } else {
            let digest = std::str::from_utf8(token)
                .ok()
                .and_then(|token| token_digest(token, self.limits.token_bytes).ok());
            if !digest.is_some_and(|digest| bool::from(guard.hash.ct_eq(&digest))) {
                return Err(Error::Denied(Denial::TokenMismatch));
            }
        }
        // A valid token ignores signature bytes, including non-UTF-8 bytes.
        // Only file paths/usernames enter the existing text-only adapters.
        query.signature = None;
        let query = query.into_text().map_err(|e| match e {
            sandproof_guest_runtime::file_query::Error::UnsupportedText(p) => {
                Error::UnsupportedText(p)
            }
            _ => unreachable!("text conversion only rejects unsupported text"),
        })?;
        if Instant::now() >= deadline {
            return Err(Error::DeadlineExceeded);
        }
        Ok(Authorization {
            auth: AuthPermit { guard },
            query,
            operation,
            deadline,
            request_owner: None,
        })
    }
}

fn signature(token: &[u8], query: &Query, operation: Operation) -> Zeroizing<[u8; 46]> {
    let mut hash = Sha256::new();
    hash.update(query.path.as_deref().unwrap_or_default());
    hash.update(b":");
    hash.update(operation.name());
    hash.update(b":");
    hash.update(query.username.as_deref().unwrap_or_default());
    hash.update(b":");
    hash.update(token);
    if let Some(expiration) = query.signature_expiration {
        hash.update(b":");
        hash.update(expiration.to_string().as_bytes());
    }
    let mut result = Zeroizing::new([0; 46]);
    result[..3].copy_from_slice(b"v1_");
    STANDARD_NO_PAD
        .encode_slice(hash.finalize(), &mut result[3..])
        .expect("SHA-256 has fixed base64 length");
    result
}
