use super::{
    AdmissionLimits, AdmissionStatus, CommandError, CommandPreparation, CommandReservation,
    Context, Decision, Instant, PreparationSpec, Reply, Worker, oneshot, prepare_error, watch,
};
use crate::task_accounting::{self, AsyncTask, BlockingTask};
use sandproof_guest_runtime::PreparationRejection;
use std::sync::{Arc, Mutex};

/// Constructed before spawn, so even an unpolled task resolves its observers.
pub(super) struct AdmissionOwner {
    reservation: Option<CommandReservation>,
    status: watch::Sender<AdmissionStatus>,
    reply: Option<oneshot::Sender<Reply>>,
    unsettled: bool,
    finished: bool,
    rejection: Arc<Mutex<Option<PreparationRejection>>>,
}

impl AdmissionOwner {
    pub(super) fn new(
        reservation: CommandReservation,
        status: watch::Sender<AdmissionStatus>,
        reply: oneshot::Sender<Reply>,
        rejection: Arc<Mutex<Option<PreparationRejection>>>,
    ) -> Self {
        Self {
            reservation: Some(reservation),
            status,
            reply: Some(reply),
            unsettled: false,
            finished: false,
            rejection,
        }
    }

    fn send(&mut self, reply: Reply) {
        if let Some(sender) = self.reply.take() {
            let _ = sender.send(reply);
        }
    }

    fn stopped(&mut self, error: CommandError) {
        self.unsettled = false;
        self.reservation.take(); // actual child wait (or proven no spawn) precedes credit
        self.status.send_replace(AdmissionStatus::Stopped(error));
        self.send(Reply::Stopped(error));
        self.finished = true;
    }
}

impl Drop for AdmissionOwner {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if self.unsettled
            && let Some(reservation) = self.reservation.take()
        {
            reservation.permit.forget(); // unknown responsibility cannot become capacity
        }
        self.status.send_replace(AdmissionStatus::OwnerLost);
        self.send(Reply::Unknown);
    }
}

pub(super) fn spawn(
    spec: PreparationSpec,
    context: Context,
    decision: oneshot::Receiver<Decision>,
    owner: AdmissionOwner,
) {
    task_accounting::spawn(
        AsyncTask::CommandPreparation,
        prepare_and_run(spec, context, decision, owner, spawn_worker),
    );
}

struct Prepared {
    preparation: CommandPreparation,
    context: Context,
    decision: oneshot::Receiver<Decision>,
    owner: AdmissionOwner,
}

async fn prepare_and_run(
    spec: PreparationSpec,
    context: Context,
    decision: oneshot::Receiver<Decision>,
    owner: AdmissionOwner,
    prepare: impl FnOnce(
        PreparationSpec,
        &mut Option<PreparationRejection>,
    ) -> std::io::Result<CommandPreparation>
    + Send
    + 'static,
) {
    // The original record bounds queued/running blocking work. Move every
    // owner into the closure: dropping this await cannot credit work that is
    // still queued, in a syscall, or returning a possibly live gated worker.
    let (waiting, alive) = oneshot::channel::<()>();
    let prepared = task_accounting::spawn_blocking(BlockingTask::CommandPreparation, move || {
        prepare_blocking(spec, context, decision, owner, &waiting, prepare)
    })
    .await;
    drop(alive);
    if let Ok(Some(prepared)) = prepared {
        run(prepared).await;
    }
}

fn spawn_worker(
    spec: PreparationSpec,
    rejection: &mut Option<PreparationRejection>,
) -> std::io::Result<CommandPreparation> {
    let result = CommandPreparation::spawn_observed(
        &spec.worker,
        &spec.private_directory,
        spec.session,
        spec.command,
        &spec.launch,
        spec.process_limits,
        rejection,
    );
    drop(spec);
    result
}

