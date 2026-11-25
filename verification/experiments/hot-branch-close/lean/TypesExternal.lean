import SandproofStage1Request.Types

open Aeneas Aeneas.Std

namespace SandproofBranchBatch

abbrev core.num.niche_types.NonZeroU16Inner := Unit
abbrev core.num.nonzero.NonZero (_ : Type) (_ : Type) := {value : U16 // value.val ≠ 0}

abbrev sandproof_protocol.internal.PositiveCounter := SandproofStage1RequestRust.sandproof_protocol.internal.PositiveCounter
abbrev sandproof_protocol.NodeId := SandproofStage1RequestRust.sandproof_protocol.NodeId
abbrev sandproof_protocol.DeploymentId := SandproofStage1RequestRust.sandproof_protocol.DeploymentId
abbrev sandproof_protocol.SandboxId := SandproofStage1RequestRust.sandproof_protocol.SandboxId
abbrev sandproof_protocol.TenantId := SandproofStage1RequestRust.sandproof_protocol.TenantId
abbrev sandproof_protocol.ResourceId := SandproofStage1RequestRust.sandproof_protocol.ResourceId
abbrev sandproof_protocol.lifecycle.digest.ContentDigest := SandproofStage1RequestRust.sandproof_protocol.lifecycle.digest.ContentDigest
abbrev sandproof_protocol.lifecycle.identity.SessionId := SandproofStage1RequestRust.sandproof_protocol.lifecycle.identity.SessionId
abbrev sandproof_protocol.lifecycle.identity.RequestWindowId := SandproofStage1RequestRust.sandproof_protocol.lifecycle.identity.RequestWindowId
abbrev sandproof_protocol.lifecycle.identity.RequestSequence := SandproofStage1RequestRust.sandproof_protocol.lifecycle.identity.RequestSequence
abbrev sandproof_protocol.lifecycle.identity.IncarnationId := SandproofStage1RequestRust.sandproof_protocol.lifecycle.identity.IncarnationId

abbrev core.io.error.Error := SandproofSqliteRust.core.io.error.Error
abbrev rusqlite.error.Error := SandproofSqliteRust.rusqlite.error.Error
abbrev serde_json.error.Error := SandproofSqliteRust.serde_json.error.Error
abbrev sandproof_protocol.ContractError := SandproofStage1RequestRust.sandproof_protocol.ContractError
abbrev CrashPoint := SandproofStage1RequestRust.sandproof_controller_service.CrashPoint

end SandproofBranchBatch
