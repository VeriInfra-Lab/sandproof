//! Serial management adapter from E2B HTTP requests to the durable reducer.
//! It does not impersonate the host: all worker receipts come from `HostAdapter`.

use std::{
    marker::PhantomData,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use sandbox_controller_core as core;
use sandproof_controller_service::{
    AdmissionError, AdmissionLimits, ControllerJournal, HostEvidence, JournalError, NeverCrash,
    WorkerTransportError, finish_accepted_kill_v1, finish_admitted_create_v1, kill_v1,
};
use sandproof_protocol::{WorkAction, WorkReceipt};
use serde::{Serialize, de::DeserializeOwned};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    ConnectRequest, ControllerPort, CreateRequest, PortError, SandboxInfo, SandboxPage,
    SandboxSession,
};

pub trait HostAdapter: Send + 'static {
    type Evidence: HostEvidence + Serialize + DeserializeOwned + PartialEq + Send;

    /// # Errors
    /// Rejects an unavailable physical disk observation.
    fn available_disk_bytes(&self) -> Result<u64, PortError>;

    /// # Errors
    /// A host credential creation failure leaves the admitted create pending.
    fn put_secret(&mut self, sandbox: core::SandboxId, token: &str) -> Result<(), PortError>;
    /// # Errors
    /// Missing credentials cannot be replaced by a new token on connect.
    fn token_for(&self, sandbox: core::SandboxId) -> Result<String, PortError>;
    /// # Errors
    /// Transport loss must remain unknown, never an implicit retry.
    fn execute(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<WorkReceipt<Self::Evidence>, WorkerTransportError>;

    /// Observe an already dispatched action without performing it again.
    /// `None` means the physical outcome is still unknown and must stay so.
    fn observe(&self, action: &WorkAction<core::EffectId>) -> Option<WorkReceipt<Self::Evidence>>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    pub receipts_recovered: usize,
    pub workflows_completed: usize,
    pub unresolved_sandboxes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataKind {
    Exec,
    FileRead,
    FileWrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataTarget {
    pub vm: core::VmId,
    pub generation: core::Generation,
}

fn same_secret(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (&a, &b) in left.iter().zip(right) {
        difference |= a ^ b;
    }
    difference == 0
}

#[derive(Clone, Debug)]
pub struct ManagedConfig {
    pub limits: AdmissionLimits,
    pub disk_bytes: u64,
    pub file_policy_digest: core::PolicyDigest,
    pub default_timeout_secs: u64,
    pub cpu_count: u32,
    pub memory_mb: u32,
    pub disk_size_mb: u32,
    pub envd_version: String,
    pub domain: String,
}

struct Inner<H: HostAdapter> {
    journal: ControllerJournal,
    host: H,
}

/// The mutex gives one process a serial operation boundary across journal
/// decisions, host effects, and public responses. `SQLite`'s process lease
/// excludes another controller process.
pub struct ManagedPort<H: HostAdapter> {
    inner: Mutex<Inner<H>>,
    config: ManagedConfig,
    _evidence: PhantomData<fn() -> H::Evidence>,
}

fn now_ms() -> Result<i64, PortError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| PortError::Busy)?;
    i64::try_from(duration.as_millis()).map_err(|_| PortError::Busy)
}

fn rfc3339(milliseconds: i64) -> Result<String, PortError> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(milliseconds) * 1_000_000)
        .map_err(|_| PortError::Busy)?
        .format(&Rfc3339)
        .map_err(|_| PortError::Busy)
}

fn token() -> Result<String, PortError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| PortError::Busy)?;
    let mut result = String::with_capacity(64);
    for byte in bytes {
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 15)]));
    }
    Ok(result)
}

fn tenant_id(tenant: &str) -> Result<core::TenantId, PortError> {
    let value = tenant.parse::<u64>().map_err(|_| PortError::Invalid)?;
    if value == 0 {
        return Err(PortError::Invalid);
    }
    Ok(core::TenantId(value))
}

