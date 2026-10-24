//! Serialized durable decisions and revocations; slow physical work runs outside the lock.
mod lifecycle_continue;
mod lifecycle_restore;
mod lifecycle_source;
mod lifecycle_unload;

use std::{
    collections::{BTreeMap, BTreeSet},
    marker::PhantomData,
    sync::Arc,
};

#[cfg(feature = "contention-diagnostics")]
use super::diagnostics::SerialMutex as Mutex;
#[cfg(not(feature = "contention-diagnostics"))]
use std::sync::Mutex;

use crate::application::{
    ControllerHost, DataAccessPort, DataAuthorization, DataKind, DataLease, DataTarget,
    RelayReaper, TrackedRelayChild,
};
use crate::template_registry::{
    CandidateStore, PublicationFailure, PublicationView, TemplatePage, TemplateView, valid_alias,
    valid_candidate_id,
};
use crate::templates::{TemplateBinding, TemplateCatalog, TemplateError};
use crate::{
    AdmissionError, AdmissionLimits, ControllerJournal, CreateAdmission, CreateOptions,
    JournalError, NeverCrash, WorkerTransportError, finish_accepted_kill,
    finish_admitted_create_timed, finish_compensating_create,
};
use sandbox_controller_core as core;
use sandproof_protocol::{WorkAction, WorkReceipt};
use tokio::sync::watch;

use super::fault::{FaultContext, FaultRecord, FaultStage};
use super::listing::{InstancePage, ListSelection};
use super::models::{
    BuildCommand, BuildSettlement, BuildSettlementCommand, BuildStatus, BuildWindowClosed,
    ConnectCommand, CreateCommand, InstanceDetail, InstanceInfo, Session,
};
use super::{ApplicationError, Clock, SystemClock};
use recovery::{reclaimed_sandboxes, recover_locked, release_reclaimed_slots};

mod data_access;
mod data_credentials;
mod execution;
mod internal_audit;
mod internal_health;
mod internal_lifecycle;
mod internal_validation;
mod lifecycle;
mod lifecycle_intent;
mod lifecycle_pending;
mod maintenance;
mod physical_storage;
pub(super) mod recovery;
mod template_commands;
pub use maintenance::MaintenanceReport;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    pub receipts_recovered: usize,
    pub workflows_completed: usize,
    pub unresolved_sandboxes: usize,
    /// Sandboxes whose recovery hit a host error or rejected observation.
    /// Their current operation stays incomplete until exact evidence arrives.
    pub failed_sandboxes: usize,
    /// One bounded diagnostic sample from this pass, not a durable failure log.
    /// Does not determine whether recovery may continue.
    pub first_fault: Option<FaultRecord>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExpiryReport {
    pub completed: usize,
    /// Already expired instances whose stop or cleanup is still unresolved.
    pub unresolved_sandboxes: usize,
}

/// A request authorized against the durable world, with a local cancellation
/// signal. The signal can withdraw an existing route; it never grants access.
#[derive(Debug)]
pub struct DataPermit {
    sandbox: core::SandboxId,
    kind: DataKind,
    pub target: DataTarget,
    pub revocation: watch::Receiver<bool>,
    /// Held through the response body and exact relay-process exit.
    pub drain: DataLease,
    /// Already-locked host gate acquired under the same authority check.
    pub helper_gate: Option<std::fs::File>,
    lifecycle: Option<(
        sandproof_protocol::lifecycle::LifecycleAccessIdentity,
        sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    )>,
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
pub struct ApplicationConfig {
    pub public_ipv4_enabled: bool,
    pub limits: AdmissionLimits,
    pub disk_bytes: u64,
    pub file_policy_digest: core::PolicyDigest,
    pub cpu_count: u32,
    pub memory_mb: u32,
    pub disk_size_mb: u32,
    pub envd_version: String,
    pub domain: String,
}

struct Inner<H: ControllerHost> {
    journal: ControllerJournal,
    host: Arc<H>,
    active_sandboxes: BTreeSet<core::SandboxId>,
    active_sessions: BTreeSet<String>,
    data_revocations: BTreeMap<u64, Vec<watch::Sender<bool>>>,
    data_drains: BTreeMap<u64, DataLease>,
}

impl<H: ControllerHost> Inner<H> {
    fn revoke_data(&mut self, sandbox: core::SandboxId) {
        if let Some(signals) = self.data_revocations.get(&sandbox.0) {
            for signal in signals {
                signal.send_replace(true);
            }
        }
    }
}

/// The mutex serializes current journal decisions and route admission. Each
/// physical executor retains a separate session occupancy while using the same
/// shared Host outside this lock. `SQLite` excludes a second writer process.
pub struct ControllerApplication<H: ControllerHost> {
    inner: Mutex<Inner<H>>,
    config: ApplicationConfig,
    templates: Option<TemplateCatalog>,
    registry: Option<CandidateStore>,
    internal_build_vms: bool,
    _evidence: PhantomData<fn() -> H::Evidence>,
    clock: Arc<dyn Clock>,
}

fn token() -> Result<String, ApplicationError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| {
        let mut fault = FaultRecord::host(
            FaultStage::PutCredential,
            super::fault::FaultKind::EntropyUnavailable,
            FaultContext::default(),
        );
        fault.code = error.raw_os_error();
        fault.emit();
        ApplicationError::Busy
    })?;
    let mut result = String::with_capacity(64);
    for byte in bytes {
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 15)]));
    }
    Ok(result)
}

