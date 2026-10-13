use super::GzipError;
use axum::body::{Body, Bytes};
use http_body_util::BodyExt;
use tokio::time::{Instant, timeout_at};

/// Limits on accepted HTTP frames; allocations made by the HTTP library before
/// this boundary must be constrained separately by the eventual listener.
#[derive(Clone, Copy, Debug)]
pub struct HttpBodyLimits {
    pub total_bytes: u64,
    pub frame_bytes: usize,
    pub frames: usize,
    pub trailer_bytes: usize,
    pub trailer_fields: usize,
}

pub(crate) struct Input {
    body: Body,
    limits: HttpBodyLimits,
    expected: Option<u64>,
    received: u64,
    frames_left: usize,
    trailers: bool,
    ended: bool,
    deadline: Instant,
}

impl Input {
    pub(crate) fn new(
        body: Body,
        expected: Option<u64>,
        limits: HttpBodyLimits,
        deadline: Instant,
    ) -> Result<Self, GzipError> {
        if [
            limits.frame_bytes,
            limits.frames,
            limits.trailer_bytes,
            limits.trailer_fields,
        ]
        .into_iter()
        .any(|n| n == 0 || n > isize::MAX as usize / 16)
        {
            return Err(GzipError::InvalidConfiguration);
        }
        if expected.is_some_and(|n| n > limits.total_bytes) {
            return Err(GzipError::InputLimit);
        }
        check_deadline(deadline)?;
        Ok(Self {
            body,
            limits,
            expected,
            received: 0,
            frames_left: limits.frames,
            trailers: false,
            ended: false,
            deadline,
        })
    }

    pub(crate) async fn next(&mut self) -> Result<Option<Bytes>, GzipError> {
        loop {
            check_deadline(self.deadline)?;
            if self.ended {
                return Ok(None);
            }
            let frame = timeout_at(self.deadline, self.body.frame())
                .await
                .map_err(|_| GzipError::DeadlineExceeded)?;
            check_deadline(self.deadline)?;
            let Some(frame) = frame else {
                if self.expected.is_some_and(|n| n != self.received) {
                    return Err(GzipError::Transport);
                }
                self.ended = true;
                return Ok(None);
            };
            self.frames_left = self
                .frames_left
                .checked_sub(1)
                .ok_or(GzipError::InputLimit)?;
            let frame = frame.map_err(body_error)?;
            match frame.into_data() {
                Ok(bytes) => {
                    if self.trailers {
                        return Err(GzipError::Transport);
                    }
                    if bytes.len() > self.limits.frame_bytes
                        || bytes.len() as u64 > self.limits.total_bytes - self.received
                    {
                        return Err(GzipError::InputLimit);
                    }
                    self.received += bytes.len() as u64;
                    if self.expected.is_some_and(|n| self.received > n) {
                        return Err(GzipError::Transport);
                    }
                    return Ok(Some(bytes));
                }
                Err(frame) => {
                    let headers = frame.into_trailers().map_err(|_| GzipError::Transport)?;
                    if self.trailers {
                        return Err(GzipError::Transport);
                    }
                    self.trailers = true;
                    let bytes = headers.iter().try_fold(0_usize, |n, (key, value)| {
                        n.checked_add(key.as_str().len())?.checked_add(value.len())
                    });
                    if headers.len() > self.limits.trailer_fields
                        || bytes.is_none_or(|n| n > self.limits.trailer_bytes)
                    {
                        return Err(GzipError::InputLimit);
                    }
                }
            }
        }
    }
}

pub(super) fn body_error(error: axum::Error) -> GzipError {
    // axum 0.8.9 Body::from_stream boxes an axum::Error twice. Unwrap only
    // known owned wrappers, with a fixed depth; arbitrary errors stay opaque.
    let mut error = error.into_inner();
    for _ in 0..4 {
        error = match error.downcast::<GzipError>() {
            Ok(error) => return *error,
            Err(error) => error,
        };
        match error.downcast::<axum::Error>() {
            Ok(inner) => error = inner.into_inner(),
            Err(_) => break,
        }
    }
    GzipError::Transport
}

pub(super) fn check_deadline(deadline: Instant) -> Result<(), GzipError> {
    if Instant::now() >= deadline {
        Err(GzipError::DeadlineExceeded)
    } else {
        Ok(())
    }
}
