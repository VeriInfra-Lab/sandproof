//! One admitted upload: encoding/gzip header -> account -> metadata -> mode.
use super::{
    BatchLimits, BatchReply, FileStreamError, FileStreams, GzipLimits, MultipartLimits,
    Reservation, UploadBodyError, timely,
};
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
    }

    /// Send successes and actual file I/O faults under the original upload slot
    /// and deadline. Other preflight/transport/component errors remain explicit
    /// until their complete public contracts are connected; never fake a body.
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
            Err(
                UploadRequestError::Transfer(FileStreamError::UploadFault(fault))
                | UploadRequestError::Body(UploadBodyError::Transfer(FileStreamError::UploadFault(
                    fault,
                ))),
            ) => crate::upload_response::io_failure(
                fault,
                expected,
                response,
                reservation.deadline,
                reservation.lease,
            )
            .await
            .map_err(UploadHttpError::Response),
            Err(error) => Err(UploadHttpError::Request(error)),
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
    ) -> Result<BatchReply, UploadRequestError> {
        let deadline = reservation.deadline;
        let auth = Arc::new(auth);
        let owner = Arc::new((Arc::clone(&auth), reservation.clone()));
        let encoding = file_encoding::content(&request.headers, limits.encoding)
            .map_err(UploadRequestError::Encoding)?;
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
                .map_err(UploadRequestError::GzipPreflight)?,
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
        let metadata =
            file_upload::metadata(&request.headers).map_err(UploadRequestError::Headers)?;
        let content_type = request
            .headers
            .get("content-type")
            .map_or(&[][..], |value| value.as_bytes());
        let kind =
            wire::kind(content_type, limits.headers.bytes).map_err(UploadRequestError::Headers)?;
        if kind == Kind::Unsupported {
            return Err(UploadRequestError::UnsupportedContentType);
        }
        if kind == Kind::Raw && request.path.is_none() {
            return Err(UploadRequestError::RawPathRequired);
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
        .map_err(UploadRequestError::Body)
    }
}
enum Input {
    Identity(Body, Option<u64>),
    Gzip(GzipBody),
}