fn sandbox_id(journal: &ControllerJournal, external: &str) -> Result<core::SandboxId, PortError> {
    let world = journal.world().map_err(map_journal)?;
    for (sandbox, _) in world.sandbox_records() {
        if journal.sandbox_id(*sandbox).map_err(map_journal)?.as_str() == external {
            return Ok(*sandbox);
        }
    }
    Err(PortError::NotFound)
}

fn next_sandbox(journal: &ControllerJournal) -> Result<core::SandboxId, PortError> {
    let world = journal.world().map_err(map_journal)?;
    let largest = world
        .sandbox_records()
        .iter()
        .map(|(id, _)| id.0)
        .max()
        .unwrap_or(0);
    Ok(core::SandboxId(
        largest.checked_add(1).ok_or(PortError::QuotaExceeded)?,
    ))
}

fn next_operation(
    world: &core::World,
    sandbox: core::SandboxId,
) -> Result<core::OperationId, PortError> {
    let largest = world
        .operation_records()
        .iter()
        .filter(|(key, _)| key.sandbox == sandbox)
        .map(|(key, _)| key.operation.0)
        .max()
        .unwrap_or(0);
    Ok(core::OperationId(
        largest.checked_add(1).ok_or(PortError::Conflict)?,
    ))
}

fn map_journal(error: JournalError) -> PortError {
    let result = match &error {
        JournalError::ControllerRejected(core::controller::ControllerReject::Request(
            core::request::Reject::Unauthorized | core::request::Reject::SandboxNotFound,
        )) => PortError::NotFound,
        JournalError::ControllerRejected(_)
        | JournalError::Conflict
        | JournalError::RevisionConflict => PortError::Conflict,
        JournalError::UnresolvedAction | JournalError::CommitUncertain(_) => {
            PortError::OutcomeUnknown
        }
        _ => PortError::Busy,
    };
    drop(error);
    result
}

fn map_admission(error: AdmissionError) -> PortError {
    match error {
        AdmissionError::InvalidTimeout
        | AdmissionError::TimeOverflow
        | AdmissionError::InvalidClock => PortError::Invalid,
        AdmissionError::QuotaExceeded => PortError::QuotaExceeded,
        AdmissionError::Unauthorized | AdmissionError::NotRunning | AdmissionError::Expired => {
            PortError::NotFound
        }
        AdmissionError::Journal(error) => map_journal(error),
        _ => PortError::Busy,
    }
}

impl<H: HostAdapter> ManagedPort<H> {
    #[must_use]
    pub fn new(journal: ControllerJournal, host: H, config: ManagedConfig) -> Self {
        Self {
            inner: Mutex::new(Inner { journal, host }),
            config,
            _evidence: PhantomData,
        }
    }

    /// Reconcile exact host observations before resuming accepted workflows.
    /// It never re-executes a dispatched action. A sandbox with inconclusive
    /// evidence remains unavailable, while other sandboxes can still recover.
    /// Call this after opening the durable journal and before serving requests.
    /// # Errors
    /// Invalid evidence or a corrupt journal aborts startup recovery.
    pub fn recover_on_startup(&self) -> Result<RecoveryReport, PortError> {
        let mut inner = self.inner.lock().map_err(|_| PortError::Busy)?;
        let Inner { journal, host } = &mut *inner;
        let receipts_recovered = journal
            .reconcile_uncertain(&mut |action| host.observe(action))
            .map_err(map_journal)?;
        let pending = journal.world().map_err(map_journal)?;
        let mut report = RecoveryReport {
            receipts_recovered,
            ..RecoveryReport::default()
        };
        for (sandbox, record) in pending.sandbox_records() {
            let Some(operation) = record.current_operation else {
                continue;
            };
            let resume = match (record.phase, operation.kind) {
                (core::Phase::Starting, core::OperationKind::Create) => {
                    finish_admitted_create_v1(journal, *sandbox, operation.id, &mut |action| {
                        host.execute(action)
                    })
                    .map(|_| ())
                }
                (core::Phase::Revoking | core::Phase::Destroyed, core::OperationKind::Kill) => {
                    finish_accepted_kill_v1(journal, *sandbox, operation.id, &mut |action| {
                        host.execute(action)
                    })
                    .map(|_| ())
                }
                _ => continue,
            };
            match resume {
                Ok(()) => report.workflows_completed += 1,
                Err(JournalError::UnresolvedAction) => report.unresolved_sandboxes += 1,
                Err(error) => return Err(map_journal(error)),
            }
        }
        Ok(report)
    }

