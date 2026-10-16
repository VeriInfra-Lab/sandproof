//! Admitted decoded HTTP body -> MIME decoder -> sequential files.
//! No request router or production defaults. The batch keeps its
//! original authorization/capacity/deadline through ignored parts and HTTP EOF.
use super::{BatchFile, BatchReply, FileStreamError, UploadBatch, timely};
use crate::upload_body::{GzipError, HttpBodyLimits, Input as HttpInput};
use crate::upload_response::GzipReadError;
use axum::body::{Body, Bytes};
use sandproof_guest_runtime::multipart::{self, Decoder, Event, QuotedPrintable, SourceStage};
use tokio::time::Instant;

/// Limits for owned HTTP frames, in addition to MIME/file/result budgets.
/// These bound accepted frame payloads; the HTTP implementation must separately
/// bound allocations made before handing a frame to this component.
#[derive(Clone, Copy, Debug)]
pub struct MultipartLimits {
    pub wire: multipart::Limits,
    pub frame_bytes: usize,
    pub frames: usize,
    pub trailer_bytes: usize,
    pub trailer_fields: usize,
}
impl MultipartLimits {
    fn http(self) -> HttpBodyLimits {
        HttpBodyLimits {
            total_bytes: self.wire.total_bytes,
            frame_bytes: self.frame_bytes,
            frames: self.frames,
            trailer_bytes: self.trailer_bytes,
            trailer_fields: self.trailer_fields,
        }
    }

