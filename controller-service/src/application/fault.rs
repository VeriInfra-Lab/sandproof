//! Bounded, payload-free operator diagnostics. Never an authorization input.
//! Public transport errors remain the seven `ApplicationError` categories.
use super::ApplicationError;
use crate::{HostSlotContext, JournalError};
use sandbox_controller_core as core;
use sandproof_protocol::{ContractError, WorkAction};
use std::{fmt, io};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultScope {
    Instance,
    SharedStorage,
    SharedAuthority,
    Host,
    Unclassified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultStage {
    Journal,
    Execute,
    Observe,
    Reclaim,
    BindSlot,
    ReleaseSlot,
    DataGate,
    CenterDataAuthorization,
    CenterNodeFreshness,
    HostDataAuthorization,
    Relay,
    DiskCapacity,
    PhysicalStorage,
    PublicIpv4,
    PutCredential,
    ReadCredential,
    CheckCredential,
    TemplateValidation,
    RecoverReceipt,
    RecoverWorkflow,
    CredentialRegistration,
    CreateWorkflow,
    KillWorkflow,
    Session,
    AcceptedState,
    MaintenanceBefore,
    Recovery,
    Expiry,
    MaintenanceAfter,
    Join,
}

/// Closed labels; never retain an external error's Display/Debug payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultKind {
    Io,
    Sqlite,
    CommitUncertain,
    JsonIo,
    JsonSyntax,
    JsonData,
    JsonEof,
    InvalidIdentifier,
    UnsupportedVersion,
    MismatchedIdentity,
    MismatchedObservation,
    Conflict,
    IdentifierExhausted,
    MaintenanceCapacity,
    RevisionConflict,
    ReceiptBeforeDispatch,
    ConflictingReceipt,
    UnsupportedStore,
    CorruptStore,
    CredentialIndexPoisoned,
    InjectedCrash,
    ControllerRejected,
    ControllerBusy,
    UnsupportedInput,
    UnresolvedAction,
    OperationFailed,
    InvalidHostEvidence,
    ObsoleteReceipt,
    KvmUnavailable,
    InvalidConfiguration,
    InvalidId,
    AlreadyClaimed,
    MissingRecord,
    IdentityMismatch,
    TimedOut,
    HostCommand,
    FirecrackerApi,
    Protocol,
    RouteCapacity,
    HostInvariant,
    CredentialUnavailable,
    CredentialMismatch,
    BindingMismatch,
    TaskPanicked,
    TaskCancelled,
    EntropyUnavailable,
    Application(ApplicationError),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FaultContext {
    pub sandbox: Option<u64>,
    pub operation: Option<u64>,
    pub generation: Option<u64>,
    pub attempt: Option<u64>,
    pub vm: Option<u64>,
    pub epoch: Option<u64>,
    pub command: Option<WorkKind>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkKind {
    OpenFence,
    TerminalFence,
    PrepareVm,
    ConfigureNetwork,
    ActivateVm,
    FreezeVm,
    ApplyFilePolicy,
    SealSnapshot,
    CommitSnapshot,
    StopVm,
    AbortPrepare,
    CleanupVm,
    CleanupSlot,
}

impl FaultContext {
    #[must_use]
    pub const fn instance(sandbox: core::SandboxId, operation: Option<core::OperationId>) -> Self {
        Self {
            sandbox: Some(sandbox.0),
            operation: match operation {
                Some(id) => Some(id.0),
                None => None,
            },
            generation: None,
            attempt: None,
            vm: None,
            epoch: None,
            command: None,
        }
    }

    /// Numeric correlation only, not validation of a worker's identity. Do not
    /// format the command/policy, opaque IDs, namespace, owner or resource path.
    #[must_use]
    pub fn action(action: &WorkAction<core::EffectId>) -> Self {
        let command = action.command.command();
        let kind = match command {
            core::Command::InstallFence {
                terminal: false, ..
            } => WorkKind::OpenFence,
            core::Command::InstallFence { terminal: true, .. } => WorkKind::TerminalFence,
            core::Command::PrepareVm { .. } => WorkKind::PrepareVm,
            core::Command::ConfigureNetwork { .. } => WorkKind::ConfigureNetwork,
            core::Command::ActivateVm { .. } => WorkKind::ActivateVm,
            core::Command::FreezeVm { .. } => WorkKind::FreezeVm,
            core::Command::ApplyFilePolicy { .. } => WorkKind::ApplyFilePolicy,
            core::Command::SealSnapshot { .. } => WorkKind::SealSnapshot,
            core::Command::CommitSnapshot { .. } => WorkKind::CommitSnapshot,
            core::Command::StopVm { .. } => WorkKind::StopVm,
            core::Command::AbortPrepare { .. } => WorkKind::AbortPrepare,
            core::Command::CleanupVm { .. } => WorkKind::CleanupVm,
            core::Command::CleanupSlot { .. } => WorkKind::CleanupSlot,
        };
        let vm = match command {
            core::Command::PrepareVm { vm, .. }
            | core::Command::AbortPrepare { vm, .. }
            | core::Command::ActivateVm { vm, .. }
            | core::Command::FreezeVm { vm, .. }
            | core::Command::ConfigureNetwork { vm, .. }
            | core::Command::StopVm { vm, .. }
            | core::Command::SealSnapshot { vm, .. }
            | core::Command::CleanupVm { vm, .. } => Some(vm.0),
            _ => None,
        };
        Self {
            sandbox: Some(command.sandbox().0),
            // Current production operation IDs end in the numeric operation.
            // Unknown encodings stay absent, never copied into a diagnostic.
            operation: operation_number(action.identity.operation.as_str(), command.sandbox()),
            generation: Some(action.identity.generation),
            attempt: Some(action.command.attempt),
            vm,
            epoch: command.epoch(),
            command: Some(kind),
        }
    }

    #[must_use]
    pub const fn slot(context: &HostSlotContext) -> Self {
        Self {
            sandbox: Some(context.binding.sandbox.0),
            generation: Some(context.binding.generation),
            vm: Some(context.binding.vm.0),
            operation: None,
            attempt: None,
            epoch: None,
            command: None,
        }
    }
}

fn operation_number(encoded: &str, sandbox: core::SandboxId) -> Option<u64> {
    let mut fields = encoded.rsplitn(3, '-');
    let operation = fields.next()?.parse().ok()?;
    if fields.next()?.parse::<u64>().ok()? != sandbox.0 {
        return None;
    }
    let namespace = fields.next()?.strip_prefix("op-")?;
    (namespace.len() == 16
        && namespace
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then_some(operation)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaultRecord {
    pub scope: FaultScope,
    pub stage: FaultStage,
    pub kind: FaultKind,
    /// OS errno, `SQLite` extended code or HTTP status; never an error message.
    pub code: Option<i32>,
    pub io_kind: Option<io::ErrorKind>,
    pub context: FaultContext,
}

impl FaultRecord {
    #[must_use]
    pub const fn host(stage: FaultStage, kind: FaultKind, context: FaultContext) -> Self {
        Self {
            scope: if context.sandbox.is_some() {
                FaultScope::Instance
            } else {
                FaultScope::Host
            },
            stage,
            kind,
            code: None,
            io_kind: None,
            context,
        }
    }

    #[must_use]
    pub fn application(stage: FaultStage, error: &ApplicationError, context: FaultContext) -> Self {
        let mut record = Self::host(stage, FaultKind::Application(*error), context);
        if context.sandbox.is_none() {
            record.scope = FaultScope::Unclassified;
        }
        record
    }

    #[must_use]
    pub fn journal(stage: FaultStage, error: &JournalError, context: FaultContext) -> Self {
        let (kind, code) = match error {
            JournalError::Io(error) => (FaultKind::Io, error.raw_os_error()),
            #[cfg(feature = "durable")]
            JournalError::Sqlite(error) => (FaultKind::Sqlite, sqlite_code(error)),
            #[cfg(feature = "durable")]
            JournalError::CommitUncertain(error) => {
                (FaultKind::CommitUncertain, sqlite_code(error))
            }
            JournalError::Json(error) => (json_kind(error), None),
            JournalError::Contract(error) => (
                match error {
                    ContractError::InvalidIdentifier => FaultKind::InvalidIdentifier,
                    ContractError::UnsupportedVersion(_) => FaultKind::UnsupportedVersion,
                    ContractError::MismatchedIdentity => FaultKind::MismatchedIdentity,
                    ContractError::MismatchedObservation => FaultKind::MismatchedObservation,
                },
                None,
            ),
            JournalError::Conflict => (FaultKind::Conflict, None),
            JournalError::IdentifierExhausted => (FaultKind::IdentifierExhausted, None),
            JournalError::MaintenanceCapacity => (FaultKind::MaintenanceCapacity, None),
            JournalError::CaptureMaterialQuotaExceeded => (FaultKind::MaintenanceCapacity, None),
            JournalError::RevisionConflict => (FaultKind::RevisionConflict, None),
            JournalError::ReceiptBeforeDispatch => (FaultKind::ReceiptBeforeDispatch, None),
            JournalError::ConflictingReceipt => (FaultKind::ConflictingReceipt, None),
            JournalError::UnsupportedStore => (FaultKind::UnsupportedStore, None),
            JournalError::CorruptStore => (FaultKind::CorruptStore, None),
            JournalError::CredentialIndexPoisoned => (FaultKind::CredentialIndexPoisoned, None),
            JournalError::InjectedCrash(_) => (FaultKind::InjectedCrash, None),
            JournalError::ControllerRejected(_) => (FaultKind::ControllerRejected, None),
            JournalError::ControllerBusy => (FaultKind::ControllerBusy, None),
            JournalError::UnsupportedInput => (FaultKind::UnsupportedInput, None),
            JournalError::UnresolvedAction => (FaultKind::UnresolvedAction, None),
            JournalError::OperationFailed => (FaultKind::OperationFailed, None),
            JournalError::InvalidHostEvidence => (FaultKind::InvalidHostEvidence, None),
            JournalError::ObsoleteReceipt => (FaultKind::ObsoleteReceipt, None),
        };
        let scope = match error {
            JournalError::CredentialIndexPoisoned => FaultScope::SharedAuthority,
            JournalError::Io(_)
            | JournalError::Json(_)
            | JournalError::UnsupportedStore
            | JournalError::CorruptStore => FaultScope::SharedStorage,
            #[cfg(feature = "durable")]
            JournalError::Sqlite(_) | JournalError::CommitUncertain(_) => FaultScope::SharedStorage,
            _ if error.is_local_recovery_failure() && context.sandbox.is_some() => {
                FaultScope::Instance
            }
            _ => FaultScope::Unclassified,
        };
        let io_kind = match error {
            JournalError::Io(error) => Some(error.kind()),
            _ => None,
        };
        Self {
            scope,
            stage,
            kind,
            code,
            io_kind,
            context,
        }
    }

    /// Existing stderr operator channel only: no store, queue, or control input.
    /// A broken diagnostic sink must not alter the operation's result.
    pub fn emit(self) {
        use io::Write;
        let _ = writeln!(io::stderr().lock(), "{self}");
    }
}

impl fmt::Display for FaultRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "controller_fault scope={:?} stage={:?} kind={:?} code={:?} io_kind={:?} sandbox={:?} operation={:?} generation={:?} attempt={:?} vm={:?} epoch={:?} command={:?}",
            self.scope,
            self.stage,
            self.kind,
            self.code,
            self.io_kind,
            self.context.sandbox,
            self.context.operation,
            self.context.generation,
            self.context.attempt,
            self.context.vm,
            self.context.epoch,
            self.context.command
        )
    }
}

#[must_use]
pub fn json_kind(error: &serde_json::Error) -> FaultKind {
    match error.classify() {
        serde_json::error::Category::Io => FaultKind::JsonIo,
        serde_json::error::Category::Syntax => FaultKind::JsonSyntax,
        serde_json::error::Category::Data => FaultKind::JsonData,
        serde_json::error::Category::Eof => FaultKind::JsonEof,
    }
}

#[cfg(feature = "durable")]
fn sqlite_code(error: &rusqlite::Error) -> Option<i32> {
    match error {
        rusqlite::Error::SqliteFailure(code, _) => Some(code.extended_code),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_does_not_confuse_local_refusal_with_shared_corruption() {
        let context = FaultContext::instance(core::SandboxId(7), Some(core::OperationId(3)));
        for error in [
            JournalError::InvalidHostEvidence,
            JournalError::Contract(ContractError::MismatchedIdentity),
            JournalError::UnresolvedAction,
        ] {
            assert_eq!(
                FaultRecord::journal(FaultStage::RecoverReceipt, &error, context).scope,
                FaultScope::Instance
            );
        }
        for error in [JournalError::CorruptStore, JournalError::UnsupportedStore] {
            assert_eq!(
                FaultRecord::journal(FaultStage::Recovery, &error, context).scope,
                FaultScope::SharedStorage
            );
        }
        assert_eq!(
            FaultRecord::journal(
                FaultStage::Recovery,
                &JournalError::CredentialIndexPoisoned,
                context
            )
            .scope,
            FaultScope::SharedAuthority
        );
        assert_eq!(
            FaultRecord::journal(FaultStage::Journal, &JournalError::Conflict, context).scope,
            FaultScope::Unclassified
        );
    }

    #[test]
    fn journal_payloads_and_sql_messages_are_never_formatted() {
        let secret = "token-private /root/private user-content\nforged-log";
        let errors = vec![
            JournalError::Io(io::Error::other(secret)),
            JournalError::Json(serde_json::Error::io(io::Error::other(secret))),
        ];
        #[cfg(feature = "durable")]
        let errors = {
            let mut extended = errors;
            extended.extend([
                JournalError::Sqlite(rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(11),
                    Some(secret.into()),
                )),
                JournalError::CommitUncertain(rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(5),
                    Some(secret.into()),
                )),
            ]);
            extended
        };
        for error in errors {
            let record = FaultRecord::journal(FaultStage::Journal, &error, FaultContext::default());
            let text = record.to_string();
            assert!(!text.contains(secret));
            assert!(!text.contains("private"));
            assert!(!text.contains('\n'));
            assert!(text.len() < 320);
            assert_eq!(record.scope, FaultScope::SharedStorage);
        }
    }

    #[test]
    fn action_correlation_is_numeric_and_omits_opaque_names_and_policies() {
        use sandproof_protocol::{
            ActionIdentity, LogicalActionId, OperationId, PROTOCOL_VERSION, ResourceId,
            ResourceKind, ResourceRef, SandboxId, TenantId,
        };
        let action = WorkAction {
            identity: ActionIdentity {
                protocol_version: PROTOCOL_VERSION,
                owner: TenantId::try_from("private-owner").unwrap(),
                sandbox: SandboxId::try_from("private-sandbox").unwrap(),
                operation: OperationId::try_from("op-0123456789abcdef-7-4").unwrap(),
                epoch: 3,
                generation: 2,
                logical_action: LogicalActionId::try_from("private-action").unwrap(),
                attempt: 1,
                resource: ResourceRef {
                    kind: ResourceKind::Vm,
                    id: ResourceId::try_from("private-vm").unwrap(),
                },
            },
            command: core::EffectId {
                logical: core::LogicalEffectId {
                    command: core::Command::StopVm {
                        sandbox: core::SandboxId(7),
                        epoch: 3,
                        generation: 2,
                        vm: core::VmId(8),
                    },
                },
                attempt: 1,
            },
        };
        let context = FaultContext::action(&action);
        assert_eq!(
            context,
            FaultContext {
                sandbox: Some(7),
                operation: Some(4),
                generation: Some(2),
                attempt: Some(1),
                vm: Some(8),
                epoch: Some(3),
                command: Some(WorkKind::StopVm)
            }
        );
        assert!(
            !FaultRecord::host(FaultStage::Execute, FaultKind::TimedOut, context)
                .to_string()
                .contains("private")
        );
        for encoded in [
            "private-7",
            "op-private-7-4",
            "op-0123456789abcdef-8-4",
            "op-0123456789abcdef-7-private",
        ] {
            assert_eq!(operation_number(encoded, core::SandboxId(7)), None);
        }
    }

    #[test]
    fn largest_numeric_context_and_sample_remain_bounded() {
        let record = FaultRecord {
            scope: FaultScope::SharedAuthority,
            stage: FaultStage::CredentialRegistration,
            kind: FaultKind::MismatchedObservation,
            code: Some(i32::MIN),
            io_kind: Some(io::ErrorKind::PermissionDenied),
            context: FaultContext {
                sandbox: Some(u64::MAX),
                operation: Some(u64::MAX),
                generation: Some(u64::MAX),
                attempt: Some(u64::MAX),
                vm: Some(u64::MAX),
                epoch: Some(u64::MAX),
                command: Some(WorkKind::ConfigureNetwork),
            },
        };
        assert!(record.to_string().len() < 640);
        assert!(std::mem::size_of::<Option<FaultRecord>>() < 256);
    }
}