    /// Resolve an envd bearer credential to the exact current VM and check
    /// the same effective core authority used by management operations.
    /// The guest token is distinct from the management API key and is bound
    /// to exactly one sandbox's owner. No request may choose its own tenant.
    ///
    /// # Errors
    /// Rejects unknown, expired, revoked or non-running credentials.
    pub fn authorize_data(&self, supplied: &str, kind: DataKind) -> Result<DataTarget, PortError> {
        if supplied.len() != 64 || !supplied.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(PortError::NotFound);
        }
        let inner = self.inner.lock().map_err(|_| PortError::Busy)?;
        let world = inner.journal.world().map_err(map_journal)?;
        let mut matched = None;
        for (sandbox, record) in world.sandbox_records() {
            if record.phase != core::Phase::Running {
                continue;
            }
            let token = inner.host.token_for(*sandbox)?;
            if same_secret(supplied.as_bytes(), token.as_bytes())
                && matched.replace((*sandbox, record)).is_some()
            {
                return Err(PortError::Busy);
            }
        }
        let (sandbox, record) = matched.ok_or(PortError::NotFound)?;
        let actor = core::Actor::Tenant(record.owner);
        inner
            .journal
            .live_lifetime(&actor, sandbox, now_ms()?)
            .map_err(map_admission)?;
        let vm = record.current_vm.ok_or(PortError::NotFound)?;
        let input = match kind {
            DataKind::Exec => core::AccessInput::Exec(actor, sandbox, vm),
            DataKind::FileRead => core::AccessInput::FileRead(actor, sandbox, record.generation),
            DataKind::FileWrite => core::AccessInput::FileWrite(actor, sandbox, record.generation),
        };
        inner
            .journal
            .check(core::ControllerInput::Access(input))
            .map_err(map_journal)?;
        Ok(DataTarget {
            vm,
            generation: record.generation,
        })
    }

    /// Execute due expiries against the same journal/host lock as client
    /// calls. A renewed deadline is rechecked by `accept_expiry` before any
    /// revocation is recorded.
    ///
    /// # Errors
    /// Stops at the first uncertain host outcome; no subsequent VM is
    /// silently reported as destroyed.
    pub fn expire_due(&self) -> Result<usize, PortError> {
        let mut inner = self.inner.lock().map_err(|_| PortError::Busy)?;
        let now = now_ms()?;
        let due = inner.journal.due_lifetimes(now).map_err(map_admission)?;
        let mut completed = 0;
        for sandbox in due {
            let operation = next_operation(&inner.journal.world().map_err(map_journal)?, sandbox)?;
            inner
                .journal
                .accept_expiry(sandbox, operation, now, &mut NeverCrash)
                .map_err(map_admission)?;
            let Inner { journal, host } = &mut *inner;
            finish_accepted_kill_v1(journal, sandbox, operation, &mut |action| {
                host.execute(action)
            })
            .map_err(map_journal)?;
            completed += 1;
        }
        Ok(completed)
    }

    fn session(
        &self,
        inner: &Inner<H>,
        tenant: core::TenantId,
        sandbox: core::SandboxId,
    ) -> Result<SandboxSession, PortError> {
        let actor = core::Actor::Tenant(tenant);
        inner
            .journal
            .live_lifetime(&actor, sandbox, now_ms()?)
            .map_err(map_admission)?;
        let world = inner.journal.world().map_err(map_journal)?;
        let vm = world
            .sandbox_record(sandbox)
            .and_then(|record| record.current_vm)
            .ok_or(PortError::Busy)?;
        inner
            .journal
            .check(core::ControllerInput::Access(core::AccessInput::Exec(
                actor, sandbox, vm,
            )))
            .map_err(map_journal)?;
        Ok(SandboxSession {
            template_id: "base".into(),
            sandbox_id: inner
                .journal
                .sandbox_id(sandbox)
                .map_err(map_journal)?
                .as_str()
                .to_owned(),
            client_id: format!("tenant-{}", tenant.0),
            envd_version: self.config.envd_version.clone(),
            envd_access_token: inner.host.token_for(sandbox)?,
            domain: self.config.domain.clone(),
        })
    }

    fn info(
        &self,
        inner: &Inner<H>,
        tenant: core::TenantId,
        sandbox: core::SandboxId,
    ) -> Result<SandboxInfo, PortError> {
        let lifetime = inner
            .journal
            .live_lifetime(&core::Actor::Tenant(tenant), sandbox, now_ms()?)
            .map_err(map_admission)?;
        let started_at = lifetime.started_at_ms.ok_or(PortError::Busy)?;
        Ok(SandboxInfo {
            template_id: "base".into(),
            sandbox_id: inner
                .journal
                .sandbox_id(sandbox)
                .map_err(map_journal)?
                .as_str()
                .to_owned(),
            client_id: format!("tenant-{}", tenant.0),
            started_at: rfc3339(started_at)?,
            end_at: rfc3339(lifetime.deadline_ms)?,
            envd_version: self.config.envd_version.clone(),
            cpu_count: self.config.cpu_count,
            memory_mb: self.config.memory_mb,
            disk_size_mb: self.config.disk_size_mb,
            state: "running".into(),
        })
    }
}

