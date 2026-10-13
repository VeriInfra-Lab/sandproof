//! Admitted identity-encoded HTTP body -> MIME decoder -> sequential files.
//! No request router, gzip decoder or production defaults. The batch keeps its
//! original authorization/capacity/deadline through ignored parts and HTTP EOF.
use super::{BatchFile, BatchReply, FileStreamError, UploadBatch, timely};
use axum::body::{Body, Bytes};
use http_body_util::BodyExt;
use sandproof_guest_runtime::multipart::{self, Decoder, Event, QuotedPrintable};
use tokio::time::{Instant, timeout_at};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MultipartError {
    Transfer(FileStreamError),
    Decode(multipart::Error),
    Transport,
    InputLimit,
    InvalidConfiguration,
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

impl UploadBatch {
    /// Consume a fresh batch and an identity-encoded HTTP multipart body.
    /// Original request headers/authentication/encoding selection belong to the
    /// caller. Content-Length, when supplied, is the whole HTTP body length.
    /// # Errors
    /// Invalid/truncated input, missing filename, budgets, transport failure,
    /// original absolute deadline or file I/O. Earlier writes never roll back.
    pub async fn receive_multipart(
        mut self,
        content_type: &str,
        body: Body,
        expected: Option<u64>,
        limits: MultipartLimits,
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
                            .ok_or(FileStreamError::InvalidArgument)?
                    };
                    let file = self.start_file(filename, None).await?;
                    receive_file(&mut input, file, part.quoted_printable).await?;
                }
                Event::Part(_) => drain_field(&mut input).await?,
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
                    let prefix = qp.abort(multipart::Error::Truncated);
                    write_bytes(&mut file, &prefix.bytes).await?;
                }
                return Err(error);
            }
        };
        match event {
            Event::Data(bytes) => {
                if let Some(qp) = &mut qp {
                    let output = qp.feed(&bytes);
                    write_bytes(&mut file, &output.bytes).await?;
                    if let Some(error) = output.error {
                        return Err(error.into());
                    }
                } else {
                    write_bytes(&mut file, &bytes).await?;
                }
            }
            Event::PartEnd => {
                if let Some(mut qp) = qp {
                    let output = qp.finish();
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

async fn drain_field(input: &mut Input) -> Result<(), MultipartError> {
    // Only file fields have decoding/file effects. Unknown fields are discarded
    // as raw bytes, still covered by all wire/part budgets and the same deadline.
    loop {
        match input.next().await? {
            Some(Event::Data(_)) => {}
            Some(Event::PartEnd) => return Ok(()),
            _ => return Err(multipart::Error::Malformed.into()),
        }
    }
}

struct Input {
    body: Body,
    decoder: Decoder,
    pending: Bytes,
    limits: MultipartLimits,
    deadline: Instant,
    expected: Option<u64>,
    received: u64,
    frames_left: usize,
    trailers: bool,
    eof: bool,
}
impl Input {
    fn new(
        content_type: &str,
        body: Body,
        expected: Option<u64>,
        limits: MultipartLimits,
        deadline: Instant,
    ) -> Result<Self, MultipartError> {
        if [
            limits.frame_bytes,
            limits.frames,
            limits.trailer_bytes,
            limits.trailer_fields,
        ]
        .into_iter()
        .any(|n| n == 0 || n > isize::MAX as usize / 16)
        {
            return Err(MultipartError::InvalidConfiguration);
        }
        if expected.is_some_and(|n| n > limits.wire.total_bytes) {
            return Err(MultipartError::InputLimit);
        }
        timely(deadline, Ok(()))?;
        let boundary = multipart::boundary(content_type, limits.wire.header_bytes)?;
        let decoder = Decoder::new(&boundary, limits.wire)?;
        Ok(Self {
            body,
            decoder,
            pending: Bytes::new(),
            limits,
            deadline,
            expected,
            received: 0,
            frames_left: limits.frames,
            trailers: false,
            eof: false,
        })
    }

    async fn next(&mut self) -> Result<Option<Event>, MultipartError> {
        loop {
            timely(self.deadline, Ok(()))?;
            if let Some(event) = self.decoder.next_event()? {
                return Ok(Some(event));
            }
            if self.eof {
                return Ok(None);
            }
            if !self.pending.is_empty() {
                let count = self.pending.len().min(self.limits.wire.chunk_bytes);
                let chunk = self.pending.split_to(count);
                self.decoder.feed(&chunk)?;
                continue;
            }
            let result = timeout_at(self.deadline, self.body.frame())
                .await
                .map_err(|_| FileStreamError::DeadlineExceeded)?;
            timely(self.deadline, Ok(()))?;
            let Some(frame) = result else {
                if self.expected.is_some_and(|n| n != self.received) {
                    return Err(MultipartError::Transport);
                }
                self.eof = true;
                self.decoder.end_input();
                continue;
            };
            self.frames_left = self
                .frames_left
                .checked_sub(1)
                .ok_or(MultipartError::InputLimit)?;
            let frame = frame.map_err(|_| MultipartError::Transport)?;
            match frame.into_data() {
                Ok(bytes) => {
                    if self.trailers {
                        return Err(MultipartError::Transport);
                    }
                    if bytes.len() > self.limits.frame_bytes
                        || bytes.len() as u64 > self.limits.wire.total_bytes - self.received
                    {
                        return Err(MultipartError::InputLimit);
                    }
                    self.received += bytes.len() as u64;
                    if self.expected.is_some_and(|n| self.received > n) {
                        return Err(MultipartError::Transport);
                    }
                    self.pending = bytes;
                }
                Err(frame) => {
                    let headers = frame
                        .into_trailers()
                        .map_err(|_| MultipartError::Transport)?;
                    if self.trailers {
                        return Err(MultipartError::Transport);
                    }
                    self.trailers = true;
                    let length = headers.iter().try_fold(0_usize, |n, (k, v)| {
                        n.checked_add(k.as_str().len())?.checked_add(v.len())
                    });
                    if headers.len() > self.limits.trailer_fields
                        || length.is_none_or(|n| n > self.limits.trailer_bytes)
                    {
                        return Err(MultipartError::InputLimit);
                    }
                }
            }
        }
    }
}
