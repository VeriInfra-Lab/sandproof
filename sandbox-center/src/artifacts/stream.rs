//! Bounded file chunks and a fixed total stream deadline; no full-file buffer.
use hyper::body::{Body, Bytes, Frame, SizeHint};
use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    fs::File,
    io::{AsyncRead, ReadBuf},
    time::{Instant, Sleep, sleep_until},
};

pub(super) struct ArtifactBody {
    file: Option<File>,
    remaining: u64,
    deadline: Pin<Box<Sleep>>,
    buffer: Vec<u8>,
}
impl ArtifactBody {
    pub(super) fn new(file: std::fs::File, bytes: u64, deadline: Instant) -> Self {
        Self {
            file: Some(File::from_std(file)),
            remaining: bytes,
            deadline: Box::pin(sleep_until(deadline)),
            buffer: vec![0; 64 * 1024],
        }
    }
    fn fail(&mut self, kind: io::ErrorKind) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        self.file.take();
        Poll::Ready(Some(Err(io::Error::new(
            kind,
            "protected artifact stream interrupted",
        ))))
    }
}
impl Body for ArtifactBody {
    type Data = Bytes;
    type Error = io::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        if self.file.is_none() {
            return Poll::Ready(None);
        }
        if self.deadline.as_mut().poll(cx).is_ready() {
            return self.fail(io::ErrorKind::TimedOut);
        }
        if self.remaining == 0 {
            self.file.take();
            return Poll::Ready(None);
        }
        let count = usize::try_from(self.remaining.min(self.buffer.len() as u64))
            .unwrap_or(self.buffer.len());
        let this = self.as_mut().get_mut();
        let mut buffer = ReadBuf::new(&mut this.buffer[..count]);
        let Some(file) = this.file.as_mut() else {
            return Poll::Ready(None);
        };
        match Pin::new(file).poll_read(cx, &mut buffer) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(_)) => self.fail(io::ErrorKind::Other),
            Poll::Ready(Ok(())) => {
                let filled = buffer.filled();
                if filled.is_empty() {
                    return self.fail(io::ErrorKind::UnexpectedEof);
                }
                let bytes = Bytes::copy_from_slice(filled);
                this.remaining -= bytes.len() as u64;
                Poll::Ready(Some(Ok(Frame::data(bytes))))
            }
        }
    }
    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.remaining)
    }
    fn is_end_stream(&self) -> bool {
        self.file.is_none() || self.remaining == 0
    }
}
