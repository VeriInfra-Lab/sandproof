use std::future::pending;
use std::sync::PoisonError;

use sandproof_guest_runtime::{CommandEvent, LaunchObservation};

use super::{
    Arc, AuthPermit, CommandError, CommandExit, CommandProcess, DeadlineAction, Duration,
    ExecutionDeadline, Instant, LaunchStatus, ProcessSignal, Shared,
};

struct OwnerGuard {
    shared: Arc<Shared>,
    settled: bool,
}

impl Drop for OwnerGuard {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        // A panic/runtime abort is an unknown owner loss, not capacity credit.
        // Quarantine exactly one slot without leaking the retained output state.
        if let Some(permit) = self
            .shared
            .record
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            permit.forget();
        }
        self.shared
            .streams
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .finish(Err(CommandError::Unavailable));
        self.shared.status.send_modify(|state| {
            state.owner_lost = true;
        });
    }
}

async fn expires(deadline: ExecutionDeadline) {
    match deadline.0 {
        Some(at) => tokio::time::sleep_until(at).await,
        None => pending().await,
    }
}

fn kill_at_deadline(shared: &Shared) {
    let action = if shared
        .control
        .signal(shared.identity, ProcessSignal::Kill)
        .is_ok()
    {
        DeadlineAction::KillSent
    } else {
        DeadlineAction::KillFailed
    };
    shared
        .status
        .send_modify(|state| state.deadline_action = Some(action));
}

fn finish(shared: &Shared, result: Result<CommandExit, CommandError>) {
    if let Ok(mut input) = shared.input.try_lock() {
        input.take();
    }
    shared
        .streams
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .finish(result);
    shared.status.send_modify(|state| {
        state.completion = Some(result);
        state.completed_at = Some(Instant::now());
    });
}

pub(super) fn spawn(
    process: CommandProcess,
    authorization: AuthPermit,
    deadline: ExecutionDeadline,
    request: Arc<dyn Send + Sync>,
    shared: Arc<Shared>,
) {
    // Construct before spawn: dropping an unpolled task must also quarantine
    // its unresolved reservation and wake waiters with owner_lost.
    let guard = OwnerGuard {
        shared: Arc::clone(&shared),
        settled: false,
    };
    tokio::spawn(run(
        process,
        authorization,
        deadline,
        request,
        shared,
        guard,
    ));
}

async fn run(
    mut process: CommandProcess,
    authorization: AuthPermit,
    mut deadline: ExecutionDeadline,
    request: Arc<dyn Send + Sync>,
    shared: Arc<Shared>,
    mut guard: OwnerGuard,
) {
    // This synchronous filesystem step may take time. Recheck the original
    // execution deadline below before any release byte can execute user code.
    process.retire_gate_path();
    // A pre-expired command is aborted before release. No user program ran.
    if process.launch_observation().is_none() && deadline.0.is_some_and(|at| at <= Instant::now()) {
        shared.status.send_modify(|state| {
            state.launch = LaunchStatus::Failed(CommandError::DeadlineExceeded);
        });
        if process.abort_prepared().await.is_ok() {
            finish(&shared, Err(CommandError::DeadlineExceeded));
            guard.settled = true;
        }
        return;
    }
    let launch = if let Some(observation) = process.launch_observation() {
        observation
    } else {
        tokio::select! {
            // If both are ready, expiry wins before a fresh gate release.
            biased;
            () = expires(deadline) => {
                kill_at_deadline(&shared);
                deadline = ExecutionDeadline(None);
                LaunchObservation::Unknown
            }
            result = process.release() => result.unwrap_or(LaunchObservation::Unknown),
        }
    };
    drop(authorization);
    let mut failure = match launch {
        LaunchObservation::NoExecErrorObserved => None,
        LaunchObservation::ExecFailed(code) => Some(CommandError::ExecFailed(code)),
        LaunchObservation::Unknown => Some(CommandError::LaunchUnknown),
    };
    if failure.is_none() {
        // Disabled stdin is a normal closed input; no request owns this handle.
        *shared.input.lock().await = process.take_input(shared.identity).ok();
    }
    shared.status.send_modify(|state| {
        state.launch = failure.map_or(LaunchStatus::Started, LaunchStatus::Failed);
    });
    if let Some(error) = failure {
        shared
            .streams
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .finish(Err(error));
    }
    // A deadline may have won before release's first poll. In that case there
    // is no released child to drain and abort observes the original helper.
    if process.launch_observation().is_none() {
        if process.abort_prepared().await.is_ok() {
            finish(&shared, Err(failure.unwrap_or(CommandError::Unavailable)));
            guard.settled = true;
        }
        return;
    }
    // Preparation and actual release have settled. Later execution, wait and
    // output remain charged to the independent command record/subscriptions.
    drop(request);
    let mut retry = None;
    loop {
        let observation = tokio::select! {
            biased;
            () = expires(deadline) => {
                kill_at_deadline(&shared);
                deadline = ExecutionDeadline(None);
                continue;
            }
            result = observe(&mut process, failure, retry) => result,
        };
        retry = None;
        match observation {
            Ok(Some(CommandEvent::Output { channel, bytes })) => {
                if !publish_output(&mut process, &shared, channel, bytes) {
                    failure = Some(CommandError::Unavailable);
                }
            }
            Ok(Some(CommandEvent::End {
                code,
                signal,
                core_dumped,
            })) => {
                let exit = CommandExit {
                    code,
                    signal,
                    core_dumped,
                };
                finish(&shared, failure.map_or(Ok(exit), Err));
                guard.settled = true;
                return;
            }
            Ok(None) | Err(_) => {
                failure.get_or_insert(CommandError::Unavailable);
                // A persistent wait failure retains ownership/capacity and can
                // still receive expiry/control. Never spin or invent exit.
                retry = Some(Instant::now() + Duration::from_secs(1));
            }
        }
        if let Some(error) = failure {
            shared
                .streams
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .finish(Err(error));
        }
    }
}

fn publish_output(
    process: &mut CommandProcess,
    shared: &Arc<Shared>,
    channel: super::Channel,
    bytes: Vec<u8>,
) -> bool {
    let count = bytes.len();
    shared
        .streams
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .publish(shared, channel, bytes);
    process
        .consume_output(shared.identity, channel, count)
        .is_ok()
}

async fn observe(
    process: &mut CommandProcess,
    failure: Option<CommandError>,
    retry: Option<Instant>,
) -> std::io::Result<Option<CommandEvent>> {
    use std::os::unix::process::ExitStatusExt;
    if let Some(at) = retry {
        tokio::time::sleep_until(at).await;
    }
    if failure.is_some() {
        let status = process.discard_and_wait().await?;
        Ok(Some(CommandEvent::End {
            code: status.code().unwrap_or(-1),
            signal: status.signal(),
            core_dumped: status.core_dumped(),
        }))
    } else {
        process.next_event().await
    }
}
