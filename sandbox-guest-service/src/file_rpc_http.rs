//! Shared unary filesystem HTTP dispatch; no listener or catch-all error policy.
use crate::{
    AccountPool, AccountSource, MetadataSource, Session,
    file_io::{FilePool, ReadyRpc, RpcError, RpcFailure},
    file_rpc_requests::{self, FileRpcRequests, SelectionError},
    file_rpc_response,
    files::FileSettings,
    owned_body::Owner,
};
use axum::{
    body::Body,
    http::{HeaderValue, Request},
    response::Response,
};
use sandproof_guest_runtime::file_protocol::{self, JsonContentType};
use std::{fmt, sync::Arc};
use tokio::time::Instant;

#[cfg(test)]
mod tests;

pub struct FileRpcHttp<M, S> {
    inner: Arc<Inner<M, S>>,
}
struct Inner<M, S> {
    session: Session<M>,
    requests: FileRpcRequests,
    files: FilePool,
    accounts: AccountPool<S>,
    settings: FileSettings,
    response: file_rpc_response::Limits,
}
/// A response already determined before file effects, or one opaque ready
/// operation. Consumers may inspect the request but cannot replace its fields,
/// authorization, settings, accounts, capacity or absolute deadline.
pub enum Prepared<M, S> {
    Response(Response),
    Ready(Box<Ready<M, S>>),
}
pub struct Ready<M, S> {
    inner: Arc<Inner<M, S>>,
    operation: ReadyRpc,
    codec: JsonContentType,
    encoding: file_rpc_response::Encoding,
}
impl<M, S> Clone for FileRpcHttp<M, S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}
impl<M: MetadataSource, S: AccountSource> FileRpcHttp<M, S> {
    #[must_use]
    pub fn new(
        session: Session<M>,
        requests: FileRpcRequests,
        files: FilePool,
        accounts: AccountPool<S>,
        settings: FileSettings,
        response: file_rpc_response::Limits,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                session,
                requests,
                files,
                accounts,
                settings,
                response,
            }),
        }
    }

    /// Admit an unconsumed HTTP request before token/account work, select the
    /// fixed JSON codec, decode once, execute once and return an owned response.
    /// Settings and all pools are trusted daemon inputs, with no defaults here.
    /// # Errors
    /// Admission, unsupported route/format, unmapped context/body/I/O
    /// error or failed response preparation. Acquired slots and original expiry
    /// remain owned by the failure; the listener needs a bounded failure policy.
    pub async fn handle(&self, request: Request<Body>) -> Result<Response, Failure> {
        match self.prepare(request).await? {
            Prepared::Response(response) => Ok(response),
            Prepared::Ready(ready) => ready.execute().await,
        }
    }

    /// Same admission, preflight and protocol/body processing as handle, before
    /// any file I/O. A caller can apply additional trusted restrictions to the
    /// immutable decoded request (for example an isolated test fixture).
    /// # Errors
    /// Same request/preparation failures and original ownership as handle.
    pub async fn prepare(&self, request: Request<Body>) -> Result<Prepared<M, S>, Failure> {
        let inner = &self.inner;
        let admitted = inner.requests.admit().map_err(|error| Failure {
            error: Error::Admission(error),
            deadline: None,
            _owner: None,
        })?;
        let deadline = admitted.deadline();
        let owner = admitted.owner();
        let method = match request.uri().path() {
            "/filesystem.Filesystem/Stat" => "Stat",
            "/filesystem.Filesystem/MakeDir" => "MakeDir",
            "/filesystem.Filesystem/ListDir" => "ListDir",
            "/filesystem.Filesystem/Move" => "Move",
            "/filesystem.Filesystem/Remove" => "Remove",
            _ => return Err(Failure::retained(Error::Route, deadline, owner)),
        };
        let (parts, body) = request.into_parts();
        let authenticated = match admitted.authenticate(&inner.session, parts.headers).await {
            Ok(authenticated) => authenticated,
            Err(failure) => {
                if failure.has_response() {
                    return failure
                        .into_authorization_response(
                            inner.response,
                            parts.method == axum::http::Method::HEAD,
                        )
                        .await
                        .map(Prepared::Response)
                        .map_err(Failure::response);
                }
                return Err(Failure::request(failure));
            }
        };
        // Determine response format without returning a protocol rejection yet:
        // the original explicit Basic middleware must still run first.
        let codec = authenticated.json_codec();
        let selection = file_rpc_requests::select_http(&parts.method, codec);
        let encoding = file_rpc_requests::response_encoding(authenticated.headers());
        let prepared = match inner
            .files
            .prepare_rpc(authenticated, &inner.accounts, inner.settings.clone())
            .await
        {
            Ok(prepared) => prepared,
            Err(failure) => {
                if codec.is_some() && failure.rejection().is_some() {
                    return failure
                        .into_response(inner.response)
                        .await
                        .map(Prepared::Response)
                        .map_err(Failure::response);
                }
                return Err(Failure::execution(failure));
            }
        };
        let codec = match selection {
            Ok(codec) => codec,
            Err(SelectionError::Method) => {
                let deadline = prepared.deadline();
                let owner = prepared.owner();
                // Preflight has finished: response retention keeps capacity,
                // but must not keep its authorization read lock or account.
                drop(prepared);
                return file_rpc_response::method_not_allowed(
                    parts.method == axum::http::Method::HEAD,
                    inner.response,
                    deadline,
                    owner,
                )
                .await
                .map(Prepared::Response)
                .map_err(Failure::response);
            }
            Err(error) => {
                return Err(Failure::retained(
                    Error::Selection(error),
                    prepared.deadline(),
                    prepared.owner(),
                ));
            }
        };
        let ready = match prepared.decode(method, body).await {
            Ok(ready) => ready,
            Err(failure) => {
                if failure.has_response() {
                    return failure
                        .into_response(inner.response)
                        .await
                        .map(Prepared::Response)
                        .map_err(Failure::response);
                }
                return Err(Failure::request(failure));
            }
        };
        Ok(Prepared::Ready(Box::new(Ready {
            inner: self.inner.clone(),
            operation: ready,
            codec,
            encoding,
        })))
    }
}
impl<M: MetadataSource, S: AccountSource> Ready<M, S> {
    #[must_use]
    pub const fn request(&self) -> &file_protocol::Request {
        self.operation.request()
    }
    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.operation.deadline()
    }

    /// Execute exactly once under the original pools/settings/authorization,
    /// then encode actual records or the known default-account rejection.
    /// # Errors
    /// Unmapped file/context failure or response preparation failure, retaining
    /// original capacity/deadline. Completed or partial I/O is never replayed.
    pub async fn execute(self) -> Result<Response, Failure> {
        let Self {
            inner,
            operation,
            codec,
            encoding,
        } = self;
        match operation.execute(&inner.accounts).await {
            Ok(reply) => {
                let mut response = reply
                    .into_response_with_encoding(inner.response, encoding)
                    .await
                    .map_err(Failure::response)?;
                response
                    .headers_mut()
                    .insert("content-type", HeaderValue::from_static(codec.canonical()));
                Ok(response)
            }
            Err(failure) => {
                if failure.rejection().is_some() || failure.fault().is_some() {
                    failure
                        .into_response(inner.response)
                        .await
                        .map_err(Failure::response)
                } else {
                    Err(Failure::execution(failure))
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Admission(file_rpc_requests::AdmissionError),
    Route,
    Selection(SelectionError),
    Request(file_rpc_requests::Error),
    Execution(RpcError),
    Response(file_rpc_response::Error),
}
pub struct Failure {
    error: Error,
    deadline: Option<Instant>,
    _owner: Option<Owner>,
}
impl Failure {
    fn retained(error: Error, deadline: Instant, owner: Owner) -> Self {
        Self {
            error,
            deadline: Some(deadline),
            _owner: Some(owner),
        }
    }
    fn request(failure: file_rpc_requests::Failure) -> Self {
        Self::retained(
            Error::Request(failure.error().clone()),
            failure.deadline(),
            Arc::new(failure),
        )
    }
    fn execution(failure: RpcFailure) -> Self {
        Self::retained(
            Error::Execution(failure.error()),
            failure.deadline(),
            Arc::new(failure),
        )
    }
    fn response(failure: file_rpc_response::Failure) -> Self {
        Self::retained(
            Error::Response(failure.error()),
            failure.deadline(),
            Arc::new(failure),
        )
    }
    #[must_use]
    pub const fn error(&self) -> &Error {
        &self.error
    }
    #[must_use]
    pub const fn deadline(&self) -> Option<Instant> {
        self.deadline
    }
}
impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileRpcHttpFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}
