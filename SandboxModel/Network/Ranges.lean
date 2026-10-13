import SandboxModel.Network.Contains
open Aeneas Aeneas.Std Result ControlFlow SandproofNetworkRust
namespace SandboxModel.Network

theorem ranges_loop_refines
    (ranges : Slice sandbox_controller_core.model.Ipv4Cidr) (address : U32) (start : Usize) :
    sandbox_controller_core.network.ipv4_list_contains_loop ranges address start
      ⦃ out => out = rangesMatch ((ranges.val.drop start.val).map rangeView) address.val ⦄ := by
  unfold sandbox_controller_core.network.ipv4_list_contains_loop
  apply Aeneas.Std.loop.spec_decr_nat
    (fun index => ranges.val.length - index.val)
    (fun index => rangesMatch ((ranges.val.drop index.val).map rangeView) address.val =
      rangesMatch ((ranges.val.drop start.val).map rangeView) address.val)
    (fun out => out = rangesMatch ((ranges.val.drop start.val).map rangeView) address.val)
  · intro index invariant
    unfold sandbox_controller_core.network.ipv4_list_contains_loop.body
    dsimp only
    split
    · rename_i inBounds
      step as ⟨range, rangePost⟩
      rw [cidr_contains_refines range address]
      have indexBound : index.val < ranges.val.length := by simpa using inBounds
      have dropEq : ranges.val.drop index.val = range :: ranges.val.drop (index.val + 1) := by
        rw [List.drop_eq_getElem_cons indexBound, ← rangePost]
      by_cases hit : (rangeView range).matches address.val
      · simp [hit]
        rw [dropEq] at invariant
        simpa [rangesMatch, hit] using invariant
      · simp [hit]
        have nextInvariant :
            rangesMatch ((ranges.val.drop (index.val + 1)).map rangeView) address.val =
              rangesMatch ((ranges.val.drop start.val).map rangeView) address.val := by
          rw [dropEq] at invariant
          simpa [rangesMatch, hit] using invariant
        step as ⟨next, nextPost⟩
        constructor
        · rw [nextPost]
          simpa using nextInvariant
        · omega
    · rename_i outside
      simp only [WP.spec_ok]
      have lower : ranges.val.length ≤ index.val := by simpa using outside
      rw [List.drop_eq_nil_of_le lower] at invariant
      simpa [rangesMatch] using invariant
  · rfl

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
