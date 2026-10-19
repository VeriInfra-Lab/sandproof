import SandproofStorage.Funs
import SandboxModel.Foundations.ErrorMessages

open Aeneas Aeneas.Std
namespace SandproofStorageRust.Results

theorem key_preserves_deployment (config : store.StoreConfig)
    (represented : SandproofFoundationsRust.IdentifierLibrary.StringRepresentable config.deployment) :
    store.StoreConfig.key config = .ok ("sandproof:authority:" ++ config.deployment) := by
  have prefixBytes :
      (SandproofFoundationsRust.IdentifierLibrary.stringBytes "sandproof:authority:").map (·.val) =
        [115, 97, 110, 100, 112, 114, 111, 111, 102, 58, 97, 117, 116, 104, 111, 114, 105, 116, 121, 58] := by
    decide
  simp [prefixBytes, store.StoreConfig.key, sandproof_protocol.DeploymentId.as_str,
    alloc.string.String.Insts.CoreOpsDerefDerefStr.deref,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreOpsDerefDerefStr.deref,
    represented, core.fmt.rt.Argument.new_display,
    SandproofFoundationsRust.core.fmt.rt.Argument.new_display,
    core.fmt.Arguments.new, SandproofFoundationsRust.core.fmt.Arguments.new,
    alloc.fmt.format,
    Shared0T.Insts.CoreFmtDisplay.fmt,
    SandproofFoundationsRust.Shared0T.Insts.CoreFmtDisplay.fmt,
    Str.Insts.CoreFmtDisplay, Str.Insts.CoreFmtDisplay.fmt,
    SandproofFoundationsRust.core.fmt.Formatter.write_str,
    SandboxModel.Foundations.decodeString_stringBytes,
    core.hint.must_use]

#print axioms key_preserves_deployment

end SandproofStorageRust.Results