fn prepare_blocking(
    spec: PreparationSpec,
    context: Context,
    mut decision: oneshot::Receiver<Decision>,
    mut owner: AdmissionOwner,
    waiting: &oneshot::Sender<()>,
    prepare: impl FnOnce(
        PreparationSpec,
        &mut Option<PreparationRejection>,
    ) -> std::io::Result<CommandPreparation>,
) -> Option<Prepared> {
    // Check after queueing, before any filesystem or process operation. No
    // fresh deadline, retry, or replacement worker is created by this hop.
    // A valid Commit cannot precede Ready. Any early decision or a dropped
    // sender withdraws this queued preparation, as does loss of its waiter.
    if waiting.is_closed()
        || owner.reply.as_ref().is_none_or(oneshot::Sender::is_closed)
        || !matches!(
            decision.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        )
    {
        owner.stopped(CommandError::Cancelled);
        return None;
    }
    if Instant::now() >= context.until {
        owner.stopped(CommandError::DeadlineExceeded);
        return None;
    }
    // Mark before entering spawn: panic between a successful kernel spawn and
    // normal return may have created a child. Error return itself owns none.
    owner.unsettled = true;
    let mut rejection = None;
    let preparation = match prepare(spec, &mut rejection) {
        Ok(preparation) => preparation,
        Err(error) => {
            if let Ok(mut fact) = owner.rejection.lock() {
                *fact = rejection;
            }
            owner.stopped(prepare_error(&error));
            return None;
        }
    };
    Some(Prepared {
        preparation,
        context,
        decision,
        owner,
    })
}

async fn run(prepared: Prepared) {
    let Prepared {
        mut preparation,
        context,
        mut decision,
        mut owner,
    } = prepared;
    let Context {
        authorization,
        execution,
        limits,
        until,
        request,
    } = context;
    let prepared = tokio::select! {
        biased;
        () = tokio::time::sleep_until(until) => Err(CommandError::DeadlineExceeded),
        _ = &mut decision => Err(CommandError::Cancelled),
        result = preparation.finish() => {
            if result.is_err() && let Ok(mut fact) = owner.rejection.lock() {
                *fact = preparation.rejection();
            }
            result.map_err(|error| prepare_error(&error))
        },
    };
    let process = match prepared {
        Ok(process) => process,
        Err(error) => {
            drop(authorization); // withdrawn dispatch can no longer release user code
            clean(
                Worker::Preparing(Box::new(preparation)),
                error,
                limits,
                owner,
            )
            .await;
            return;
        }
    };
    drop(preparation); // original child/directory have moved, close the handshake listener
    let identity = process.identity();
    owner.status.send_replace(AdmissionStatus::Ready(identity));
    let confirm = tokio::select! {
        biased;
        () = tokio::time::sleep_until(until) => Err(CommandError::DeadlineExceeded),
        result = &mut decision => match result {
            Ok(Decision::Commit(actual)) if actual == identity => Ok(()),
            Ok(Decision::Commit(_)) => Err(CommandError::StaleIdentity),
            Ok(Decision::Abort) | Err(_) => Err(CommandError::Cancelled),
        },
    };
    // Final acceptance boundary: all prior time/cancellation checks still
    // leave the worker unreleased. After this point, reply loss is unknown.
    let confirm = confirm.and_then(|()| {
        if Instant::now() >= until {
            Err(CommandError::DeadlineExceeded)
        } else if owner.reply.as_ref().is_none_or(oneshot::Sender::is_closed) {
            Err(CommandError::Cancelled)
        } else {
            Ok(())
        }
    });
    if let Err(error) = confirm {
        drop(authorization);
        clean(Worker::Ready(Box::new(process)), error, limits, owner).await;
        return;
    }
    if let Some(reservation) = owner.reservation.take() {
        let launch =
            reservation.start_registered_admitted(process, authorization, execution, request);
        owner
            .status
            .send_replace(AdmissionStatus::Dispatched(identity));
        owner.finished = true;
        owner.send(Reply::Dispatched(launch)); // dropped reply cannot cancel the new owner
    }
}

#[cfg(test)]
mod tests;

async fn clean(
    mut worker: Worker,
    error: CommandError,
    limits: AdmissionLimits,
    mut owner: AdmissionOwner,
) {
    owner.status.send_replace(AdmissionStatus::Stopping(error));
    loop {
        if matches!(
            tokio::time::timeout(limits.cleanup_timeout, worker.abort()).await,
            Ok(Ok(()))
        ) {
            drop(worker); // close the original handles/private directory before reporting stopped
            owner.stopped(error);
            return;
        }
        owner.status.send_replace(AdmissionStatus::CleanupUnknown);
        owner.send(Reply::Unknown);
        tokio::time::sleep(limits.retry_delay).await;
    }
}
