//! Idle I/O budgets, separate from command execution and sandbox lifetime.

use super::*;
use axum::{body::Bytes, serve::Listener};
use hyper::body::{Frame, SizeHint};
use std::{
    future::Future,
    net::SocketAddr,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpStream},
    time::{Instant, Sleep},
};

#[derive(Clone)]
pub(super) struct Activity {
    progress: Arc<Mutex<Instant>>,
    timed_out: Arc<AtomicBool>,
}

impl Activity {
    pub fn new() -> Self {
        Self {
            progress: Arc::new(Mutex::new(Instant::now())),
            timed_out: Arc::new(AtomicBool::new(false)),
        }
    }

    fn mark(&self) {
        *self.progress.lock().unwrap() = Instant::now();
    }

    pub fn timed_out(&self) -> bool {
        self.timed_out.load(Ordering::Acquire)
    }

    pub async fn stalled(&self, idle: Duration) {
        loop {
            let deadline = *self.progress.lock().unwrap() + idle;
            tokio::time::sleep_until(deadline).await;
            if Instant::now() >= *self.progress.lock().unwrap() + idle {
                return;
            }
        }
    }
}

pub(super) struct IdleBody {
    inner: Body,
    idle: Duration,
    deadline: Option<Pin<Box<Sleep>>>,
    activity: Activity,
    ended: bool,
}

impl IdleBody {
    pub fn wrap(inner: Body, seconds: u64, activity: Activity) -> Body {
        Body::new(Self {
            inner,
            idle: Duration::from_secs(seconds),
            deadline: None,
            activity,
            ended: false,
        })
    }
}

impl hyper::body::Body for IdleBody {
    type Data = Bytes;
    type Error = Box<dyn Error + Send + Sync>;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        if self.ended {
            return Poll::Ready(None);
        }
        match Pin::new(&mut self.inner).poll_frame(cx) {
            Poll::Pending => {
                if self.deadline.is_none() {
                    self.deadline = Some(Box::pin(tokio::time::sleep(self.idle)));
                }
                if self.deadline.as_mut().unwrap().as_mut().poll(cx).is_ready() {
                    self.ended = true;
                    self.activity.timed_out.store(true, Ordering::Release);
                    return Poll::Ready(Some(Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "body producer idle timeout",
                    )
                    .into())));
                }
                Poll::Pending
            }
            Poll::Ready(frame) => {
                if frame.as_ref().is_some_and(|frame| {
                    frame
                        .as_ref()
                        .is_ok_and(|frame| frame.data_ref().is_some_and(|data| !data.is_empty()))
                }) {
                    self.activity.mark();
                    self.deadline = None;
                }
                if frame.is_none() {
                    self.ended = true;
                }
                Poll::Ready(frame.map(|frame| frame.map_err(Into::into)))
            }
        }
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

/// Times only a pending write/flush/shutdown, never a quiet command waiting
/// to produce output. Used on the frontend and the namespace relay pipes.
pub(crate) struct WriteIdle<T> {
    inner: T,
    idle: Option<Duration>,
    deadline: Option<Pin<Box<Sleep>>>,
}

impl<T> WriteIdle<T> {
    pub fn new(inner: T, idle: Option<Duration>) -> Self {
        Self {
            inner,
            idle,
            deadline: None,
        }
    }

    fn check<R>(
        &mut self,
        cx: &mut Context<'_>,
        result: Poll<io::Result<R>>,
    ) -> Poll<io::Result<R>> {
        if result.is_ready() {
            self.deadline = None;
            return result;
        }
        if let Some(idle) = self.idle {
            let deadline = self
                .deadline
                .get_or_insert_with(|| Box::pin(tokio::time::sleep(idle)));
            if deadline.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "transport write idle timeout",
                )));
            }
        }
        Poll::Pending
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for WriteIdle<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for WriteIdle<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write(cx, buf);
        self.check(cx, result)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let result = Pin::new(&mut self.inner).poll_flush(cx);
        self.check(cx, result)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let result = Pin::new(&mut self.inner).poll_shutdown(cx);
        self.check(cx, result)
    }
}

pub(crate) struct GatewayListener {
    listener: TcpListener,
    idle: Option<Duration>,
}

impl GatewayListener {
    pub fn new(listener: TcpListener, idle: Option<Duration>) -> Self {
        Self { listener, idle }
    }
}

impl Listener for GatewayListener {
    type Io = WriteIdle<TcpStream>;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        let (stream, address) = Listener::accept(&mut self.listener).await;
        (WriteIdle::new(stream, self.idle), address)
    }
    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test(start_paused = true)]
    async fn stalled_writer_times_out_but_quiet_reader_does_not() {
        let (writer, _reader) = tokio::io::duplex(1);
        let mut writer = WriteIdle::new(writer, Some(Duration::from_secs(5)));
        let error = writer.write_all(b"ab").await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        let mut byte = [0];
        assert!(
            tokio::time::timeout(Duration::from_secs(20), writer.read_exact(&mut byte))
                .await
                .is_err()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn writer_can_progress_for_longer_than_one_idle_window() {
        let (writer, mut reader) = tokio::io::duplex(1);
        let mut writer = WriteIdle::new(writer, Some(Duration::from_secs(5)));
        let start = Instant::now();
        let consumer = tokio::spawn(async move {
            let mut result = Vec::new();
            for _ in 0..3 {
                tokio::time::sleep(Duration::from_secs(4)).await;
                result.push(reader.read_u8().await.unwrap());
            }
            result
        });
        writer.write_all(b"abc").await.unwrap();
        assert!(start.elapsed() > Duration::from_secs(5));
        assert_eq!(consumer.await.unwrap(), b"abc");
    }

    #[tokio::test(start_paused = true)]
    async fn idle_body_fails_without_claiming_successful_eof() {
        struct Pending;
        impl hyper::body::Body for Pending {
            type Data = Bytes;
            type Error = io::Error;
            fn poll_frame(
                self: Pin<&mut Self>,
                _: &mut Context<'_>,
            ) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
                Poll::Pending
            }
        }
        let activity = Activity::new();
        let mut body = IdleBody::wrap(Body::new(Pending), 5, activity.clone());
        assert!(body.frame().await.unwrap().is_err());
        assert!(activity.timed_out());
        assert!(body.frame().await.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn header_idle_clock_follows_upload_progress() {
        let activity = Activity::new();
        let waiting = activity.clone();
        let waiter = tokio::spawn(async move { waiting.stalled(Duration::from_secs(5)).await });
        for _ in 0..5 {
            tokio::time::sleep(Duration::from_secs(4)).await;
            activity.mark();
            assert!(!waiter.is_finished());
        }
        tokio::time::sleep(Duration::from_secs(6)).await;
        waiter.await.unwrap();
    }
}
