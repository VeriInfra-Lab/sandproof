use super::GzipError;
use axum::body::{Body, Bytes};
use http_body_util::BodyExt;
use tokio::time::{Instant, timeout_at};

/// Limits on accepted HTTP frames; allocations made by the HTTP library before
/// this boundary must be constrained separately by the eventual listener.
#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpBodyLimits {
    pub total_bytes: u64,
    pub frame_bytes: usize,
    pub frames: usize,
    pub trailer_bytes: usize,
    pub trailer_fields: usize,
}
impl HttpBodyLimits {
    pub(crate) fn validate(self) -> Result<Self, GzipError> {
        if [
            self.frame_bytes,
            self.frames,
            self.trailer_bytes,
            self.trailer_fields,
        ]
        .into_iter()
        .any(|n| n == 0 || n > isize::MAX as usize / 16)
        {
            return Err(GzipError::InvalidConfiguration);
        }
        Ok(self)
    }
}

pub(crate) struct Input {
    body: Body,
    limits: HttpBodyLimits,
    expected: Option<u64>,
    received: u64,
    frames_left: usize,
    trailers: bool,
    ended: bool,
    read_failed: bool,
    deadline: Instant,
}

impl Input {
    pub(crate) fn new(
        body: Body,
        expected: Option<u64>,
        limits: HttpBodyLimits,
        deadline: Instant,
    ) -> Result<Self, GzipError> {
        let limits = limits.validate()?;
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
            read_failed: false,
            deadline,
        })
    }

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn read_failed(&self) -> bool {
        self.read_failed
    }

    pub(crate) async fn next(&mut self) -> Result<Option<Bytes>, GzipError> {
        self.read_failed = false;
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
            let frame = match frame {
                Ok(frame) => frame,
                Err(error) => {
                    self.read_failed = true;
                    return Err(body_error(error));
                }
            };
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

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;
    use std::time::Duration;

    #[tokio::test]
    async fn source_failure_marker_excludes_local_length_frame_and_deadline_failures() {
        let limits = HttpBodyLimits {
            total_bytes: 64,
            frame_bytes: 16,
            frames: 8,
            trailer_bytes: 64,
            trailer_fields: 8,
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        for error in [
            GzipError::Transport,
            GzipError::Checksum,
            GzipError::InputLimit,
        ] {
            let body = Body::from_stream(stream::iter([Err::<Bytes, _>(error)]));
            let mut input = Input::new(body, None, limits, deadline).unwrap();
            assert_eq!(input.next().await, Err(error));
            assert!(input.read_failed());
        }
        for (body, expected, error) in [
            (Body::empty(), Some(1), GzipError::Transport),
            (Body::from(vec![0; 17]), None, GzipError::InputLimit),
            (Body::from(vec![0; 2]), Some(1), GzipError::Transport),
        ] {
            let mut input = Input::new(body, expected, limits, deadline).unwrap();
            assert_eq!(input.next().await, Err(error));
            assert!(!input.read_failed());
        }
        let mut input = Input::new(Body::empty(), None, limits, deadline).unwrap();
        input.deadline = Instant::now();
        assert_eq!(input.next().await, Err(GzipError::DeadlineExceeded));
        assert!(!input.read_failed());
    }
}
