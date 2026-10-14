//! Shared GET/HEAD/POST /files dispatch over the original bounded components.
//! No listener, socket parsing, production defaults or catch-all HTTP policy.
use crate::{
    AccountPool, AccountSource, MetadataSource, Session,
    file_authorization::Operation,
    file_download::{ContentType, IdentityRequest},
    file_requests::{AdmissionError, FileRequests, ResponseError},
    file_transfer::{
        AuthorizedUpload, DownloadLimits, DownloadRequestError, FileStreams, UploadHttpError,
        UploadLimits,
    },
    files::FileSettings,
    owned_body::Owner,
    upload_response,
};
use axum::{
    body::{Body, HttpBody},
    http::{HeaderMap, Request},
    response::Response,
};
use sandproof_guest_runtime::{file_download::date::LocalZone, file_mime::Catalog};
use std::{fmt, sync::Arc};
use tokio::time::Instant;

#[cfg(test)]
mod tests;

/// Trusted daemon configuration/snapshots, not fields from the HTTP request.
/// Existing account/transfer/response components validate their respective
/// settings. No daemon environment, MIME files or local zone are guessed here.
pub struct Settings<Z> {
    pub files: FileSettings,
    pub upload: UploadLimits,
    pub download: DownloadLimits,
    pub upload_response: upload_response::Limits,
    pub mime: Arc<Catalog>,
    pub local: Arc<Z>,
}

pub struct FileHttp<M, S, Z> {
    inner: Arc<Inner<M, S, Z>>,
}
struct Inner<M, S, Z> {
    session: Session<M>,
    requests: FileRequests,
    streams: FileStreams,
    accounts: AccountPool<S>,
    settings: Settings<Z>,
}
impl<M, S, Z> Clone for FileHttp<M, S, Z> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<M: MetadataSource, S: AccountSource, Z: LocalZone + Send + Sync + 'static> FileHttp<M, S, Z> {
    pub(crate) fn shares_session(&self, session: &Session<M>) -> bool {
        self.inner.session.shares_state(session)
    }
    #[must_use]
    pub fn new(
        session: Session<M>,
        requests: FileRequests,
        streams: FileStreams,
        accounts: AccountPool<S>,
        settings: Settings<Z>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                session,
                requests,
                streams,
                accounts,
                settings,
            }),
        }
    }

    /// Accept an unconsumed body from a trusted HTTP framing layer. The exact
    /// body size hint is the framing length; Content-Length is not parsed again.
    /// Query/path/user, authorization and budgets follow the shared entry; a
    /// download cannot select its own MIME path, multipart boundary or response
    /// headers. Only an actually completed upload becomes a success response.
    /// # Errors
    /// Unsupported route, failed admission, unmapped preparation/transfer error
    /// or failed response delivery. Errors preserve the original request slot
    /// until dropped; callers must use a separately bounded listener policy.
    pub async fn handle(&self, request: Request<Body>) -> Result<Response, Failure> {
        let inner = &self.inner;
        let admitted = inner.requests.admit().map_err(|error| Failure {
            error: Error::Admission(error),
            deadline: None,
            _owner: None,
        })?;
        let deadline = admitted.deadline();
        let owner = admitted.owner();
        if request.uri().path() != "/files" {
            return Err(Failure::retained(Error::UnsupportedRoute, deadline, owner));
        }
        let authorization = match admitted
            .authorize(
                &inner.session,
                request.method(),
                request.headers(),
                request.uri().query().unwrap_or("").as_bytes(),
            )
            .await
        {
            Ok(authorization) => authorization,
            Err(rejected) => {
                return rejected.into_response().await.map_err(|error| {
                    Failure::retained(Error::Preparation(error), deadline, owner)
                });
            }
        };
        let (parts, body) = request.into_parts();
        let settings = &inner.settings;
        match authorization.operation() {
            Operation::Write => {
                let expected = body.size_hint().exact();
                inner
                    .streams
                    .upload_authorized(
                        AuthorizedUpload {
                            authorization,
                            headers: parts.headers,
                            body,
                            expected,
                        },
                        &inner.accounts,
                        &settings.files,
                        settings.upload,
                        settings.upload_response,
                    )
                    .await
                    .map_err(|error| Failure::retained(Error::Upload(error), deadline, owner))
            }
            Operation::Read => {
                // Go closes the unused request body; never drain it before
                // replying or let client bytes drive another file operation.
                drop(body);
                inner
                    .streams
                    .download_authorized(
                        authorization,
                        IdentityRequest {
                            method: parts.method,
                            headers: parts.headers,
                            response: HeaderMap::new(),
                            content_type: ContentType::Detect {
                                original_path: String::new(),
                                catalog: Arc::clone(&settings.mime),
                            },
                            disposition_path: None,
                            // Generated lazily in the original blocking range worker.
                            boundary: None,
                        },
                        &inner.accounts,
                        &settings.files,
                        settings.download,
                        Zone(Arc::clone(&settings.local)),
                    )
                    .await
                    .map(crate::file_download::Download::into_response)
                    .map_err(|error| Failure::retained(Error::Download(error), deadline, owner))
            }
        }
    }
}

struct Zone<Z>(Arc<Z>);
impl<Z: LocalZone> LocalZone for Zone<Z> {
    fn offset(&self, name: &[u8], wall_as_utc: i64) -> Option<i32> {
        self.0.offset(name, wall_as_utc)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Admission(AdmissionError),
    UnsupportedRoute,
    Preparation(ResponseError),
    Upload(UploadHttpError),
    Download(DownloadRequestError),
}

/// Classified failure plus the original request responsibility. A cancelled
/// account/file task may separately retain the same slot until it actually ends.
/// This is not a public wire error and has no `IntoResponse` fallback.
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
        f.debug_struct("FileHttpFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}
