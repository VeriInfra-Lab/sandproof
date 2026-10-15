use super::{Error, Prepared, config::CONFIG_BYTES};
use crate::{MetadataSource, MmdsClient, startup::VmIdentity};
use sandproof_guest_runtime::{
    file_download::local_zone::Snapshot, file_mime::Catalog, pid_claims::PidClaims, startup_files,
};
use std::{fs::File, future::Future};
use tokio::{
    net::TcpListener,
    time::{Instant, timeout_at},
};

struct Inputs {
    directory: File,
    mime: Catalog,
    zone: Snapshot,
}

fn timely(deadline: Instant) -> Result<(), Error> {
    if Instant::now() >= deadline {
        Err(Error::DeadlineExceeded)
    } else {
        Ok(())
    }
}

impl Prepared {
    fn deadline(&self) -> Result<Instant, Error> {
        Instant::now()
            .checked_add(self.config.startup.timeout)
            .ok_or(Error::Configuration)
    }

    // One consumed startup, one blocking job, no retry queue. The task owns
    // every path and opened FD through its actual exit even after cancellation.
    async fn inputs(&self, deadline: Instant) -> Result<Inputs, Error> {
        let directory = self.config.paths.pid_directory.clone();
        let gate = self.config.paths.gate_directory.clone();
        let worker = self.config.paths.worker.clone();
        let owner = self.config.paths.owner_uid;
        let timezone = self.config.snapshots.timezone.clone();
        let zone_limits = self.config.snapshots.timezone_limits;
        let mime_limits = self.config.snapshots.mime;
        let job = tokio::task::spawn_blocking(move || {
            timely(deadline)?;
            let directory = startup_files::state_directory(&directory, owner, CONFIG_BYTES)
                .map_err(|_| Error::Files)?;
            drop(
                startup_files::state_directory(&gate, owner, CONFIG_BYTES)
                    .map_err(|_| Error::Files)?,
            );
            drop(startup_files::worker(&worker, owner, CONFIG_BYTES).map_err(|_| Error::Files)?);
            timely(deadline)?;
            let mime = Catalog::load_system(mime_limits).map_err(|_| Error::Snapshots)?;
            timely(deadline)?;
            let zone = Snapshot::load_file(&timezone, zone_limits).map_err(|_| Error::Snapshots)?;
            timely(deadline)?;
            Ok(Inputs {
                directory,
                mime,
                zone,
            })
        });
        timeout_at(deadline, job)
            .await
            .map_err(|_| Error::DeadlineExceeded)?
            .map_err(|_| Error::WorkerUnavailable)?
    }

    /// Explicit known-new VM provisioning. The trusted installer creates the
    /// private directories first. Existing PID files are never replaced, even
    /// after an incomplete earlier initialization; there is no repair/retry.
    /// This is not called by serve and does not start or authenticate a service.
    /// # Errors
    /// Input, metadata, storage or original startup deadline failure. Failure
    /// may leave an incomplete or complete file; it never implies no effect.
    pub async fn initialize(self) -> Result<(), Error> {
        let deadline = self.deadline()?;
        let inputs = self.inputs(deadline).await?;
        let source =
            MmdsClient::firecracker(self.config.metadata).map_err(|_| Error::Configuration)?;
        let metadata = timeout_at(deadline, source.fetch())
            .await
            .map_err(|_| Error::DeadlineExceeded)?
            .map_err(|e| Error::Startup(crate::startup::Error::Metadata(e)))?;
        let identity = VmIdentity::new(
            metadata.instance_id,
            metadata.template_id,
            self.config.startup.identity_bytes,
        )
        .map_err(Error::Startup)?;
        let owner = self.config.paths.owner_uid;
        let max_pid = self.config.paths.max_pid;
        let (cancelled, _alive) = tokio::sync::oneshot::channel::<()>();
        let job = tokio::task::spawn_blocking(move || {
            timely(deadline)?;
            if cancelled.is_closed() {
                return Err(Error::Cancelled);
            }
            let claims = PidClaims::create(inputs.directory, owner, identity.binding(), max_pid)
                .map_err(|e| Error::Startup(crate::startup::Error::Storage(e)))?;
            drop(claims);
            timely(deadline)
        });
        timeout_at(deadline, job)
            .await
            .map_err(|_| Error::DeadlineExceeded)?
            .map_err(|_| Error::WorkerUnavailable)?
    }

    /// Open original state, consume one durable service session and assemble
    /// all routes before binding. Every startup stage shares this deadline;
    /// cancellation cannot undo already entered blocking I/O or burned numbers.
    /// Shutdown stops transport admission and uses the original drain semantics;
    /// it is not a receipt that old commands/descendants or file effects stopped.
    /// # Errors
    /// Invalid inputs, unavailable metadata/state, deadline, bind or accept
    /// failure. An error never requests replay of a command or file operation.
    pub async fn serve(
        self,
        shutdown: impl Future<Output = ()>,
    ) -> Result<crate::listener::Report, Error> {
        let deadline = self.deadline()?;
        let inputs = self.inputs(deadline).await?;
        let opened = timeout_at(
            deadline,
            self.startup.open(
                inputs.directory,
                self.config.paths.owner_uid,
                self.config.paths.max_pid,
                self.config.initialization,
            ),
        )
        .await
        .map_err(|_| Error::DeadlineExceeded)?
        .map_err(Error::Startup)?;
        timely(deadline)?;
        let address = self.config.listen;
        let (http, listener) = self.assemble(opened, inputs.mime, inputs.zone)?;
        let socket = timeout_at(deadline, TcpListener::bind(address))
            .await
            .map_err(|_| Error::DeadlineExceeded)?
            .map_err(|_| Error::Listener)?;
        timely(deadline)?;
        http.serve(listener, socket, shutdown)
            .await
            .map_err(|_| Error::Listener)
    }
}
