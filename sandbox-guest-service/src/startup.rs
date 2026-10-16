//! Normal startup binds trusted metadata, initialization and existing PID state.
//! This never provisions missing state or restores an old active command table.
use crate::{InitLimits, Metadata, MetadataSource, ServiceError, Session};
use sandproof_guest_runtime::pid_claims::{self, Binding, PidClaims};
use sha2::{Digest, Sha256};
use std::{fs::File, sync::Arc, time::Duration};
use tokio::{
    sync::{Semaphore, oneshot},
    time::{Instant, timeout_at},
};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    /// Combined retained instance/template UTF-8 bytes. The metadata transport
    /// must separately bound its response and its earlier allocations.
    pub identity_bytes: usize,
    /// One deadline covers metadata and queued/running persistent-state open.
    pub timeout: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidConfiguration,
    InvalidMetadata,
    Busy,
    DeadlineExceeded,
    Cancelled,
    Metadata(ServiceError),
    Storage(pid_claims::Error),
    WorkerUnavailable,
}

/// Immutable host-provided VM creation identity. Its IDs must never name a
/// different VM incarnation; tokens and daemon restarts do not change them.
/// This representation is not evidence that caller-provided IDs are authentic.
pub struct VmIdentity {
    instance: String,
    template: String,
    binding: Binding,
}
impl VmIdentity {
    /// The same derivation is available to trusted NEW-VM provisioning before
    /// `PidClaims::create`. Normal startup uses only open with this same identity.
    /// # Errors
    /// Empty/NUL-containing/over-budget IDs or an invalid representation budget.
    pub fn new(instance: String, template: String, maximum: usize) -> Result<Self, Error> {
        if maximum == 0 || maximum > isize::MAX as usize / 16 {
            return Err(Error::InvalidConfiguration);
        }
        if instance.is_empty()
            || template.is_empty()
            || instance.contains('\0')
            || template.contains('\0')
            || instance
                .len()
                .checked_add(template.len())
                .is_none_or(|n| n > maximum)
        {
            return Err(Error::InvalidMetadata);
        }
        // Domain and lengths make the two fields unambiguous. Only the stable
        // identity enters this digest, never a rotating credential or defaults.
        let mut hash = Sha256::new();
        hash.update(b"sandproof/guest/pid-binding/v1\0");
        for value in [&instance, &template] {
            hash.update(
                u64::try_from(value.len())
                    .map_err(|_| Error::InvalidMetadata)?
                    .to_le_bytes(),
            );
            hash.update(value.as_bytes());
        }
        let binding = Binding::new(hash.finalize().into()).map_err(Error::Storage)?;
        Ok(Self {
            instance,
            template,
            binding,
        })
    }

    #[must_use]
    pub const fn binding(&self) -> Binding {
        self.binding
    }
}

/// Each new MMDS exchange for initialization must retain the original VM IDs.
/// Credential rotation is allowed; replacing the VM/template is not. The
/// original Session still owns same-token replay and authorization semantics.
pub struct PinnedMetadata<M> {
    source: Arc<M>,
    identity: Arc<VmIdentity>,
}
impl<M: MetadataSource> MetadataSource for PinnedMetadata<M> {
    async fn fetch(&self) -> Result<Metadata, ServiceError> {
        let metadata = self.source.fetch().await?;
        if metadata.instance_id != self.identity.instance
            || metadata.template_id != self.identity.template
        {
            return Err(ServiceError::Unavailable);
        }
        Ok(metadata)
    }
}

pub struct Opened<M> {
    session: Session<PinnedMetadata<M>>,
    claims: PidClaims,
    service_session: u64,
}
impl<M> Opened<M> {
    #[must_use]
    pub fn binding(&self) -> Binding {
        self.claims.binding()
    }

    /// The durably burned internal identity for this service lifetime. Use it
    /// in both the active PID registry and command dispatcher for this startup.
    /// It is not an access token or an authorization for an old process.
    #[must_use]
    pub const fn service_session(&self) -> u64 {
        self.service_session
    }

    /// The daemon uses this Session for all handlers and moves the original
    /// locked claims into its sole `PidRegistrar`. No active PID is recovered.
    #[must_use]
    pub fn into_parts(self) -> (Session<PinnedMetadata<M>>, PidClaims) {
        (self.session, self.claims)
    }
}

