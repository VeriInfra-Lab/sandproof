//! Stat/directory -> encoding -> original open -> identity or gzip, one worker.
use super::{
    Arc, ContentType, Control, Download, DownloadError, HeaderValue, IdentityLimits,
    IdentityRequest, Instant, LocalZone, Method, OwnedBody, Owner, Read, Reader, Seek, Source,
    StatusCode, Timestamp, file_mime, prepare_identity, set_disposition,
};
use crate::{
    file_encoding,
    upload_body::{self, GzipError},
};
use sandproof_guest_runtime::gzip::{EncodeLimits, Encoder};

#[derive(Clone, Copy)]
pub struct NegotiationLimits {
    pub identity: IdentityLimits,
    pub encoding: file_encoding::Limits,
    pub compressed_bytes: u64,
}

impl Download {
    /// The admitted inspection must stat and reject directories, returning the
    /// original mtime and a deferred open of the same resolved path. It must
    /// not open or read before selection. All closures and source destruction
    /// run on this one worker; there is no second admission or deadline.
    /// # Errors
    /// Unmapped I/O, configuration/budgets or original deadline. Known Linux
    /// filesystem/selection/initial seek failures become bounded HTTP replies.
    /// Body errors cannot be retried or turned into successful EOF.
    pub async fn open_negotiated<S, F, R, Z>(
        inspect: S,
        mut request: IdentityRequest,
        limits: NegotiationLimits,
        local: Z,
        deadline: Instant,
        owner: Owner,
    ) -> Result<Self, DownloadError>
    where
        S: FnOnce() -> Result<(Timestamp, F), DownloadError> + Send + 'static,
        F: FnOnce() -> Result<R, DownloadError> + 'static,
        R: Read + Seek + 'static,
        Z: LocalZone + Send + 'static,
    {
        let identity = limits.identity;
        if identity.chunk_bytes == 0
            || identity.chunk_bytes > isize::MAX as usize / 16
            || identity.path_bytes == 0
            || identity.path_bytes > isize::MAX as usize / 16
        {
            return Err(DownloadError::InvalidConfiguration);
        }
        let ((status, headers), body) =
            OwnedBody::start(deadline, Arc::clone(&owner), move |_, cancel| {
                let control = Control { deadline, cancel };
                control.check()?;
                if request
                    .disposition_path
                    .as_ref()
                    .is_some_and(|path| path.len() > identity.path_bytes)
                {
                    return Err(DownloadError::ResourceExhausted);
                }
                super::super::header_budget(
                    &request.response,
                    identity.plan.response_header_bytes,
                )?;
                let (modified, open) = match inspect() {
                    Ok(result) => result,
                    Err(error) => return failure(error, &request, identity, control),
                };
                control.check()?;
                let encoding = match file_encoding::download(&request.headers, limits.encoding) {
                    Ok(encoding) => encoding,
                    Err(error) => {
                        return failure(
                            DownloadError::Negotiation(error),
                            &request,
                            identity,
                            control,
                        );
                    }
                };
                request
                    .response
                    .insert("vary", HeaderValue::from_static("Accept-Encoding"));
                control.check()?;
                let source = match open() {
                    Ok(source) => source,
                    Err(error) => return failure(error, &request, identity, control),
                };
                control.check()?;
                set_disposition(&mut request, identity)?;
                super::super::header_budget(
                    &request.response,
                    identity.plan.response_header_bytes,
                )?;
                if encoding == file_encoding::Encoding::Identity {
                    return identity_reader(source, modified, request, identity, &local, control);
                }

                let kind = gzip_type(request.content_type, identity.path_bytes)?;
                request.response.insert("content-type", kind);
                request
                    .response
                    .insert("content-encoding", HeaderValue::from_static("gzip"));
                request.response.remove("content-length");
                super::super::header_budget(
                    &request.response,
                    identity.plan.response_header_bytes,
                )?;
                let mut encoder = upload_body::encode::encoder(
                    source,
                    EncodeLimits {
                        chunk_bytes: identity.chunk_bytes,
                        decoded_bytes: identity.file_bytes,
                        encoded_bytes: limits.compressed_bytes.min(identity.plan.wire_bytes),
                    },
                    deadline,
                    control.cancel.clone(),
                )
                .map_err(DownloadError::Gzip)?;
                let reader = if request.method == Method::HEAD {
                    // Go still copies/compresses the source for HEAD; the transport
                    // suppresses bytes. Complete that work even if HTTP never polls
                    // a HEAD response body. No compressed bytes are retained.
                    while encoder.next_chunk().map_err(gzip_error)?.is_some() {
                        control.check()?;
                    }
                    drop(encoder);
                    ReaderKind::Empty
                } else {
                    ReaderKind::Gzip(Box::new(encoder))
                };
                control.check()?;
                Ok(((StatusCode::OK, request.response), reader))
            })
            .await?;
        Ok(Self {
            status,
            headers,
            body,
            _owner: owner,
        })
    }
}
fn identity_reader<R: Read + Seek>(
    source: R,
    modified: Timestamp,
    mut request: IdentityRequest,
    limits: IdentityLimits,
    local: &impl LocalZone,
    control: Control,
) -> Result<((StatusCode, super::HeaderMap), ReaderKind<R>), DownloadError> {
    let head = request.method == Method::HEAD;
    let headers = request.response.clone();
    let error_path = request.disposition_path.take();
    let deadline = control.deadline;
    let cancel = control.cancel.clone();
    let plan = match prepare_identity(source, modified, request, limits, local, control) {
        Ok((metadata, reader)) => return Ok((metadata, ReaderKind::Identity(reader))),
        Err(DownloadError::SniffSeek | DownloadError::LengthSeek) => super::super::serve_error(
            headers,
            StatusCode::INTERNAL_SERVER_ERROR,
            b"seeker can't seek\n".as_slice().into(),
            limits.plan,
        )?,
        Err(DownloadError::RangeSeekIo(errno)) => {
            super::errors::range(errno, error_path.as_deref(), headers, limits)?
        }
        Err(error) => return Err(error),
    };
    error_reader(plan, head, Control { deadline, cancel }, limits.chunk_bytes)
}
fn failure<R: Read + Seek>(
    error: DownloadError,
    request: &IdentityRequest,
    limits: IdentityLimits,
    control: Control,
) -> Result<((StatusCode, super::HeaderMap), ReaderKind<R>), DownloadError> {
    control.check()?;
    let plan = super::errors::json(error, request, limits)?;
    error_reader(
        plan,
        request.method == Method::HEAD,
        control,
        limits.chunk_bytes,
    )
}
fn error_reader<R: Read + Seek>(
    plan: super::ResponsePlan,
    head: bool,
    control: Control,
    chunk_bytes: usize,
) -> Result<((StatusCode, super::HeaderMap), ReaderKind<R>), DownloadError> {
    super::response_reader(
        std::io::Cursor::new(Vec::new()),
        plan,
        head,
        control,
        chunk_bytes,
    )
    .map(|(metadata, reader)| (metadata, ReaderKind::Error(reader)))
}
fn gzip_type(content: ContentType, path_bytes: usize) -> Result<HeaderValue, DownloadError> {
    match content {
        ContentType::Known(kind) => Ok(kind),
        ContentType::Detect {
            original_path,
            catalog,
        } => {
            if original_path.len() > path_bytes {
                return Err(DownloadError::ResourceExhausted);
            }
            let kind = catalog
                .lookup(file_mime::extension(&original_path))
                .map_err(DownloadError::Mime)?
                .filter(|value| !value.is_empty())
                .unwrap_or("application/octet-stream");
            HeaderValue::from_str(kind).map_err(|_| DownloadError::InvalidConfiguration)
        }
    }
}
fn gzip_error(error: sandproof_guest_runtime::gzip::Error) -> DownloadError {
    DownloadError::Gzip(GzipError::from(error))
}
enum ReaderKind<R: Read + Seek> {
    Identity(Reader<R>),
    Gzip(Box<Encoder<upload_body::encode::Reader<R>>>),
    Error(Reader<std::io::Cursor<Vec<u8>>>),
    Empty,
}
impl<R: Read + Seek> Source<DownloadError> for ReaderKind<R> {
    fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, DownloadError> {
        match self {
            Self::Identity(reader) => reader.next_chunk(),
            Self::Gzip(encoder) => encoder.next_chunk().map_err(gzip_error),
            Self::Error(reader) => reader.next_chunk(),
            Self::Empty => Ok(None),
        }
    }
}
