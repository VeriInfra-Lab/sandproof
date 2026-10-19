//! Preserve a finite rejection while the peer finishes its outstanding upload.
//! No discarded bytes are decoded, forwarded or dispatched. The original
//! connection task and absolute deadline also own this bounded closing phase.
use super::Limits;
use bytes::Bytes;
use hyper::body::{Body, Frame, Incoming, SizeHint};
use std::{
    future::Future,
    io::{self, IoSlice},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, ready},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    time::{Instant, Sleep},
};

pub(super) struct ObservedBody {
    inner: Incoming,
    abandoned: Arc<AtomicBool>,
    ended: bool,
}

impl ObservedBody {
    pub(super) fn new(inner: Incoming, abandoned: Arc<AtomicBool>) -> Self {
        let ended = inner.is_end_stream();
        Self {
            inner,
            abandoned,
            ended,
        }
    }
}

impl Body for ObservedBody {
    type Data = Bytes;
    type Error = hyper::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, hyper::Error>>> {
        let result = Pin::new(&mut self.inner).poll_frame(cx);
        if matches!(result, Poll::Ready(None | Some(Err(_)))) || self.inner.is_end_stream() {
            self.ended = true;
        }
        result
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

impl Drop for ObservedBody {
    fn drop(&mut self) {
        if !self.ended {
            self.abandoned.store(true, Ordering::Release);
        }
    }
}

pub(super) struct ClosingIo<I> {
    inner: I,
    abandoned: Arc<AtomicBool>,
    remaining: usize,
    grace: std::time::Duration,
    deadline: Instant,
    timer: Option<Pin<Box<Sleep>>>,
    write_shutdown: bool,
    buffer: [u8; 1024],
}

impl<I> ClosingIo<I> {
    pub(super) fn new(
        inner: I,
        abandoned: Arc<AtomicBool>,
        limits: Limits,
        deadline: Instant,
    ) -> Self {
        Self {
            inner,
            abandoned,
            // Reuse explicit transport budgets, never enlarge a request limit.
            remaining: limits.buffer_bytes,
            grace: limits.header_timeout,
            deadline,
            timer: None,
            write_shutdown: false,
            buffer: [0; 1024],
        }
    }
}

impl<I: AsyncRead + Unpin> AsyncRead for ClosingIo<I> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buffer)
    }
}

impl<I: AsyncRead + AsyncWrite + Unpin> AsyncWrite for ClosingIo<I> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, bytes)
    }
    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write_vectored(cx, bytes)
    }
    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if !this.write_shutdown {
            // Hyper has flushed the response before entering this method.
            // End only the write half; keep accepting the already refused input.
            ready!(Pin::new(&mut this.inner).poll_shutdown(cx))?;
            this.write_shutdown = true;
        }
        if !this.abandoned.load(Ordering::Acquire) {
            return Poll::Ready(Ok(()));
        }
        let timer = this.timer.get_or_insert_with(|| {
            let until = Instant::now()
                .checked_add(this.grace)
                .unwrap_or(this.deadline)
                .min(this.deadline);
            Box::pin(tokio::time::sleep_until(until))
        });
        // Yield even when a producer keeps the socket permanently readable.
        for _ in 0..16 {
            if timer.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "rejected request cleanup deadline",
                )));
            }
            if this.remaining == 0 {
                return Poll::Ready(Err(io::Error::other("rejected request cleanup byte limit")));
            }
            let length = this.remaining.min(this.buffer.len());
            let mut buffer = ReadBuf::new(&mut this.buffer[..length]);
            ready!(Pin::new(&mut this.inner).poll_read(cx, &mut buffer))?;
            let received = buffer.filled().len();
            if received == 0 {
                return Poll::Ready(Ok(()));
            }
            this.remaining -= received;
        }
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}
