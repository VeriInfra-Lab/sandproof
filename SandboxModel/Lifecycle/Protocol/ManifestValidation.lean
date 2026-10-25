import SandboxModel.Lifecycle.Protocol.ManifestScan
import SandboxModel.Lifecycle.Protocol.ManifestValidationLaws
import SandboxModel.Lifecycle.Protocol.ManifestProfile
import SandboxModel.Lifecycle.Protocol.ManifestEquality
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.ManifestValidation
open sandproof_protocol.lifecycle.manifest sandproof_protocol.lifecycle.identity
open SandboxModel.Stage1Protocol.ManifestLength
open SandboxModel.Stage1Protocol.ManifestProfile
open SandboxModel.Stage1Protocol.ManifestEquality

theorem validate_refines (value : SnapshotManifest) (expected : CaptureBinding)
    (profile : Profile) (represented : Representable profile) :
    (do let result ← manifest_validate value expected profile; .ok (observe result)) =
      .ok (validationResult value expected profile) := by
  classical
  unfold manifest_validate SnapshotManifest.validate validationResult
  simp only [MANIFEST_FORMAT_VERSION, sandproof_protocol.lifecycle.PROTOCOL_VERSION,
    core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    capture_eq, profile_eq,
    sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId.eq,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    bind_tc_ok]
  by_cases format : value.format_version = 1#u16
  · simp only [format, ne_eq, not_true_eq_false, if_false, bne_self_eq_false, Bool.false_eq_true]
    by_cases binding : value.binding = expected
    · subst expected
      simp only [decide_true, if_false, not_true_eq_false]
      by_cases protocol : value.binding.operation.protocol_version = 4#u16
      · simp only [protocol, bne_self_eq_false, Bool.false_eq_true, if_false]
        by_cases deployment : value.binding.operation.key.deployment = value.binding.operation.node.deployment_id
        · simp only [deployment, decide_true]
          cases session : value.binding.operation.expected with
          | Absent => simp [session, BindingValid, HasVersion, observe]
          | Version version =>
            have valid_binding : BindingValid value.binding := ⟨protocol, deployment, version, session⟩
            simp only [bind_tc_ok, if_true, valid_binding, not_true_eq_false, if_false]
            by_cases compatible : value.compatibility = profile
            · simp only [compatible, decide_true, if_false,
                not_true_eq_false, profile_refines profile represented]
              by_cases valid_profile : ValidProfile profile
              · simp only [valid_profile, if_pos, not_true_eq_false, if_false,
                  core.result.Result.Insts.CoreOpsTry.branch, bind_tc_ok]
                have bound : profile.writable_drives.val.length ≤ 16 := valid_profile.2.2.1
                obtain ⟨count, add, post⟩ := (WP.spec_equiv_exists _ _).mp
                  (Usize.add_bv_spec (x := 2#usize) (y := alloc.vec.Vec.len profile.writable_drives)
                    (by scalar_tac))
                have count_val : count.val = 2 + profile.writable_drives.val.length := by
                  simpa using post.1
                simp only [add, bind_tc_ok]
                by_cases files : value.files.val.length = 2 + profile.writable_drives.val.length
                · have equal_count : alloc.vec.Vec.len value.files = count := by scalar_tac
                  simp only [equal_count, bne_self_eq_false, Bool.false_eq_true, if_false,
                    files, not_true_eq_false]
                  exact scan_refines value.files profile 0#u64 0#usize files bound
                · have different_count : ¬ alloc.vec.Vec.len value.files = count := by
                    intro h
                    have := congrArg UScalar.val h
                    simp at this
                    omega
                  simp [different_count, files, observe]
              · simp [valid_profile, core.result.Result.Insts.CoreOpsTry.branch,
                  core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
                  observe]
            · simp [compatible, observe]
        · simp [deployment, BindingValid, observe]
      · simp [protocol, BindingValid, observe]
    · simp [binding, observe]
  · simp [format, observe]

theorem validate_contract : ValidationContract manifest_validate := validate_refines

theorem accepted_iff (value : SnapshotManifest) (expected : CaptureBinding)
    (profile : Profile) (represented : Representable profile) (total : U64) :
    manifest_validate value expected profile = .ok (.Ok total) ↔
      Accepted value expected profile total.val := by
  rw [← observe_run_iff _ (.Ok total), validate_refines value expected profile represented]
  simp only [observe, Result.ok.injEq]
  exact validation_result_ok value expected profile total.val

theorem refused_iff (value : SnapshotManifest) (expected : CaptureBinding)
    (profile : Profile) (represented : Representable profile) (error : Err) :
    manifest_validate value expected profile = .ok (.Err error) ↔
      validationResult value expected profile = .Err error := by
  rw [← observe_run_iff _ (.Err error), validate_refines value expected profile represented]
  simp [observe]

end SandboxModel.Stage1Protocol.ManifestValidation

#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.validate_refines
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.validate_contract

#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.accepted_iff
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.refused_iff
