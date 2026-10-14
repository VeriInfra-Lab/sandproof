//! Explicit Basic account lookup precedes the caller's protocol selection.
use super::{Authenticated, Body, Decoded, Error, Failure, Instant, Request, check_deadline};
use crate::{AccountPool, AccountSource, ContextError, ContextLimits, basic_auth};
use sandproof_guest_runtime::accounts::Account;
use std::sync::Arc;

pub struct Prepared {
    authenticated: Authenticated,
    // Preserve the exact explicit account for Start context composition.
    // PID operations don't switch credentials or refresh this lookup.
    account: Option<Account>,
}
pub struct Ready {
    pub(super) decoded: Decoded,
    pub(super) account: Option<Account>,
}
impl Authenticated {
    /// Resolve only an explicit Basic username before decoding. Absent or
    /// malformed Basic defers the initialized default account to Start. This
    /// phase does not interpret Connect headers or poll the body.
    /// # Errors
    /// Invalid limits, missing/wrong account, saturation or original expiry.
    /// A blocked account worker retains authorization and request capacity until
    /// it actually exits, including after this waiter is dropped or times out.
    pub async fn prepare<S: AccountSource>(
        self,
        accounts: &AccountPool<S>,
        limits: ContextLimits,
    ) -> Result<Prepared, Failure> {
        let deadline = self.deadline();
        let owner = self.admitted.owner.clone();
        let result = async {
            check_deadline(deadline)?;
            if limits.username_bytes == 0 || limits.authorization_bytes == 0 {
                return Err(Error::Context(ContextError::InvalidArgument));
            }
            let name = basic_auth::explicit_user(
                self.headers(),
                limits.username_bytes,
                limits.authorization_bytes,
            )
            .map_err(|error| {
                Error::Context(match error {
                    basic_auth::Error::Unauthenticated => ContextError::Unauthenticated,
                    basic_auth::Error::ResourceExhausted => ContextError::ResourceExhausted,
                })
            })?;
            let authenticated = Arc::new(self);
            let account = if let Some(name) = name {
                if name.is_empty() || name.contains(['\0', '\r', '\n', ':']) {
                    return Err(Error::Context(ContextError::Unauthenticated));
                }
                let account = accounts
                    .lookup_admitted(name.clone(), deadline, authenticated.clone())
                    .await
                    .map_err(Error::Context)?;
                if account.name != name {
                    return Err(Error::Context(ContextError::Unavailable));
                }
                Some(account)
            } else {
                None
            };
            check_deadline(deadline)?;
            let authenticated = Arc::try_unwrap(authenticated).map_err(|_| Error::Unavailable)?;
            Ok(Prepared {
                authenticated,
                account,
            })
        }
        .await;
        result.map_err(|error| Failure::new(error, deadline, owner))
    }
}
impl Prepared {
    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.authenticated.deadline()
    }
    #[must_use]
    pub const fn headers(&self) -> &axum::http::HeaderMap {
        self.authenticated.headers()
    }

    /// Decode exactly one selected, uncompressed JSON request after the caller's
    /// HTTP negotiation. Start/Connect require their original Connect envelope;
    /// unary methods take plain JSON. The caller supplies its already evaluated
    /// protocol deadline; it can only shorten the original admission deadline.
    /// # Errors
    /// Bounded transport/framing/JSON errors or expiry. No command effects.
    pub async fn decode(
        self,
        method: &'static str,
        body: Body,
        protocol_deadline: Instant,
    ) -> Result<Ready, Failure> {
        Ok(Ready {
            decoded: self
                .authenticated
                .decode(method, body, protocol_deadline)
                .await?,
            account: self.account,
        })
    }
}
impl Ready {
    #[must_use]
    pub const fn request(&self) -> &Request {
        &self.decoded.request
    }
    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.decoded.deadline
    }
}