fn tenant_id(tenant: &str) -> Result<core::TenantId, ApplicationError> {
    let value = tenant
        .parse::<u64>()
        .map_err(|_| ApplicationError::Invalid)?;
    if value == 0 {
        return Err(ApplicationError::Invalid);
    }
    Ok(core::TenantId(value))
}

fn sandbox_id(
    journal: &ControllerJournal,
    external: &str,
) -> Result<core::SandboxId, ApplicationError> {
    let world = journal.world().map_err(map_journal)?;
    let private = journal
        .template_validation_sandboxes()
        .map_err(map_journal)?;
    for (sandbox, _) in world.sandbox_records() {
        if !private.contains(&sandbox.0)
            && journal.sandbox_id(*sandbox).map_err(map_journal)?.as_str() == external
        {
            return Ok(*sandbox);
        }
    }
    Err(ApplicationError::NotFound)
}

fn next_sandbox(journal: &ControllerJournal) -> Result<core::SandboxId, ApplicationError> {
    journal.next_sandbox_id().map_err(map_journal)
}

fn next_operation(
    journal: &ControllerJournal,
    sandbox: core::SandboxId,
) -> Result<core::OperationId, ApplicationError> {
    journal
        .next_operation_id(sandbox)
        .map_err(|error| match error {
            JournalError::IdentifierExhausted => ApplicationError::Conflict,
            other => map_journal(other),
        })
}

fn map_journal(error: JournalError) -> ApplicationError {
    map_journal_at(error, FaultStage::Journal, FaultContext::default())
}

fn map_journal_at(
    error: JournalError,
    stage: FaultStage,
    context: FaultContext,
) -> ApplicationError {
    // Expected public refusals do not become operator failure logs.
    if !matches!(
        error,
        JournalError::ControllerRejected(_)
            | JournalError::Conflict
            | JournalError::RevisionConflict
            | JournalError::IdentifierExhausted
    ) {
        FaultRecord::journal(stage, &error, context).emit();
    }
    let result = match &error {
        JournalError::ControllerRejected(core::controller::ControllerReject::Request(
            core::request::Reject::Unauthorized | core::request::Reject::SandboxNotFound,
        )) => ApplicationError::NotFound,
        JournalError::ControllerRejected(_)
        | JournalError::Conflict
        | JournalError::RevisionConflict => ApplicationError::Conflict,
        JournalError::UnresolvedAction | JournalError::CommitUncertain(_) => {
            ApplicationError::OutcomeUnknown
        }
        JournalError::OperationFailed => ApplicationError::Failed,
        JournalError::IdentifierExhausted => ApplicationError::QuotaExceeded,
        _ => ApplicationError::Busy,
    };
    drop(error);
    result
}

fn map_unknown_journal(
    error: JournalError,
    stage: FaultStage,
    context: FaultContext,
) -> ApplicationError {
    FaultRecord::journal(stage, &error, context).emit();
    drop(error);
    ApplicationError::OutcomeUnknown
}

