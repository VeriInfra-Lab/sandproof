//! Explicit Basic lookup precedes Connect decoding. An opaque handoff preserves
//! its account, settings, authorization, file slot and original deadline.
use super::{
    AccountPool, AccountSource, Arc, ContextError, FilePool, FileReply, FileSettings, HeaderMap,
    Instant, Owner, Ownership, Request, RpcError, RpcFailure, check_deadline, filesystem,
};
use crate::{
    basic_auth,
    file_rpc_requests::{Authenticated, Decoded, Failure},
    file_rpc_response::Rejection,
    files::{RpcContextError, SelectionError, file_account},
};
use axum::body::Body;
use sandproof_guest_runtime::{accounts::Account, filesystem::FileLimits};

pub struct PreparedRpc {
    authenticated: Authenticated,
    selected: Selected,
}
struct Selected {
    account: Option<Account>,
    settings: FileSettings,
    limits: FileLimits,
    lease: Arc<super::super::Lease>,
}
pub struct ReadyRpc {
    decoded: Decoded,
    selected: Selected,
}

impl FilePool {
    /// Reserve file capacity and resolve only an explicit Basic account before
    /// Connect timeout/method/body decoding. Malformed/absent Basic defers the
    /// default account lookup until execution. No body is polled here.
    /// # Errors
    /// Admission, settings, username or account failure. Known missing users
    /// retain their fixed rejection. The failure holds both acquired slots, while
    /// any still-running account worker also retains the original authorization.
    pub async fn prepare_rpc<S: AccountSource>(
        &self,
        mut authenticated: Authenticated,
        accounts: &AccountPool<S>,
        settings: FileSettings,
    ) -> Result<PreparedRpc, RpcFailure> {
        let mut deadline = authenticated.deadline();
        let mut owner = authenticated.owner();
        let mut rejection = None;
        let result = async {
            deadline = deadline.min(self.deadline().map_err(RpcError::Dispatch)?);
            check_deadline(deadline).map_err(RpcError::Dispatch)?;
            let lease = self.reserve(owner.clone()).map_err(RpcError::Dispatch)?;
            owner = lease.clone();
            authenticated.retain(lease.clone(), deadline);
            settings.validate().map_err(RpcError::Context)?;
            let name = basic_auth::explicit_user(
                authenticated.headers(),
                settings.limits.username_bytes,
                settings.limits.authorization_bytes,
            )
            .map_err(|error| {
                RpcError::Context(match error {
                    basic_auth::Error::Unauthenticated => ContextError::Unauthenticated,
                    basic_auth::Error::ResourceExhausted => ContextError::ResourceExhausted,
                })
            })?;
            let authenticated = Arc::new(authenticated);
            let account = if let Some(name) = name {
                Some(
                    file_account(
                        &name,
                        accounts,
                        &settings,
                        Some((deadline, authenticated.clone())),
                    )
                    .await
                    .map_err(|error| match error {
                        SelectionError::UnknownUser(user) => {
                            rejection = Some(Rejection::ExplicitUser(user));
                            RpcError::Context(ContextError::Unauthenticated)
                        }
                        SelectionError::Context(error) => RpcError::Context(error),
                    })?,
                )
            } else {
                None
            };
            check_deadline(deadline).map_err(RpcError::Dispatch)?;
            // Successful join has dropped the account worker's owner. A late
            // or cancelled worker instead keeps its own clone until real exit.
            let authenticated = Arc::try_unwrap(authenticated)
                .map_err(|_| RpcError::Context(ContextError::Unavailable))?;
            Ok(PreparedRpc {
                authenticated,
                selected: Selected {
                    account,
                    settings,
                    limits: self.limits,
                    lease,
                },
            })
        }
        .await;
        result.map_err(|error| RpcFailure {
            error,
            rejection,
            fault: None,
            deadline,
            _owner: owner,
        })
    }
}

impl PreparedRpc {
    pub(crate) fn owner(&self) -> Owner {
        self.selected.lease.clone()
    }

    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.authenticated.deadline()
    }
    #[must_use]
    pub const fn headers(&self) -> &HeaderMap {
        self.authenticated.headers()
    }

    /// Consume the original preflight and decode under both retained slots.
    /// The trusted caller must first select the supported HTTP protocol/codec.
    /// # Errors
    /// Same bounded framing/JSON/timeout failures as the shared decoder. Queued
    /// decoding, returned failures and cancellation keep their original owner.
    pub async fn decode(self, method: &str, body: Body) -> Result<ReadyRpc, Failure> {
        Ok(ReadyRpc {
            decoded: self.authenticated.decode(method, body).await?,
            selected: self.selected,
        })
    }
}

impl ReadyRpc {
    #[must_use]
    pub const fn request(&self) -> &Request {
        self.decoded.request()
    }
    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.decoded.deadline()
    }

    /// Reuse the explicit account or resolve the default now, then dispatch
    /// under the same file slot, settings and capped deadline. No reacquisition
    /// or account refresh is permitted for an already selected explicit user.
    /// # Errors
    /// Context, I/O, expiry or worker failure; timeout never implies rollback.
    pub async fn execute<S: AccountSource>(
        self,
        accounts: &AccountPool<S>,
    ) -> Result<FileReply, RpcFailure> {
        let request = self.decoded.into_rpc();
        let deadline = request.deadline;
        let Selected {
            account,
            settings,
            limits,
            lease,
        } = self.selected;
        let owner: Owner = lease.clone();
        let mut rejection = None;
        let mut fault = None;
        let result = async {
            check_deadline(deadline).map_err(RpcError::Dispatch)?;
            let auth = Arc::new(request.auth);
            let context = if let Some(account) = account {
                auth.resolve_file_rpc_account(&request.request, account, &settings)
                    .map_err(RpcContextError::from)
            } else {
                auth.resolve_default_file_rpc_admitted(
                    &request.request,
                    accounts,
                    &settings,
                    deadline,
                    Arc::new((auth.clone(), lease.clone())),
                )
                .await
            }
            .map_err(|error| match error {
                RpcContextError::UnknownUser(user) => {
                    rejection = Some(Rejection::DefaultUser(user));
                    RpcError::Context(ContextError::Unauthenticated)
                }
                RpcContextError::Expansion(error) => {
                    rejection = Some(Rejection::PathExpansion(error));
                    RpcError::Context(ContextError::InvalidArgument)
                }
                RpcContextError::Context(error) => RpcError::Context(error),
            })?;
            FilePool::dispatch_observed(auth, lease, deadline, move || {
                filesystem::execute_detailed(
                    &context.request,
                    Ownership {
                        uid: context.owner.uid,
                        gid: context.owner.gid,
                    },
                    limits,
                )
            })
            .await
            .map_err(|failure| {
                fault = failure.fault;
                RpcError::Dispatch(failure.error)
            })
        }
        .await;
        result.map_err(|error| RpcFailure {
            error,
            rejection,
            fault,
            deadline,
            _owner: owner,
        })
    }
}