impl<H: HostAdapter> ControllerPort for ManagedPort<H> {
    fn create(&self, tenant: &str, request: CreateRequest) -> Result<SandboxSession, PortError> {
        let tenant = tenant_id(tenant)?;
        if request.template_id != "base"
            || request.allow_internet_access != Some(false)
            || request
                .metadata
                .as_ref()
                .is_some_and(|metadata| !metadata.is_empty())
            || request
                .env_vars
                .as_ref()
                .is_some_and(|env_vars| !env_vars.is_empty())
        {
            return Err(PortError::Invalid);
        }
        let timeout = request.timeout.unwrap_or(
            i64::try_from(self.config.default_timeout_secs).map_err(|_| PortError::Invalid)?,
        );
        let timeout = u64::try_from(timeout).map_err(|_| PortError::Invalid)?;
        let mut inner = self.inner.lock().map_err(|_| PortError::Busy)?;
        let sandbox = next_sandbox(&inner.journal)?;
        let physical_needed = self
            .config
            .disk_bytes
            .checked_add(self.config.limits.cleanup_reserve_bytes)
            .ok_or(PortError::QuotaExceeded)?;
        if inner.host.available_disk_bytes()? < physical_needed {
            return Err(PortError::QuotaExceeded);
        }
        let actor = core::Actor::Tenant(tenant);
        inner
            .journal
            .admit_create(
                core::CreateRequest {
                    actor,
                    owner: tenant,
                    sandbox,
                    workspace: core::FsId(sandbox.0),
                    file_policy: core::FilePolicyRef {
                        digest: self.config.file_policy_digest,
                    },
                    policy: core::NetworkPolicy::DenyAll,
                    operation: core::OperationId(1),
                },
                now_ms()?,
                timeout,
                self.config.disk_bytes,
                self.config.limits,
                &mut NeverCrash,
            )
            .map_err(map_admission)?;
        let secret = token()?;
        inner.host.put_secret(sandbox, &secret)?;
        let Inner { journal, host } = &mut *inner;
        finish_admitted_create_v1(journal, sandbox, core::OperationId(1), &mut |action| {
            host.execute(action)
        })
        .map_err(map_journal)?;
        self.session(&inner, tenant, sandbox)
    }

