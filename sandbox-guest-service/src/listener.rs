//! Bounded HTTP/1 transport for explicitly supplied guest routes.
//!
//! This does not install a daemon, select routes or authorize requests. The
//! caller supplies an already-bound listener, all limits and a handler. Kernel
//! backlog/socket buffers, handler allocations and blocking work have separate
//! owners and budgets; Hyper's buffer limit is not a process memory bound.
use axum::{body::Body, http::Request, response::Response};
use hyper::{server::conn::http1, service::service_fn};
use hyper_util::rt::{TokioIo, TokioTimer};
use std::{fmt, future::Future, io, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpListener,
    sync::watch,
    task::JoinSet,
    time::Instant,
};

#[cfg(test)]
mod tests;

/// Trusted transport configuration, with no production defaults. The absolute
/// connection lifetime covers headers, body, handler, response and keep-alive;
/// it is never restarted by bytes or another request. Closing transport does
/// not establish that a file effect or command stopped, and never retries it.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub connections: usize,
    pub header_count: usize,
    pub buffer_bytes: usize,
    pub header_timeout: Duration,
    pub connection_timeout: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidLimits;

pub struct Listener {
    limits: Limits,
}

// Pinned Hyper 1.11.1 h1::role::Server::parse retains HeaderIndices (two
// usize pairs) and httparse::Header (str/byte slices), four words each. Check
// their combined scratch arithmetic before any connection can request these
// allocations. This is not a process memory bound or an allocation guarantee;
// dependency upgrades must recheck the parser's storage representation.
const HEADER_SCRATCH_BYTES: usize = 8 * std::mem::size_of::<usize>();

/// Transport observations only, not successful operation receipts. Counters
/// saturate rather than wrap. No request headers, paths or errors are retained.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub admitted: u64,
    pub refused: u64,
    pub completed: u64,
    pub failed: u64,
    pub expired: u64,
    pub peak_connections: usize,
}

impl Listener {
    /// # Errors
    /// Rejects empty budgets, Hyper buffers below its 8192-byte minimum and
    /// deadlines or parser scratch/buffer sizes that cannot be represented.
    /// Header time may not exceed the connection lifetime. Configuration is
    /// checked before accepting sockets, without allocating parser storage.
    pub fn new(limits: Limits) -> Result<Self, InvalidLimits> {
        let now = Instant::now();
        if limits.connections == 0
            || limits.header_count == 0
            || limits
                .header_count
                .checked_mul(HEADER_SCRATCH_BYTES)
                .is_none_or(|bytes| bytes > isize::MAX as usize)
            || limits.buffer_bytes < 8192
            || limits.buffer_bytes > isize::MAX as usize
            || limits.header_timeout.is_zero()
            || limits.connection_timeout.is_zero()
            || limits.header_timeout > limits.connection_timeout
            || now.checked_add(limits.connection_timeout).is_none()
        {
            return Err(InvalidLimits);
        }
        Ok(Self { limits })
    }

    /// Accept with no application wait queue: excess sockets are closed before
    /// spawning a task or reading headers. Finished tasks are reaped before new
    /// admission. HTTP/1 keep-alive is allowed within the original deadline.
    ///
    /// Handler errors close the connection without inventing an HTTP mapping or
    /// exposing their contents; the error remains owned until Hyper finishes.
    /// Body errors likewise cannot become a complete success. On shutdown stop
    /// accepting and drain existing connections under their original deadlines.
    /// Dropping this future aborts transport tasks; separately owned blocking
    /// work must retain its original capacity until it actually ends.
    /// # Errors
    /// Socket acceptance failure. A single connection task panic is counted as
    /// failed and does not stop sibling connections or admission. Existing
    /// transport tasks are aborted on an accept error; effects are never replayed.
    pub async fn serve<H, F, E>(
        self,
        listener: TcpListener,
        handler: H,
        shutdown: impl Future<Output = ()>,
    ) -> io::Result<Report>
    where
        H: Fn(Request<Body>) -> F + Clone + Send + Sync + 'static,
        F: Future<Output = Result<Response, E>> + Send + 'static,
        E: Send + Sync + 'static,
    {
        let mut tasks = JoinSet::new();
        let mut report = Report::default();
        let (stop, stopped) = watch::channel(false);
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => break,
                Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                    report.record(result.unwrap_or(Outcome::Failed));
                }
                accepted = listener.accept() => {
                    let (socket, _) = accepted?;
                    if tasks.len() >= self.limits.connections {
                        report.refused = report.refused.saturating_add(1);
                        drop(socket);
                        continue;
                    }
                    let deadline = Instant::now()
                        .checked_add(self.limits.connection_timeout)
                        .ok_or_else(|| io::Error::other("unrepresentable connection deadline"))?;
                    report.admitted = report.admitted.saturating_add(1);
                    let handler = handler.clone();
                    let limits = self.limits;
                    tasks.spawn(connection(socket, handler, limits, deadline, stopped.clone()));
                    report.peak_connections = report.peak_connections.max(tasks.len());
                }
            }
        }
        // Release the listening socket now, not after active connections drain.
        drop(listener);
        stop.send_replace(true);
        while let Some(result) = tasks.join_next().await {
            report.record(result.unwrap_or(Outcome::Failed));
        }
        Ok(report)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Completed,
    Failed,
    Expired,
}

async fn connection<I, H, F, E>(
    socket: I,
    handler: H,
    limits: Limits,
    deadline: Instant,
    mut stopped: watch::Receiver<bool>,
) -> Outcome
where
    I: AsyncRead + AsyncWrite + Unpin,
    H: Fn(Request<Body>) -> F,
    F: Future<Output = Result<Response, E>>,
    E: Send + Sync + 'static,
{
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .max_headers(limits.header_count)
        .max_buf_size(limits.buffer_bytes)
        .header_read_timeout(limits.header_timeout);
    let service = service_fn(move |request: Request<hyper::body::Incoming>| {
        let future = handler(request.map(Body::new));
        async move { future.await.map_err(HandlerFailure) }
    });
    let connection = builder.serve_connection(TokioIo::new(socket), service);
    tokio::pin!(connection);
    if *stopped.borrow() {
        connection.as_mut().graceful_shutdown();
    } else {
        tokio::select! {
            biased;
            () = tokio::time::sleep_until(deadline) => return Outcome::Expired,
            _ = stopped.changed() => connection.as_mut().graceful_shutdown(),
            result = &mut connection => return outcome(&result),
        }
    }
    tokio::select! {
        biased;
        () = tokio::time::sleep_until(deadline) => Outcome::Expired,
        result = connection => outcome(&result),
    }
}

fn outcome(result: &Result<(), hyper::Error>) -> Outcome {
    if result.is_ok() {
        Outcome::Completed
    } else {
        Outcome::Failed
    }
}
impl Report {
    fn record(&mut self, outcome: Outcome) {
        let counter = match outcome {
            Outcome::Completed => &mut self.completed,
            Outcome::Failed => &mut self.failed,
            Outcome::Expired => &mut self.expired,
        };
        *counter = counter.saturating_add(1);
    }
}

// Keep the typed failure and any capacity it owns through Hyper's error path,
// while withholding even its Debug representation from transport diagnostics.
struct HandlerFailure<E>(E);
impl<E> fmt::Debug for HandlerFailure<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GuestHandlerFailure")
    }
}
impl<E> fmt::Display for HandlerFailure<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("guest request failed")
    }
}
impl<E: Send + Sync + 'static> std::error::Error for HandlerFailure<E> {}
