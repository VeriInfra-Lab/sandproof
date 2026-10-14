//! One admitted upload: encoding/gzip header -> account -> metadata -> mode.
use super::{
    BatchLimits, BatchReply, FileStreamError, FileStreams, GzipLimits, MultipartLimits,
    Reservation, UploadBodyError, timely,
};
use crate::upload_response::{GzipHeaderError, GzipReadError, Rejection};
use crate::{
    AccountPool, AccountSource, AuthPermit, ContextError, file_encoding, file_upload,
    files::FileSettings, upload_body::GzipBody,
};
use axum::{body::Body, http::HeaderMap};
use sandproof_guest_runtime::file_upload::{self as wire, Kind};
use std::sync::Arc;

/// Query values are already decoded by the caller; absent differs from empty.
/// Expected length comes from the HTTP framing layer, not a re-parsed header.
pub struct UploadRequest {
    pub username: Option<String>,
    pub path: Option<String>,
    pub headers: HeaderMap,
    pub body: Body,
    pub expected: Option<u64>,
}
/// The capability owns the signed path/user; the body cannot replace them.
pub struct AuthorizedUpload {
    pub authorization: crate::file_authorization::Authorization,
    pub headers: HeaderMap,
    pub body: Body,
    pub expected: Option<u64>,
}
#[derive(Clone, Copy)]
pub struct UploadLimits {
    pub headers: file_upload::HeaderLimits,
    pub encoding: file_encoding::Limits,
    pub gzip: GzipLimits,
    pub multipart: MultipartLimits,
    pub batch: BatchLimits,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UploadRequestError {
    Authorization(crate::file_authorization::Error),
    Headers(wire::Error),
    Encoding(file_encoding::Error),
    GzipPreflight(crate::upload_body::GzipError),
    Context(ContextError),
    Transfer(FileStreamError),
    Body(UploadBodyError),
    UnsupportedContentType,
    UnsupportedText,
    RawPathRequired,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UploadHttpError {
    Request(UploadRequestError),
    Response(crate::upload_response::Error),
}

enum Failure {
    Request(UploadRequestError),
    Rejected(Rejection),
}
impl From<UploadRequestError> for Failure {
    fn from(error: UploadRequestError) -> Self {
        Self::Request(error)
    }
}
impl Failure {
    fn selection(error: crate::files::SelectionError) -> Self {
        match error {
            crate::files::SelectionError::UnknownUser(error) => {
                Self::Rejected(Rejection::UnknownUser(error))
            }
            crate::files::SelectionError::Context(error) => {
                Self::Request(UploadRequestError::Context(error))
            }
        }
    }
    fn legacy(self) -> UploadRequestError {
        match self {
            Self::Request(error) => error,
            Self::Rejected(error) => match error {
                Rejection::UnknownUser(_) => {
                    UploadRequestError::Context(ContextError::Unauthenticated)
                }
                Rejection::PathExpansion(_) => UploadRequestError::Body(UploadBodyError::Transfer(
                    FileStreamError::Context(ContextError::InvalidArgument),
                )),
                Rejection::UnsupportedEncoding(_) => {
                    UploadRequestError::Encoding(file_encoding::Error::Unsupported)
                }
                Rejection::UnsupportedContentType(_) => UploadRequestError::UnsupportedContentType,
                Rejection::RawPathRequired => UploadRequestError::RawPathRequired,
                Rejection::Metadata(_) => UploadRequestError::Headers(wire::Error::InvalidMetadata),
                Rejection::GzipHeader(error) => UploadRequestError::GzipPreflight(error.into()),
                Rejection::QuotedPrintable(_) | Rejection::MultipartHeaders(_) => {
                    UploadRequestError::Body(UploadBodyError::Decode(
                        sandproof_guest_runtime::multipart::Error::Malformed,
                    ))
                }
                Rejection::MultipartRead(error) => {
                    UploadRequestError::Body(UploadBodyError::Decode(error.coarse()))
                }
                Rejection::RawGzipBody(error) | Rejection::MultipartGzip { error, .. } => {
                    UploadRequestError::Body(UploadBodyError::Gzip(error.into()))
                }
                Rejection::MissingFilename => {
                    UploadRequestError::Body(UploadBodyError::MissingFilename)
                }
                Rejection::MultipartSetup(error) => {
                    UploadRequestError::Body(UploadBodyError::MultipartSetup(error))
                }
                Rejection::DuplicatePath(error) => UploadRequestError::Body(
                    UploadBodyError::Transfer(FileStreamError::DuplicatePath(error)),
                ),
            },
        }
    }

    fn gzip_preflight(error: crate::upload_body::GzipError) -> Self {
        match GzipHeaderError::try_from(error) {
            Ok(error) => Self::Rejected(Rejection::GzipHeader(error)),
            Err(error) => Self::Request(UploadRequestError::GzipPreflight(error)),
        }
    }

    fn body(error: UploadBodyError) -> Self {
        match error {
            UploadBodyError::Transfer(FileStreamError::PathExpansion(error)) => {
                Self::Rejected(Rejection::PathExpansion(error))
            }
            UploadBodyError::MultipartGzip { stage, error } => {
                Self::Rejected(Rejection::MultipartGzip { stage, error })
            }
            UploadBodyError::QuotedPrintable(error) => {
                Self::Rejected(Rejection::QuotedPrintable(error))
            }
            UploadBodyError::MultipartHeaders(error) => {
                Self::Rejected(Rejection::MultipartHeaders(error))
            }
            UploadBodyError::MultipartRead(error) => {
                Self::Rejected(Rejection::MultipartRead(error))
            }
            UploadBodyError::MultipartSetup(error) => {
                Self::Rejected(Rejection::MultipartSetup(error))
            }
            UploadBodyError::MissingFilename => Self::Rejected(Rejection::MissingFilename),
            UploadBodyError::Transfer(FileStreamError::DuplicatePath(error)) => {
                Self::Rejected(Rejection::DuplicatePath(error))
            }
            other => Self::Request(UploadRequestError::Body(other)),
        }
    }

    fn raw_gzip_body(error: UploadBodyError) -> Self {
        if let UploadBodyError::Gzip(error) = error {
            match GzipReadError::try_from(error) {
                Ok(error) => Self::Rejected(Rejection::RawGzipBody(error)),
                Err(error) => Self::body(UploadBodyError::Gzip(error)),
            }
        } else {
            Self::body(error)
        }
    }
}
impl FileStreams {
    /// Caller has authenticated this request. Does not install a route or
    /// implement signing/query decoding, complete HTTP errors or serialization.
    /// # Errors
    /// Ordered preflight/account/header failures, original deadline or partial
    /// body/file failure. Completed files and failed prefixes are never replayed.
    pub async fn upload_request<S: AccountSource>(
        &self,
        auth: AuthPermit,
        request: UploadRequest,
        accounts: &AccountPool<S>,
        settings: &FileSettings,
        limits: UploadLimits,
    ) -> Result<BatchReply, UploadRequestError> {
        let reservation = self.upload_admission(&request.headers, limits, None)?;
        self.upload_request_reserved(auth, request, accounts, settings, limits, reservation)
            .await
            .map_err(Failure::legacy)
    }

    /// Send successes, observed file I/O faults and supported header/dispatch/part
    /// rejections and unknown accounts under the original upload slot and deadline. Other account,
    /// codec/transport and component failures remain explicit.
    /// # Errors
    /// Unmapped request rejection, unknown I/O observation, response budget,
    /// expired original deadline or response delivery failure.
    pub async fn upload_request_response<S: AccountSource>(
        &self,
        auth: AuthPermit,
        request: UploadRequest,
        accounts: &AccountPool<S>,
        settings: &FileSettings,
        limits: UploadLimits,
        response: crate::upload_response::Limits,
    ) -> Result<axum::response::Response, UploadHttpError> {
        self.upload_response_before((auth, request), accounts, settings, limits, response, None)
            .await
    }

    /// Consume only a write capability and preserve its query and original
    /// deadline through account lookup, file effects and response bytes.
    /// # Errors
    /// Wrong operation/expiry or the existing admitted upload/response errors.
    pub async fn upload_authorized<S: AccountSource>(
        &self,
        request: AuthorizedUpload,
        accounts: &AccountPool<S>,
        settings: &FileSettings,
        limits: UploadLimits,
        response: crate::upload_response::Limits,
    ) -> Result<axum::response::Response, UploadHttpError> {
        let (auth, query, origin) = request
            .authorization
            .take(crate::file_authorization::Operation::Write)
            .map_err(|error| UploadHttpError::Request(UploadRequestError::Authorization(error)))?;
        self.upload_response_before(
            (
                auth,
                UploadRequest {
                    username: query.username,
                    path: query.path,
                    headers: request.headers,
                    body: request.body,
                    expected: request.expected,
                },
            ),
            accounts,
            settings,
            limits,
            response,
            Some(origin),
        )
        .await
    }

    async fn upload_response_before<S: AccountSource>(
        &self,
        start: (AuthPermit, UploadRequest),
        accounts: &AccountPool<S>,
        settings: &FileSettings,
        limits: UploadLimits,
        response: crate::upload_response::Limits,
        original: Option<crate::file_authorization::Origin>,
    ) -> Result<axum::response::Response, UploadHttpError> {
        let (auth, request) = start;
        let reservation = self
            .upload_admission(&request.headers, limits, original)
            .map_err(UploadHttpError::Request)?;
        let expected = request.expected;
        let (settlement, mut settled) = tokio::sync::watch::channel(());
        let mut work = reservation.clone();
        work.settlement = Some(settlement);
        let result = self
            .upload_request_reserved(auth, request, accounts, settings, limits, work)
            .await;
        match result {
            Ok(mut reply) => {
                // The completed result is not a live worker. Keep its original
                // lease for response bytes, but do not wait on our own receipt.
                reply.reservation.settlement.take();
                settle(reservation.deadline, &mut settled).await?;
                reply
                    .into_http_response(response)
                    .await
                    .map_err(UploadHttpError::Response)
            }
            Err(Failure::Request(
                UploadRequestError::Transfer(FileStreamError::UploadFault(fault))
                | UploadRequestError::Body(UploadBodyError::Transfer(FileStreamError::UploadFault(
                    fault,
                ))),
            )) => {
                settle(reservation.deadline, &mut settled).await?;
                crate::upload_response::io_failure(
                    fault,
                    expected,
                    response,
                    reservation.deadline,
                    reservation.lease,
                )
                .await
                .map_err(UploadHttpError::Response)
            }
            Err(Failure::Rejected(error)) => {
                settle(reservation.deadline, &mut settled).await?;
                crate::upload_response::rejected(
                    error,
                    response,
                    reservation.deadline,
                    reservation.lease,
                )
                .await
                .map_err(UploadHttpError::Response)
            }
            Err(Failure::Request(error)) => Err(UploadHttpError::Request(error)),
        }
    }

    fn upload_admission(
        &self,
        headers: &HeaderMap,
        limits: UploadLimits,
        original: Option<crate::file_authorization::Origin>,
    ) -> Result<Reservation, UploadRequestError> {
        file_upload::check_headers(headers, limits.headers).map_err(UploadRequestError::Headers)?;
        let (lease, deadline) = self
            .admit_before(original)
            .map_err(UploadRequestError::Transfer)?;
        Ok(Reservation {
            lease,
            deadline,
            settlement: None,
        })
    }

    async fn upload_request_reserved<S: AccountSource>(
        &self,
        auth: AuthPermit,
        request: UploadRequest,
        accounts: &AccountPool<S>,
        settings: &FileSettings,
        limits: UploadLimits,
        reservation: Reservation,
    ) -> Result<BatchReply, Failure> {
        let deadline = reservation.deadline;
        let auth = Arc::new(auth);
        let owner = Arc::new((Arc::clone(&auth), reservation.clone()));
        let encoding =
            file_encoding::content(&request.headers, limits.encoding).map_err(|error| {
                if error == file_encoding::Error::Unsupported {
                    Failure::Rejected(Rejection::UnsupportedEncoding(
                        request
                            .headers
                            .get("content-encoding")
                            .map_or(&[][..], |v| v.as_bytes())
                            .to_vec(),
                    ))
                } else {
                    Failure::Request(UploadRequestError::Encoding(error))
                }
            })?;
        let input = match encoding {
            file_encoding::Encoding::Identity => Input::Identity(request.body, request.expected),
            file_encoding::Encoding::Gzip => Input::Gzip(
                GzipBody::open(
                    request.body,
                    request.expected,
                    limits.gzip,
                    deadline,
                    owner.clone(),
                )
                .await
                .map_err(Failure::gzip_preflight)?,
            ),
        };
        let selection = auth
            .resolve_http_files_admitted(
                request.username.as_deref(),
                accounts,
                settings,
                deadline,
                owner,
            )
            .await
            .map_err(Failure::selection)?;
        timely(deadline, Ok(())).map_err(UploadRequestError::Transfer)?;
        let metadata = file_upload::metadata_detailed(&request.headers)
            .map_err(|error| Failure::Rejected(Rejection::Metadata(error)))?;
        let content_type = request
            .headers
            .get("content-type")
            .map_or(&[][..], |value| value.as_bytes());
        let kind =
            wire::kind(content_type, limits.headers.bytes).map_err(UploadRequestError::Headers)?;
        if kind == Kind::Unsupported {
            return Err(Failure::Rejected(Rejection::UnsupportedContentType(
                content_type.to_vec(),
            )));
        }
        if kind == Kind::Raw && request.path.is_none() {
            return Err(Failure::Rejected(Rejection::RawPathRequired));
        }
        let batch = self
            .upload_batch_reserved(
                auth,
                selection,
                request.path.as_deref(),
                metadata,
                limits.batch,
                reservation,
            )
            .map_err(UploadRequestError::Transfer)?;
        match (kind, input) {
            (Kind::Raw, Input::Identity(body, expected)) => {
                batch.receive_raw(body, expected, limits.gzip.http).await
            }
            (Kind::Raw, Input::Gzip(input)) => batch.receive_gzip_input(input).await,
            (Kind::Multipart, input) => {
                let content_type = std::str::from_utf8(content_type)
                    .map_err(|_| UploadRequestError::UnsupportedText)?;
                let (body, expected, gzip) = match input {
                    Input::Identity(body, expected) => (body, expected, false),
                    Input::Gzip(input) => (input.into_body(), None, true),
                };
                batch
                    .receive_multipart_source(content_type, body, expected, limits.multipart, gzip)
                    .await
            }
            (Kind::Unsupported, _) => unreachable!(),
        }
        .map_err(|error| {
            if kind == Kind::Raw && encoding == file_encoding::Encoding::Gzip {
                Failure::raw_gzip_body(error)
            } else {
                Failure::body(error)
            }
        })
    }
}
enum Input {
    Identity(Body, Option<u64>),
    Gzip(GzipBody),
}

async fn settle(
    deadline: tokio::time::Instant,
    settled: &mut tokio::sync::watch::Receiver<()>,
) -> Result<(), UploadHttpError> {
    let result = tokio::time::timeout_at(deadline, settled.changed())
        .await
        .map_err(|_| UploadHttpError::Response(crate::upload_response::Error::DeadlineExceeded))?;
    if result.is_ok() {
        return Err(UploadHttpError::Response(
            crate::upload_response::Error::Unavailable,
        ));
    }
    super::timely(deadline, Ok(()))
        .map_err(|_| UploadHttpError::Response(crate::upload_response::Error::DeadlineExceeded))
}
