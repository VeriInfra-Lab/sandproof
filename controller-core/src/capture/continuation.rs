//! Original-VM continuation inputs. These receipts name facts already validated
//! and durably recorded by the service; digest equality is not an OS/API proof.
//! No type here authorizes physical work before the original dispatch commits.
use crate::{Command, Epoch, Generation, OperationKey, VmId};

/// Exact original Pause identity. Native identifiers (including VM zero) retain
/// their native domains; equality with the saved plan is the authority check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct OriginalContinueIdentity {
    pub key: OperationKey,
    pub epoch: Epoch,
    pub generation: Generation,
    pub vm: VmId,
}
impl OriginalContinueIdentity {
    #[must_use]
    pub const fn activation_command(self) -> Command {
        Command::ActivateVm {
            sandbox: self.key.sandbox,
            epoch: self.epoch,
            generation: self.generation,
            vm: self.vm,
        }
    }
}

/// SHA-256 bindings to the complete original Full request and its confirmed
/// durable end-fact record. The service checks original action/source/worker,
/// API completion or definite non-release, and actual writer termination.
/// Neither a worker exit, missing marker nor a current Running GET suffices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct CaptureWorkEndReceipt {
    pub work_binding: [u8; 32],
    pub end_binding: [u8; 32],
}