    pub(crate) fn validate(self) -> Result<(), MultipartError> {
        self.http().validate()?;
        self.wire.validate_read_buffered()?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UploadBodyError {
    MultipartGzip {
        stage: SourceStage,
        error: GzipReadError,
    },
    QuotedPrintable(multipart::QuotedPrintableError),
    MultipartHeaders(multipart::HeaderError),
    MultipartRead(multipart::FramingError),
    MultipartSetup(multipart::BoundaryRejection),
    MissingFilename,
    Transfer(FileStreamError),
    Decode(multipart::Error),
    Transport,
    InputLimit,
    InvalidConfiguration,
    Gzip(GzipError),
    GzipPreflight(GzipError),
}
/// Compatibility name for the original multipart-only entry point.
pub type MultipartError = UploadBodyError;
impl From<GzipError> for UploadBodyError {
    fn from(error: GzipError) -> Self {
        match error {
            GzipError::Transport => Self::Transport,
            GzipError::InputLimit => Self::InputLimit,
            GzipError::InvalidConfiguration => Self::InvalidConfiguration,
            GzipError::DeadlineExceeded => Self::Transfer(FileStreamError::DeadlineExceeded),
            other => Self::Gzip(other),
        }
    }
}
impl From<FileStreamError> for MultipartError {
    fn from(error: FileStreamError) -> Self {
        Self::Transfer(error)
    }
}
impl From<multipart::Error> for MultipartError {
    fn from(error: multipart::Error) -> Self {
        Self::Decode(error)
    }
}
impl From<multipart::ReadError> for MultipartError {
    fn from(error: multipart::ReadError) -> Self {
        match error {
            multipart::ReadError::Decode(error) => Self::Decode(error),
            multipart::ReadError::Headers(error) => Self::MultipartHeaders(error),
            multipart::ReadError::Framing(error) => Self::MultipartRead(error),
            // A stage alone cannot stand in for the retained source cause.
            multipart::ReadError::Source(_) => Self::Decode(multipart::Error::Poisoned),
        }
    }
}
impl From<multipart::QuotedPrintableReadError> for MultipartError {
    fn from(error: multipart::QuotedPrintableReadError) -> Self {
        match error {
            multipart::QuotedPrintableReadError::Decode(error) => Self::Decode(error),
            multipart::QuotedPrintableReadError::Encoding(error) => Self::QuotedPrintable(error),
        }
    }
}
impl From<multipart::BoundaryError> for MultipartError {
    fn from(error: multipart::BoundaryError) -> Self {
        match error {
            multipart::BoundaryError::Rejected(error) => Self::MultipartSetup(error),
            multipart::BoundaryError::Decode(error) => Self::Decode(error),
        }
    }
}

impl UploadBatch {
    /// Consume a fresh batch and an identity-encoded HTTP multipart body.
    /// Original request headers/authentication/encoding selection belong to the
    /// caller. Content-Length, when supplied, is the whole HTTP body length.
    /// # Errors
    /// Invalid/truncated input, missing filename, budgets, transport failure,
    /// original absolute deadline or file I/O. Earlier writes never roll back.
    pub async fn receive_multipart(
        self,
        content_type: &str,
        body: Body,
        expected: Option<u64>,
        limits: MultipartLimits,
    ) -> Result<BatchReply, MultipartError> {
        self.receive_multipart_source(content_type, body, expected, limits, false)
            .await
    }

    pub(in crate::file_transfer) async fn receive_multipart_source(
        mut self,
        content_type: &str,
        body: Body,
        expected: Option<u64>,
        limits: MultipartLimits,
        gzip: bool,
    ) -> Result<BatchReply, MultipartError> {
        if !self.ready || !self.results.is_empty() || self.received != 0 {
            return Err(FileStreamError::InvalidArgument.into());
        }
        let mut input = Input::new(
            content_type,
            body,
            expected,
            limits,
            self.reservation.deadline,
            gzip,
        )?;
        let mut ended = false;
        while let Some(event) = input.next().await? {
            match event {
                Event::Part(part) if part.name.as_deref() == Some("file") => {
                    let filename = if self.query_path.is_some() {
                        ""
                    } else {
                        part.filename
                            .as_deref()
                            .ok_or(UploadBodyError::MissingFilename)?
                    };
                    let file = self.start_file(filename, None).await?;
                    receive_file(&mut input, file, part.quoted_printable).await?;
                }
                Event::Part(part) => drain_field(&mut input, part.quoted_printable).await?,
                Event::End => ended = true,
                _ => return Err(multipart::Error::Malformed.into()),
            }
        }
        if !ended {
            return Err(multipart::Error::Truncated.into());
        }
        // Input::next reaches None only after real HTTP EOF (including trailers
        // and bounded epilogue), not merely after the MIME closing delimiter.
        self.finish().map_err(Into::into)
    }
}

async fn receive_file(
    input: &mut Input,
    mut file: BatchFile<'_>,
    encoded: bool,
) -> Result<(), MultipartError> {
    let mut qp = if encoded {
        Some(QuotedPrintable::new(input.limits.wire.chunk_bytes)?)
    } else {
        None
    };
    loop {
        let event = match input.next().await {
            Ok(Some(event)) => event,
            result => {
                let error = result
                    .err()
                    .unwrap_or(MultipartError::Decode(multipart::Error::Truncated));
                if let Some(qp) = &mut qp {
                    let prefix = qp.abort_detailed(multipart::Error::Truncated);
                    write_bytes(&mut file, &prefix.bytes).await?;
                    // Only actual source failure or the MIME reader's body
                    // truncation can finish a QP line as a failed read. Local
                    // length/budget/deadline failures keep their own identity.
                    if (input.source_error.is_some()
                        && error == input.source_failure(SourceStage::Body)
                        || matches!(
                            error,
                            MultipartError::MultipartRead(multipart::FramingError::BodyTruncated)
                        ))
                        && let Some(multipart::QuotedPrintableReadError::Encoding(observed)) =
                            prefix.error
                    {
                        return Err(MultipartError::QuotedPrintable(observed));
                    }
                }
                return Err(error);
            }
        };
        match event {
            Event::Data(bytes) => {
                if let Some(reader) = &mut qp {
                    let end = input.decoder.data_read_end();
                    let output = reader.feed_reader(&bytes, end);
                    write_bytes(&mut file, &output.bytes).await?;
                    if let Some(error) = output.error {
                        return Err(
                            if error
                                == multipart::QuotedPrintableReadError::Decode(
                                    multipart::Error::Truncated,
                                )
                                && end
                                    == multipart::PartReadEnd::Failed(multipart::Error::Truncated)
                            {
                                input.body_failure()
                            } else {
                                error.into()
                            },
                        );
                    }
                    if end == multipart::PartReadEnd::Eof {
                        qp = None;
                    }
                } else {
                    write_bytes(&mut file, &bytes).await?;
                }
            }
            Event::PartEnd => {
                if let Some(mut qp) = qp {
                    let output = qp.finish_detailed();
                    write_bytes(&mut file, &output.bytes).await?;
                    if let Some(error) = output.error {
                        return Err(error.into());
                    }
                }
                return file.finish().await.map_err(Into::into);
            }
            _ => return Err(multipart::Error::Malformed.into()),
        }
    }
}

async fn write_bytes(file: &mut BatchFile<'_>, bytes: &[u8]) -> Result<(), MultipartError> {
    for chunk in bytes.chunks(file.chunk_bytes()) {
        file.write(chunk).await?;
    }
    Ok(())
}

async fn drain_field(input: &mut Input, encoded: bool) -> Result<(), MultipartError> {
    let mut discard = multipart::Discard::new(encoded, input.limits.wire.chunk_bytes)?;
    match input.next_with(Some(&mut discard)).await? {
        Some(Event::PartEnd) => Ok(()),
        _ => Err(multipart::Error::Malformed.into()),
    }
}

struct Input {
    body: HttpInput,
    decoder: Decoder,
    pending: Bytes,
    limits: MultipartLimits,
    deadline: Instant,
    eof: bool,
    source_error: Option<GzipError>,
    gzip: bool,
}
impl Input {
    fn new(
        content_type: &str,
        body: Body,
        expected: Option<u64>,
        limits: MultipartLimits,
        deadline: Instant,
        gzip: bool,
    ) -> Result<Self, MultipartError> {
        let body = HttpInput::new(body, expected, limits.http(), deadline)?;
        timely(deadline, Ok(()))?;
        let boundary = multipart::boundary_detailed(content_type, limits.wire.header_bytes)?;
        let decoder = Decoder::new_read_buffered(&boundary, limits.wire)?;
        Ok(Self {
            body,
            decoder,
            pending: Bytes::new(),
            limits,
            deadline,
            eof: false,
            source_error: None,
            gzip,
        })
    }

    async fn next(&mut self) -> Result<Option<Event>, MultipartError> {
        self.next_with(None).await
    }

    async fn next_with(
        &mut self,
        mut discard: Option<&mut multipart::Discard>,
    ) -> Result<Option<Event>, MultipartError> {
        loop {
            timely(self.deadline, Ok(()))?;
            let event = if let Some(discard) = &mut discard {
                discard
                    .advance(&mut self.decoder)
                    .map(|status| match status {
                        multipart::DiscardStatus::NeedInput => None,
                        // Internal completion of Close only; never used to finish
                        // a file or publish successful raw-part completion.
                        multipart::DiscardStatus::Complete => Some(Event::PartEnd),
                    })
            } else {
                self.decoder.next_event_detailed()
            };
            match event {
                Err(multipart::ReadError::Source(stage)) => return Err(self.source_failure(stage)),
                Ok(Some(event)) => return Ok(Some(event)),
                Err(error) => return Err(error.into()),
                Ok(None) => {}
            }
            if self.eof {
                return Ok(None);
            }
            if !self.pending.is_empty() {
                let count = self.pending.len().min(self.decoder.source_read_capacity()?);
                let bytes = self.pending.split_to(count);
                let chunks = bytes.chunks(self.limits.wire.chunk_bytes);
                let length = chunks.len();
                for (index, chunk) in chunks.enumerate() {
                    self.decoder.feed_read(chunk, index + 1 == length)?;
                }
                continue;
            }
            let next = match self.body.next().await {
                Err(error)
                    if self.body.read_failed()
                        && matches!(
                            error,
                            GzipError::Transport
                                | GzipError::Truncated
                                | GzipError::Header
                                | GzipError::Checksum
                                | GzipError::Deflate
                        ) =>
                {
                    self.source_error = Some(error);
                    // Drain bytes already read before assigning the failure to
                    // headers, file contents or NextPart. Never turn it into EOF.
                    self.decoder.fail_source()?;
                    continue;
                }
                result => result?,
            };
            let Some(bytes) = next else {
                self.eof = true;
                self.decoder.end_input();
                continue;
            };
            self.pending = bytes;
        }
    }

    fn source_failure(&self, stage: SourceStage) -> MultipartError {
        let Some(error) = self.source_error else {
            return MultipartError::Decode(multipart::Error::Poisoned);
        };
        if self.gzip
            && stage != SourceStage::Complete
            && let Ok(error) = GzipReadError::try_from(error)
        {
            return MultipartError::MultipartGzip { stage, error };
        }
        error.into()
    }

    fn body_failure(&self) -> MultipartError {
        if self.source_error.is_some() {
            self.source_failure(SourceStage::Body)
        } else {
            MultipartError::MultipartRead(multipart::FramingError::BodyTruncated)
        }
    }
}