/// One startup coordinator per daemon. All retries/clones share one slot;
/// cancellation cannot create an unbounded queue of blocking file opens.
pub struct Startup<M> {
    source: Arc<M>,
    limits: Limits,
    slot: Arc<Semaphore>,
}
impl<M> Clone for Startup<M> {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            limits: self.limits,
            slot: self.slot.clone(),
        }
    }
}
impl<M: MetadataSource> Startup<M> {
    /// # Errors
    /// Rejects invalid identity/deadline budgets before metadata or file work.
    pub fn new(source: M, limits: Limits) -> Result<Self, Error> {
        if limits.identity_bytes == 0
            || limits.identity_bytes > isize::MAX as usize / 16
            || limits.timeout.is_zero()
            || Instant::now().checked_add(limits.timeout).is_none()
        {
            return Err(Error::InvalidConfiguration);
        }
        Ok(Self {
            source: Arc::new(source),
            limits,
            slot: Arc::new(Semaphore::new(1)),
        })
    }

    /// Open this VM's EXISTING state under the original trusted directory FD
    /// and durably allocate a new service session before returning any state.
    /// The caller establishes directory ancestry and expected owner and bounds
    /// the runtime's blocking pool; this method does not create/open a pathname.
    /// Missing/corrupt/mismatched data is not repaired, migrated or reset.
    /// # Errors
    /// Invalid config, busy startup, metadata failure, original deadline or
    /// existing storage refusal. Timeout cannot interrupt fsync/open/read; the
    /// slot and FD remain held until queued/running work actually exits. A
    /// canceled/timed-out allocation may burn a number; it is never recycled.
    pub async fn open(
        &self,
        directory: File,
        owner: u32,
        max_pid: u32,
        initialization: InitLimits,
    ) -> Result<Opened<M>, Error> {
        initialization
            .validate()
            .map_err(|_| Error::InvalidConfiguration)?;
        if max_pid == 0 || max_pid > pid_claims::MAX_PID {
            return Err(Error::InvalidConfiguration);
        }
        let deadline = Instant::now()
            .checked_add(self.limits.timeout)
            .ok_or(Error::InvalidConfiguration)?;
        let slot = self
            .slot
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Busy)?;
        let metadata = timeout_at(deadline, self.source.fetch())
            .await
            .map_err(|_| Error::DeadlineExceeded)?
            .map_err(Error::Metadata)?;
        let identity = Arc::new(VmIdentity::new(
            metadata.instance_id,
            metadata.template_id,
            self.limits.identity_bytes,
        )?);
        if Instant::now() >= deadline {
            return Err(Error::DeadlineExceeded);
        }
        let binding = identity.binding();
        let (cancelled, _alive) = oneshot::channel::<()>();
        let task = tokio::task::spawn_blocking(move || {
            if Instant::now() >= deadline {
                return Err(Error::DeadlineExceeded);
            }
            if cancelled.is_closed() {
                return Err(Error::Cancelled);
            }
            let mut claims =
                PidClaims::open(directory, owner, binding, max_pid).map_err(Error::Storage)?;
            if Instant::now() >= deadline {
                return Err(Error::DeadlineExceeded);
            }
            if cancelled.is_closed() {
                return Err(Error::Cancelled);
            }
            let service_session = claims.allocate_session().map_err(Error::Storage)?;
            if Instant::now() >= deadline {
                return Err(Error::DeadlineExceeded);
            }
            // Keep the permit with the result too: if the waiter disappeared,
            // drop the locked claims before returning startup capacity.
            Ok((claims, slot, service_session))
        });
        // Keep the tuple intact through every remaining failure path so its
        // locked claims are destroyed before its permit, in field order.
        let opened = timeout_at(deadline, task)
            .await
            .map_err(|_| Error::DeadlineExceeded)?
            .map_err(|_| Error::WorkerUnavailable)??;
        if Instant::now() >= deadline {
            return Err(Error::DeadlineExceeded);
        }
        let session = Session::new(
            PinnedMetadata {
                source: self.source.clone(),
                identity,
            },
            initialization,
        )
        .map_err(|_| Error::InvalidConfiguration)?;
        Ok(Opened {
            session,
            claims: opened.0,
            service_session: opened.2,
        })
    }
}
