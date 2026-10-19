//! Process-wide observations of tasks submitted by this crate, across runtimes.
//! Counts cover submission through destruction of the future/closure, including
//! queued blocking work. They are not admission limits or cleanup receipts.
//! Returned values, task scheduler storage, dependency-internal tasks, the root
//! `block_on` future, threads, child processes and memory require separate accounting.
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, LazyLock, Mutex, MutexGuard},
    task::{Context, Poll},
};
use tokio::task::JoinHandle;

#[cfg(test)]
mod tests;

static PROCESS: LazyLock<Meter> = LazyLock::new(Meter::default);

// Keep the report inventory identical on platforms without Linux handlers.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Clone, Copy)]
pub(crate) enum AsyncTask {
    Connection,
    CommandPreparation,
    CommandDriver,
    CommandRegistry,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Clone, Copy)]
pub(crate) enum BlockingTask {
    BodyStream,
    BodyEncode,
    CommandDecode,
    FileDecode,
    FileOperation,
    FileUpload,
    FileDownload,
    AccountLookup,
    StartupOpen,
    DaemonInitialize,
    DaemonLoad,
    PidRegistration,
    CommandPreparation,
}

/// A live count and the maximum simultaneously live count since process start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Count {
    pub live: usize,
    pub peak: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Role {
    pub name: &'static str,
    pub count: Count,
}

/// Fixed role inventory; no request values, task identities or history are kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Group<const N: usize> {
    pub count: Count,
    #[serde(serialize_with = "serialize_roles")]
    pub roles: [Role; N],
}

fn serialize_roles<S: serde::Serializer, const N: usize>(
    roles: &[Role; N],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serde::Serialize::serialize(roles.as_slice(), serializer)
}

/// One coherent snapshot under the same lock used by all counter transitions.
/// `exact = false` is sticky after arithmetic failure or a poisoned lock; such
/// a report must not be used as an exact count or a successful resource receipt.
/// The combined peak is simultaneous, not the sum of the two group peaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Snapshot {
    pub schema_version: u8,
    pub scope: &'static str,
    pub exact: bool,
    pub total: Count,
    pub asynchronous: Group<4>,
    pub blocking: Group<13>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            schema_version: 1,
            scope: "process_service_owned_tasks",
            exact: true,
            total: Count::default(),
            asynchronous: Group::new([
                "connection",
                "command_preparation",
                "command_driver",
                "command_registry",
            ]),
            blocking: Group::new([
                "body_stream",
                "body_encode",
                "command_decode",
                "file_decode",
                "file_operation",
                "file_upload",
                "file_download",
                "account_lookup",
                "startup_open",
                "daemon_initialize",
                "daemon_load",
                "pid_registration",
                "command_preparation",
            ]),
        }
    }
}

impl Count {
    fn change(&mut self, entering: bool) -> bool {
        let next = if entering {
            self.live.checked_add(1)
        } else {
            self.live.checked_sub(1)
        };
        let Some(next) = next else { return false };
        self.live = next;
        self.peak = self.peak.max(next);
        true
    }
}

impl<const N: usize> Group<N> {
    fn new(names: [&'static str; N]) -> Self {
        Self {
            count: Count::default(),
            roles: names.map(|name| Role {
                name,
                count: Count::default(),
            }),
        }
    }

    fn change(&mut self, index: usize, entering: bool) -> bool {
        // Evaluate both even if one counter has become inexact.
        self.count.change(entering) & self.roles[index].count.change(entering)
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Async(AsyncTask),
    Blocking(BlockingTask),
}

#[derive(Clone, Default)]
struct Meter(Arc<Mutex<Snapshot>>);

impl Meter {
    fn lock(&self) -> MutexGuard<'_, Snapshot> {
        self.0.lock().unwrap_or_else(|poisoned| {
            let mut state = poisoned.into_inner();
            state.exact = false;
            state
        })
    }

    fn change(&self, kind: Kind, entering: bool) {
        let mut state = self.lock();
        let valid = match kind {
            Kind::Async(role) => state.asynchronous.change(role as usize, entering),
            Kind::Blocking(role) => state.blocking.change(role as usize, entering),
        };
        let total_valid = state.total.change(entering);
        state.exact &= valid & total_valid;
    }

    fn begin(&self, kind: Kind) -> Guard {
        self.change(kind, true);
        Guard {
            meter: self.clone(),
            kind,
        }
    }

    fn track<F: Future>(&self, role: AsyncTask, work: F) -> Tracked<F> {
        let guard = self.begin(Kind::Async(role));
        Tracked {
            work: Box::pin(work),
            _guard: guard,
        }
    }

    fn blocking<F: FnOnce() -> T + Send + 'static, T: Send + 'static>(
        &self,
        role: BlockingTask,
        work: F,
    ) -> JoinHandle<T> {
        let job = Blocking {
            work: Some(work),
            _guard: self.begin(Kind::Blocking(role)),
        };
        crate::blocking_executor::spawn(move || job.run())
    }
}

struct Guard {
    meter: Meter,
    kind: Kind,
}
impl Drop for Guard {
    fn drop(&mut self) {
        self.meter.change(self.kind, false);
    }
}

// Field order is essential even before the first poll: destroy captured owners
// before crediting the task. Pin<Box<F>> avoids unsafe projection in this crate.
struct Tracked<F> {
    work: Pin<Box<F>>,
    _guard: Guard,
}
impl<F: Future> Future for Tracked<F> {
    type Output = F::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().work.as_mut().poll(cx)
    }
}

// An aborted queued closure must destroy its captures before its guard, too.
struct Blocking<F> {
    work: Option<F>,
    _guard: Guard,
}
impl<F> Blocking<F> {
    fn run<T>(mut self) -> T
    where
        F: FnOnce() -> T,
    {
        self.work.take().expect("blocking job consumed once")()
    }
}

/// Read process-wide counters without resetting peaks or allocating a history.
/// Multiple embedded services/runtimes in one process share this observation.
#[must_use]
pub fn snapshot() -> Snapshot {
    *PROCESS.lock()
}

pub(crate) fn track<F: Future>(role: AsyncTask, work: F) -> impl Future<Output = F::Output> {
    PROCESS.track(role, work)
}

#[cfg(target_os = "linux")]
pub(crate) fn spawn<F: Future + Send + 'static>(role: AsyncTask, work: F) -> JoinHandle<F::Output>
where
    F::Output: Send + 'static,
{
    tokio::spawn(track(role, work))
}

pub(crate) fn spawn_blocking<F: FnOnce() -> T + Send + 'static, T: Send + 'static>(
    role: BlockingTask,
    work: F,
) -> JoinHandle<T> {
    PROCESS.blocking(role, work)
}
