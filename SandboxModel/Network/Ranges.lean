import SandboxModel.Network.Contains
import SandboxModel.Foundations.FiniteScan
open Aeneas Aeneas.Std Result ControlFlow SandproofNetworkRust
namespace SandboxModel.Network

theorem ranges_body_agrees
    (ranges : Slice sandbox_controller_core.model.Ipv4Cidr) (address : U32) (cursor : Usize) :
    sandbox_controller_core.network.ipv4_list_contains_loop.body ranges address cursor =
      SandboxModel.Foundations.vectorAnyBody ⟨ranges⟩
        (fun range => decide ((rangeView range).matches address.val)) cursor := by
  unfold sandbox_controller_core.network.ipv4_list_contains_loop.body
    SandboxModel.Foundations.vectorAnyBody
  by_cases bound : cursor.val < ranges.val.length
  · have inBounds : cursor < Slice.len ranges := by simpa using bound
    obtain ⟨value, evaluated, same⟩ := WP.spec_imp_exists (Slice.index_usize_spec ranges cursor bound)
    subst value
    simp [inBounds, bound, evaluated, cidr_contains_refines, alloc.vec.Vec.val]
  · have outside : ¬ cursor < Slice.len ranges := by simpa using bound
    simp [outside, bound, alloc.vec.Vec.val]

theorem ranges_loop_refines
    (ranges : Slice sandbox_controller_core.model.Ipv4Cidr) (address : U32) (start : Usize) :
    sandbox_controller_core.network.ipv4_list_contains_loop ranges address start
      ⦃ out => out = rangesMatch ((ranges.val.drop start.val).map rangeView) address.val ⦄ := by
  unfold sandbox_controller_core.network.ipv4_list_contains_loop
  have bodySame : sandbox_controller_core.network.ipv4_list_contains_loop.body ranges address =
      SandboxModel.Foundations.vectorAnyBody ⟨ranges⟩
        (fun range => decide ((rangeView range).matches address.val)) := funext (ranges_body_agrees ranges address)
  rw [bodySame]
  by_cases bound : start.val ≤ ranges.val.length
  · simpa [rangesMatch, List.any_map, ← List.map_drop, alloc.vec.Vec.val, Function.comp_def] using
      SandboxModel.Foundations.vectorAnyLoop_refines ⟨ranges⟩
        (fun range => decide ((rangeView range).matches address.val)) start bound
  · have outside : ¬ start.val < ranges.val.length := by omega
    rw [loop]
    simp [SandboxModel.Foundations.vectorAnyBody, outside, rangesMatch,
      List.drop_eq_nil_of_le (by omega : ranges.val.length ≤ start.val), alloc.vec.Vec.val]

theorem ranges_refines
    (ranges : Slice sandbox_controller_core.model.Ipv4Cidr) (address : U32) :
    sandbox_controller_core.network.ipv4_list_contains ranges address =
      .ok (rangesMatch (ranges.val.map rangeView) address.val) := by
  obtain ⟨out, he, hr⟩ := WP.spec_imp_exists (ranges_loop_refines ranges address 0#usize)
  subst out
  simpa [sandbox_controller_core.network.ipv4_list_contains] using he

theorem platform_ranges_refines :
    sandbox_controller_core.network.PLATFORM_DENY_IPV4.val.map rangeView = platformRanges := by
  unfold sandbox_controller_core.network.PLATFORM_DENY_IPV4
  rfl

theorem platform_protected_refines (address : U32) :
    sandbox_controller_core.network.platform_protected_ipv4 address =
      .ok (platformProtected address.val) := by
  unfold sandbox_controller_core.network.platform_protected_ipv4
  rw [ranges_refines, platform_ranges_refines]
  rfl

theorem platform_root_refines (address : U32) :
    SandproofNetworkRust.protected_ipv4 address = .ok (platformProtected address.val) := by
  exact platform_protected_refines address

end SandboxModel.Network
