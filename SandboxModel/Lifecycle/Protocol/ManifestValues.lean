import SandproofStage1Manifest.Funs
import SandboxModel.Lifecycle.Protocol.Model
import SandboxModel.ProofAuditCommand
import SandboxModel.Lifecycle.Protocol.LoopWitness
import SandboxModel.Foundations.Identifiers
import SandboxModel.Foundations.ByteArray
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.Manifest
abbrev WireManifest := sandproof_protocol.lifecycle.manifest.SnapshotManifest
abbrev Profile := sandproof_protocol.lifecycle.manifest.VmCompatibility
abbrev Capture := sandproof_protocol.lifecycle.manifest.CaptureBinding
abbrev File := sandproof_protocol.lifecycle.manifest.SnapshotFile

def digest : Array U8 32#usize := Array.from (List.replicate 32 (0#u8)) (by simp)
def original : Capture := {
  operation := {
    protocol_version := 3#u16, node := ⟨"deployment", "node", 1#u64⟩,
    session := "session", key := ⟨"deployment", "owner", 1#u64, 1#u64⟩,
    expected := .Version 1#u64 },
  source_incarnation := 1#u64, capture := 1#u64 }
def changed : Capture := {
  original with operation := {
    original.operation with key := { original.operation.key with sequence := 2#u64 } } }
def profile : Profile := {
  snapshot_format := 1#u16, tool := digest, cpu := digest, devices := digest,
  image_configuration := digest,
  writable_drives := alloc.vec.Vec.from [⟨"rootfs", "r"⟩] (by simp only [List.length_cons, List.length_nil]; scalar_tac) }
def files : alloc.vec.Vec File := alloc.vec.Vec.from [
  ⟨.State, "state.bin", 1#u64, digest⟩,
  ⟨.Memory, "memory.bin", 1#u64, digest⟩,
  ⟨.Disk "rootfs", "disk-0.img", 1#u64, digest⟩] (by simp only [List.length_cons, List.length_nil]; scalar_tac)
def manifestWith (p : Profile) (values : alloc.vec.Vec File) : WireManifest :=
  ⟨1#u16, original, p, values⟩
def manifest : WireManifest := manifestWith profile files

open SandboxModel.Stage1Protocol.ManifestLoop
theorem bytes_eq_self (values : List U8) :
    List.allM (fun (pair : U8 × U8) => .ok (decide (pair.1.val = pair.2.val)))
      (List.zip values values) = (.ok true : Result Bool) := by
  induction values with
  | nil => simp [pure]
  | cons value rest ih => simp [pure, ih]
theorem digest_eq_self {size : Usize} (value : Array U8 size) :
    core.array.equality.PartialEqArray.eq core.cmp.PartialEqU8 value value = .ok true := by
  simp [core.array.equality.PartialEqArray.eq, bytes_eq_self]
theorem deref_r :
    alloc.string.String.Insts.CoreOpsDerefDerefStr.deref "r" = .ok ManifestLoop.bytes := by
  have bound : SandproofFoundationsRust.IdentifierLibrary.StringRepresentable "r" := by
    simp [SandproofFoundationsRust.IdentifierLibrary.StringRepresentable,
      SandproofFoundationsRust.IdentifierLibrary.stringBytes]
    change (1 : Nat) ≤ Usize.max
    scalar_tac
  rw [alloc.string.String.Insts.CoreOpsDerefDerefStr.deref,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreOpsDerefDerefStr.deref, dif_pos bound]
  rfl
theorem distinct_first (drives : Slice sandproof_protocol.lifecycle.manifest.WritableDrive) :
    sandproof_protocol.lifecycle.manifest.distinct_drive drives 0#usize = .ok true := by
  unfold sandproof_protocol.lifecycle.manifest.distinct_drive
    sandproof_protocol.lifecycle.manifest.distinct_drive_loop
  apply loop_done
  simp [sandproof_protocol.lifecycle.manifest.distinct_drive_loop.body]
theorem profile_first_step :
    sandproof_protocol.lifecycle.manifest.VmCompatibility.validate_loop.body
      profile.writable_drives 0#usize true = .ok (.cont (1#usize, true)) := by
  have add_one : ((0#usize + 1#usize) : Result Usize) = .ok 1#usize := add_exact _ _ _ (by scalar_tac)
  simp [sandproof_protocol.lifecycle.manifest.VmCompatibility.validate_loop.body,
    profile, alloc.vec.Vec.from, alloc.vec.Vec.val, alloc.vec.Vec.index,
    alloc.vec.Vec.deref, Slice.index_usize, deref_r, path_valid, distinct_first, add_one]
theorem profile_loop :
    sandproof_protocol.lifecycle.manifest.VmCompatibility.validate_loop
      profile.writable_drives 0#usize true = .ok true := by
  unfold sandproof_protocol.lifecycle.manifest.VmCompatibility.validate_loop
  refine loop_cont _ _ (1#usize, true) _ ?_ ?_
  · exact profile_first_step
  · apply loop_done
    simp [sandproof_protocol.lifecycle.manifest.VmCompatibility.validate_loop.body, profile]
theorem profile_valid :
    sandproof_protocol.lifecycle.manifest.VmCompatibility.validate profile = .ok (.Ok ()) := by
  have completed := profile_loop
  simp only [profile] at completed
  simp [sandproof_protocol.lifecycle.manifest.VmCompatibility.validate, profile,
    alloc.vec.Vec.is_empty, SandproofFoundationsRust.alloc.vec.Vec.is_empty,
    sandproof_protocol.lifecycle.manifest.MAX_WRITABLE_DRIVES, completed]
theorem profile_eq_self :
    sandproof_protocol.lifecycle.manifest.VmCompatibility.Insts.CoreCmpPartialEqVmCompatibility.eq
      profile profile = .ok true := by
  simp [sandproof_protocol.lifecycle.manifest.VmCompatibility.Insts.CoreCmpPartialEqVmCompatibility.eq,
    profile, sandproof_protocol.lifecycle.digest.ContentDigest.Insts.CoreCmpPartialEqContentDigest.eq,
    digest_eq_self, alloc.vec.partial_eq.PartialEqVec.eq,
    sandproof_protocol.lifecycle.manifest.WritableDrive.Insts.CoreCmpPartialEqWritableDrive.eq,
    sandproof_protocol.ResourceId.Insts.CoreCmpPartialEqResourceId.eq,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq, pure]
theorem literal_owned (text : String) (bounded : text.toByteArray.size ≤ U32.max) :
    Str.Insts.AllocBorrowToOwnedString.to_owned (SandproofStage1ManifestRust.toStr text bounded) =
      .ok text := by
  have represented : (SandproofStage1ManifestRust.toStr text bounded).val =
      SandproofFoundationsRust.IdentifierLibrary.stringBytes text := by
    simp only [SandproofStage1ManifestRust.toStr, SandproofFoundationsRust.toStr,
      Aeneas.Std.toStr, Slice.from_val, SandproofFoundationsRust.IdentifierLibrary.stringBytes,
      SandboxModel.Foundations.byteArray_toList_data]
    rfl
  simp [Str.Insts.AllocBorrowToOwnedString.to_owned,
    SandproofFoundationsRust.Str.Insts.AllocBorrowToOwnedString.to_owned,
    represented, SandboxModel.Foundations.decodeString_stringBytes]
theorem sub_exact (x y z : Usize) (bound : y.val ≤ x.val)
    (difference : z.val = x.val - y.val) : ((x - y) : Result Usize) = .ok z := by
  have spec := Usize.sub_bv_spec (x := x) (y := y) bound
  obtain ⟨value, equation, post⟩ := (WP.spec_equiv_exists _ _).mp spec
  have same : value = z := by
    apply UScalar.eq_of_val_eq
    scalar_tac
  simpa [same] using equation
theorem state_file_length :
    sandproof_protocol.lifecycle.manifest.file_length
      ⟨.State, "state.bin", 1#u64, digest⟩ 0#usize profile = .ok (.Ok 1#u64) := by
  simp [sandproof_protocol.lifecycle.manifest.file_length, literal_owned,
    core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq]
theorem memory_file_length :
    sandproof_protocol.lifecycle.manifest.file_length
      ⟨.Memory, "memory.bin", 1#u64, digest⟩ 1#usize profile = .ok (.Ok 1#u64) := by
  simp [sandproof_protocol.lifecycle.manifest.file_length, literal_owned,
    core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq]
theorem disk_file_length :
    sandproof_protocol.lifecycle.manifest.file_length
      ⟨.Disk "rootfs", "disk-0.img", 1#u64, digest⟩ 2#usize profile = .ok (.Ok 1#u64) := by
  have difference : ((2#usize - 2#usize) : Result Usize) = .ok 0#usize :=
    sub_exact _ _ _ (by scalar_tac) (by scalar_tac)
  simp [sandproof_protocol.lifecycle.manifest.file_length, profile,
    alloc.vec.Vec.index, alloc.vec.Vec.from,
    Slice.index_usize, difference, sandproof_protocol.lifecycle.manifest.disk_bundle_path,
    literal_owned, core.option.Option.ok_or, core.result.Result.Insts.CoreOpsTry.branch,
    core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    sandproof_protocol.ResourceId.Insts.CoreCmpPartialEqResourceId.eq,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq]
end SandboxModel.Stage1Protocol.Manifest

#audit_controller_proof SandboxModel.Stage1Protocol.Manifest.bytes_eq_self
#audit_controller_proof SandboxModel.Stage1Protocol.Manifest.digest_eq_self
#audit_controller_proof SandboxModel.Stage1Protocol.Manifest.deref_r
#audit_controller_proof SandboxModel.Stage1Protocol.Manifest.distinct_first
#audit_controller_proof SandboxModel.Stage1Protocol.Manifest.profile_first_step
#audit_controller_proof SandboxModel.Stage1Protocol.Manifest.profile_loop
#audit_controller_proof SandboxModel.Stage1Protocol.Manifest.profile_valid
#audit_controller_proof SandboxModel.Stage1Protocol.Manifest.profile_eq_self
#audit_controller_proof SandboxModel.Stage1Protocol.Manifest.literal_owned
#audit_controller_proof SandboxModel.Stage1Protocol.Manifest.sub_exact
#audit_controller_proof SandboxModel.Stage1Protocol.Manifest.state_file_length
#audit_controller_proof SandboxModel.Stage1Protocol.Manifest.memory_file_length
#audit_controller_proof SandboxModel.Stage1Protocol.Manifest.disk_file_length