fn map_unknown_application(
    error: ApplicationError,
    stage: FaultStage,
    context: FaultContext,
) -> ApplicationError {
    FaultRecord::application(stage, &error, context).emit();
    ApplicationError::OutcomeUnknown
}

fn map_admission(error: AdmissionError) -> ApplicationError {
    match error {
        AdmissionError::InvalidTimeout
        | AdmissionError::TimeOverflow
        | AdmissionError::InvalidClock
        | AdmissionError::MaxLifetimeExceeded
        | AdmissionError::InvalidCreateOptions => ApplicationError::Invalid,
        AdmissionError::ChangingState => ApplicationError::Conflict,
        AdmissionError::QuotaExceeded => ApplicationError::QuotaExceeded,
        AdmissionError::Unauthorized | AdmissionError::NotRunning | AdmissionError::Expired => {
            ApplicationError::NotFound
        }
        AdmissionError::Journal(error) => map_journal(error),
        _ => ApplicationError::Busy,
    }
}

impl<H: ControllerHost> ControllerApplication<H> {
    #[must_use]
    pub fn new(journal: ControllerJournal, host: H, config: ApplicationConfig) -> Self {
        Self {
            inner: Mutex::new(Inner {
                journal,
                host: Arc::new(host),
                active_sandboxes: BTreeSet::new(),
                active_sessions: BTreeSet::new(),
                data_revocations: BTreeMap::new(),
                data_drains: BTreeMap::new(),
            }),
            config,
            templates: None,
            registry: None,
            internal_build_vms: false,
            _evidence: PhantomData,
            clock: Arc::new(SystemClock),
        }
    }

    /// Inject a trusted deployment clock; never exposed through HTTP.
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Opt-in trusted prebuilt templates. The original base-only constructor stays unchanged.
    /// # Errors
    /// Rejects an invalid catalog before serving requests.
    pub fn with_templates(
        journal: ControllerJournal,
        host: H,
        config: ApplicationConfig,
        templates: TemplateCatalog,
    ) -> Result<Self, ApplicationError> {
        templates
            .validate()
            .map_err(|_| ApplicationError::Invalid)?;
        let mut port = Self::new(journal, host, config);
        port.templates = Some(templates);
        Ok(port)
    }

    /// Enable durable publication only with explicit compute accounting.
    /// # Errors
    /// Rejects a changed registry root, missing catalog, or overlapping static names.
    pub fn with_registry(mut self, store: CandidateStore) -> Result<Self, ApplicationError> {
        physical_storage::validate_registry_configuration(
            &self
                .inner
                .get_mut()
                .map_err(|_| ApplicationError::Busy)?
                .journal,
            self.internal_build_vms,
            store.config(),
        )?;
        let catalog = self.templates.as_ref().ok_or(ApplicationError::Invalid)?;
        if store.config().validation_timeout_secs > self.config.limits.max_timeout_secs
            || catalog
                .templates
                .iter()
                .any(|t| t.template_id.starts_with("tpl-"))
            || catalog.aliases.keys().any(|name| name.starts_with("tpl-"))
        {
            return Err(ApplicationError::Invalid);
        }
        self.inner
            .get_mut()
            .map_err(|_| ApplicationError::Busy)?
            .journal
            .enable_template_registry(store.config())
            .map_err(map_journal)?;
        self.registry = Some(store);
        Ok(self)
    }

    /// Enable only on a separately configured private build pool.
    #[must_use]
    pub fn with_internal_build_vms(mut self, enabled: bool) -> Self {
        self.internal_build_vms = enabled;
        self
    }

    /// Enable finite retry anchors only after the bounded maintenance store is configured.
    /// # Errors
    /// Rejects public pools, missing maintenance, or changed persistent settings.
    pub fn with_build_retry_windows(
        mut self,
        config: crate::BuildRetryConfig,
    ) -> Result<Self, ApplicationError> {
        if !self.internal_build_vms || self.registry.is_some() {
            return Err(ApplicationError::Invalid);
        }
        physical_storage::validate_builder_configuration(
            &self
                .inner
                .get_mut()
                .map_err(|_| ApplicationError::Busy)?
                .journal,
        )?;
        self.inner
            .get_mut()
            .map_err(|_| ApplicationError::Busy)?
            .journal
            .enable_build_retry_windows(config)
            .map_err(map_journal)?;
        Ok(self)
    }
}
