//! Enforce the original read budget independently of parser buffer capacity.
use std::{
    io::{self, IoSlice},
    pin::Pin,
    task::{Context, Poll, ready},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub(super) struct ReadCap<I> {
    inner: I,
    maximum: usize,
}

impl<I> ReadCap<I> {
    pub(super) const fn new(inner: I, maximum: usize) -> Self {
        Self { inner, maximum }
    }
}

impl<I: AsyncRead + Unpin> AsyncRead for ReadCap<I> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let length = self.maximum.min(buffer.remaining());
        if length == 0 {
            return Poll::Ready(Ok(()));
        }
        let read = {
            // Safe initialization preserves the original ReadBuf bookkeeping;
            // no allocation, read thread, or larger caller buffer is introduced.
            let mut limited = ReadBuf::new(buffer.initialize_unfilled_to(length));
            ready!(Pin::new(&mut self.inner).poll_read(cx, &mut limited))?;
            limited.filled().len()
        };
        buffer.advance(read);
        Poll::Ready(Ok(()))
    }
}

impl<I: AsyncWrite + Unpin> AsyncWrite for ReadCap<I> {
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

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_larger_caller_buffer_does_not_change_the_original_read_budget() {
        let input = vec![b'x'; 131072];
        let mut capped = ReadCap::new(input.as_slice(), 16384);
        let mut target = vec![b'p'; 32768];
        let mut buffer = ReadBuf::new(&mut target);
        buffer.advance(7);
        std::future::poll_fn(|cx| Pin::new(&mut capped).poll_read(cx, &mut buffer))
            .await
            .unwrap();
        assert_eq!(buffer.filled().len(), 7 + 16384);
        assert_eq!(&buffer.filled()[..7], b"ppppppp");
        assert!(buffer.filled()[7..].iter().all(|byte| *byte == b'x'));
        assert!(target[7 + 16384..].iter().all(|byte| *byte == b'p'));
    }

    struct PendingOrError(bool);
    impl AsyncRead for PendingOrError {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            assert_eq!(buffer.remaining(), 4);
            if self.0 {
                Poll::Pending
            } else {
                Poll::Ready(Err(io::Error::other("fixed failure")))
            }
        }
    }

    #[tokio::test]
    async fn pending_and_errors_do_not_become_successful_eof_or_new_bytes() {
        let mut capped = ReadCap::new(PendingOrError(true), 4);
        let mut target = [0; 32];
        let mut buffer = ReadBuf::new(&mut target);
        buffer.advance(3);
        assert!(
            std::future::poll_fn(|cx| Poll::Ready(
                Pin::new(&mut capped).poll_read(cx, &mut buffer).is_pending()
            ))
            .await
        );
        assert_eq!(buffer.filled().len(), 3);
        capped.inner.0 = false;
        let error = std::future::poll_fn(|cx| Pin::new(&mut capped).poll_read(cx, &mut buffer))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(buffer.filled().len(), 3);
    }
}
