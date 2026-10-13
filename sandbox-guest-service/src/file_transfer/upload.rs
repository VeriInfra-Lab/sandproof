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
    fn legacy(self) -> UploadRequestError {
        match self {
            Self::Request(error) => error,
            Self::Rejected(error) => match error {
                Rejection::UnsupportedEncoding(_) => {
                    UploadRequestError::Encoding(file_encoding::Error::Unsupported)
                }
                Rejection::UnsupportedContentType(_) => UploadRequestError::UnsupportedContentType,
                Rejection::RawPathRequired => UploadRequestError::RawPathRequired,
                Rejection::Metadata(_) => UploadRequestError::Headers(wire::Error::InvalidMetadata),
                Rejection::GzipHeader(error) => UploadRequestError::GzipPreflight(error.into()),
                Rejection::MultipartHeaders(_) => UploadRequestError::Body(
                    UploadBodyError::Decode(sandproof_guest_runtime::multipart::Error::Malformed),
                ),
                Rejection::MultipartRead(error) => {
                    UploadRequestError::Body(UploadBodyError::Decode(error.coarse()))
                }
                Rejection::RawGzipBody(error) => {
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
        let reservation = self.upload_admission(&request.headers, limits)?;
        self.upload_request_reserved(auth, request, accounts, settings, limits, reservation)
            .await
            .map_err(Failure::legacy)
    }

    /// Send successes, observed file I/O faults and supported header/dispatch/part
    /// rejections under the original upload slot and deadline. Unmapped account,
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
        let reservation = self
            .upload_admission(&request.headers, limits)
            .map_err(UploadHttpError::Request)?;
        let expected = request.expected;
        let result = self
            .upload_request_reserved(
                auth,
                request,
                accounts,
                settings,
                limits,
                reservation.clone(),
            )
            .await;
        match result {
            Ok(reply) => reply
                .into_http_response(response)
                .await
                .map_err(UploadHttpError::Response),
            Err(Failure::Request(
                UploadRequestError::Transfer(FileStreamError::UploadFault(fault))
                | UploadRequestError::Body(UploadBodyError::Transfer(FileStreamError::UploadFault(
                    fault,
                ))),
            )) => crate::upload_response::io_failure(
                fault,
                expected,
                response,
                reservation.deadline,
                reservation.lease,
            )
            .await
            .map_err(UploadHttpError::Response),
            Err(Failure::Rejected(error)) => crate::upload_response::rejected(
                error,
                response,
                reservation.deadline,
                reservation.lease,
            )
            .await
            .map_err(UploadHttpError::Response),
            Err(Failure::Request(error)) => Err(UploadHttpError::Request(error)),
        }
    }

    fn upload_admission(
        &self,
        headers: &HeaderMap,
        limits: UploadLimits,
    ) -> Result<Reservation, UploadRequestError> {
        file_upload::check_headers(headers, limits.headers).map_err(UploadRequestError::Headers)?;
        let (lease, deadline) = self.admit().map_err(UploadRequestError::Transfer)?;
        Ok(Reservation { lease, deadline })
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
            .map_err(UploadRequestError::Context)?;
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
                let (body, expected) = match input {
                    Input::Identity(body, expected) => (body, expected),
                    Input::Gzip(input) => (input.into_body(), None),
                };
                batch
                    .receive_multipart(content_type, body, expected, limits.multipart)
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
