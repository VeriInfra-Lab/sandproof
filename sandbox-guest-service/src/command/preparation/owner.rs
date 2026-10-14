use super::{
    AdmissionLimits, AdmissionStatus, CommandError, CommandPreparation, CommandReservation,
    Context, Decision, Instant, PreparationSpec, Reply, Worker, oneshot, watch,
};

/// Constructed before spawn, so even an unpolled task resolves its observers.
pub(super) struct AdmissionOwner {
    reservation: Option<CommandReservation>,
    status: watch::Sender<AdmissionStatus>,
    reply: Option<oneshot::Sender<Reply>>,
    unsettled: bool,
    finished: bool,
}

impl AdmissionOwner {
    pub(super) fn new(
        reservation: CommandReservation,
        status: watch::Sender<AdmissionStatus>,
        reply: oneshot::Sender<Reply>,
    ) -> Self {
        Self {
            reservation: Some(reservation),
            status,
            reply: Some(reply),
            unsettled: false,
            finished: false,
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
    tokio::spawn(run(spec, context, decision, owner));
}

async fn run(
    spec: PreparationSpec,
    context: Context,
    mut decision: oneshot::Receiver<Decision>,
    mut owner: AdmissionOwner,
) {
    let Context {
        authorization,
        execution,
        limits,
        until,
        request,
    } = context;
    if owner.reply.as_ref().is_none_or(oneshot::Sender::is_closed) {
        owner.stopped(CommandError::Cancelled);
        return;
    }
    if Instant::now() >= until {
        owner.stopped(CommandError::DeadlineExceeded);
        return;
    }
    // Mark before entering spawn: panic between a successful kernel spawn and
    // normal return may have created a child. Error return itself owns none.
    owner.unsettled = true;
    let preparation = CommandPreparation::spawn(
        &spec.worker,
        &spec.private_directory,
        spec.session,
        spec.command,
        &spec.launch,
        spec.process_limits,
    );
    drop(spec);
    let mut preparation = match preparation {
        Ok(preparation) => preparation,
        Err(error) => {
            owner.stopped(prepare_error(&error));
            return;
        }
    };
    let prepared = tokio::select! {
        biased;
        () = tokio::time::sleep_until(until) => Err(CommandError::DeadlineExceeded),
        _ = &mut decision => Err(CommandError::Cancelled),
        result = preparation.finish() => result.map_err(|error| prepare_error(&error)),
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

fn prepare_error(error: &std::io::Error) -> CommandError {
    match error.kind() {
        std::io::ErrorKind::InvalidInput => CommandError::InvalidArgument,
        std::io::ErrorKind::TimedOut => CommandError::DeadlineExceeded,
        _ => CommandError::Unavailable,
    }
}

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
