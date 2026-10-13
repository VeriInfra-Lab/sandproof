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
    /// Stat, selection (including its Vary stage), open, plan or original
    /// deadline. Body errors cannot be retried or turned into successful EOF.
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
                let (modified, open) = inspect()?;
                control.check()?;
                let encoding = file_encoding::download(&request.headers, limits.encoding)
                    .map_err(DownloadError::Negotiation)?;
                request
                    .response
                    .insert("vary", HeaderValue::from_static("Accept-Encoding"));
                control.check()?;
                let source = open()?;
                control.check()?;
                set_disposition(&mut request, identity)?;
                super::super::header_budget(
                    &request.response,
                    identity.plan.response_header_bytes,
                )?;
                if encoding == file_encoding::Encoding::Identity {
                    let head = request.method == Method::HEAD;
                    let headers = request.response.clone();
                    let cancel = control.cancel.clone();
                    let result =
                        prepare_identity(source, modified, request, identity, &local, control);
                    return match result {
                        Ok((metadata, reader)) => Ok((metadata, ReaderKind::Identity(reader))),
                        Err(DownloadError::SniffSeek | DownloadError::LengthSeek) => {
                            let plan = super::super::serve_error(
                                headers,
                                StatusCode::INTERNAL_SERVER_ERROR,
                                b"seeker can't seek\n",
                                identity.plan,
                            )?;
                            super::response_reader(
                                std::io::Cursor::new(Vec::new()),
                                plan,
                                head,
                                Control { deadline, cancel },
                                identity.chunk_bytes,
                            )
                            .map(|(metadata, reader)| (metadata, ReaderKind::Error(reader)))
                        }
                        Err(error) => Err(error),
                    };
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