    fn connect(
        &self,
        tenant: &str,
        external: &str,
        request: ConnectRequest,
    ) -> Result<SandboxSession, PortError> {
        let tenant = tenant_id(tenant)?;
        let mut inner = self.inner.lock().map_err(|_| PortError::Busy)?;
        let sandbox = sandbox_id(&inner.journal, external)?;
        let timeout = request
            .timeout
            .map(|value| u64::try_from(value).map_err(|_| PortError::Invalid))
            .transpose()?;
        inner
            .journal
            .connect_lifetime(
                &core::Actor::Tenant(tenant),
                sandbox,
                now_ms()?,
                timeout,
                self.config.limits,
            )
            .map_err(map_admission)?;
        self.session(&inner, tenant, sandbox)
    }

    fn get(&self, tenant: &str, external: &str) -> Result<SandboxInfo, PortError> {
        let tenant = tenant_id(tenant)?;
        let inner = self.inner.lock().map_err(|_| PortError::Busy)?;
        let sandbox = sandbox_id(&inner.journal, external)?;
        self.info(&inner, tenant, sandbox)
    }

    fn list(
        &self,
        tenant: &str,
        limit: Option<u32>,
        next_token: Option<String>,
    ) -> Result<SandboxPage, PortError> {
        let tenant = tenant_id(tenant)?;
        let inner = self.inner.lock().map_err(|_| PortError::Busy)?;
        let world = inner.journal.world().map_err(map_journal)?;
        let start = next_token
            .map(|token| token.parse::<u64>().map_err(|_| PortError::Invalid))
            .transpose()?
            .unwrap_or(0);
        let limit = usize::try_from(limit.unwrap_or(100)).map_err(|_| PortError::Invalid)?;
        if !(1..=100).contains(&limit) {
            return Err(PortError::Invalid);
        }
        let mut matching = Vec::new();
        for (sandbox, record) in world.sandbox_records() {
            if sandbox.0 <= start || record.owner != tenant || record.phase != core::Phase::Running
            {
                continue;
            }
            match self.info(&inner, tenant, *sandbox) {
                Ok(info) => matching.push((*sandbox, info)),
                Err(PortError::NotFound) => {} // expired after last cleanup tick
                Err(error) => return Err(error),
            }
        }
        matching.sort_by_key(|(sandbox, _)| sandbox.0);
        let next = if matching.len() > limit {
            Some(matching[limit - 1].0.0.to_string())
        } else {
            None
        };
        matching.truncate(limit);
        Ok(SandboxPage {
            items: matching.into_iter().map(|(_, info)| info).collect(),
            next_token: next,
        })
    }

    fn kill(&self, tenant: &str, external: &str) -> Result<(), PortError> {
        let tenant = tenant_id(tenant)?;
        let mut inner = self.inner.lock().map_err(|_| PortError::Busy)?;
        let sandbox = sandbox_id(&inner.journal, external)?;
        let world = inner.journal.world().map_err(map_journal)?;
        let record = world.sandbox_record(sandbox).ok_or(PortError::NotFound)?;
        if record.owner != tenant || record.phase == core::Phase::Destroyed {
            return Err(PortError::NotFound);
        }
        let operation = next_operation(&world, sandbox)?;
        let Inner { journal, host } = &mut *inner;
        kill_v1(
            journal,
            core::KillRequest {
                actor: core::Actor::Tenant(tenant),
                sandbox,
                operation,
            },
            &mut |action| host.execute(action),
        )
        .map_err(map_journal)?;
        Ok(())
    }

    fn set_timeout(&self, tenant: &str, external: &str, seconds: i64) -> Result<(), PortError> {
        let tenant = tenant_id(tenant)?;
        let mut inner = self.inner.lock().map_err(|_| PortError::Busy)?;
        let sandbox = sandbox_id(&inner.journal, external)?;
        let seconds = u64::try_from(seconds).map_err(|_| PortError::Invalid)?;
        inner
            .journal
            .set_lifetime(
                &core::Actor::Tenant(tenant),
                sandbox,
                now_ms()?,
                seconds,
                self.config.limits,
            )
            .map_err(map_admission)?;
        Ok(())
    }
}
