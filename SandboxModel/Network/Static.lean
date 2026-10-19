import SandboxModel.Network.Model
import SandproofNetwork.Funs

/-! Refinement of the real generated endpoint equality and static policy loop.
The finite-list traversal, checked index increment and termination are owned
implementation obligations, not part of the standard-library contract. -/
namespace SandboxModel.Network
open Aeneas Aeneas.Std Result ControlFlow SandproofNetworkRust

theorem protocol_eq_refines
    (left right : sandbox_controller_core.model.Protocol) :
    sandbox_controller_core.model.Protocol.Insts.CoreCmpPartialEqProtocol.eq left right =
      .ok (decide ((match left with | .Tcp => false | .Udp => true) =
        (match right with | .Tcp => false | .Udp => true))) := by
  cases left <;> cases right <;>
    simp [sandbox_controller_core.model.Protocol.Insts.CoreCmpPartialEqProtocol.eq,
      sandbox_controller_core.model.Protocol.read_discriminant]

theorem endpoint_eq_refines
    (left right : sandbox_controller_core.model.Endpoint) :
    sandbox_controller_core.model.Endpoint.Insts.CoreCmpPartialEqEndpoint.eq left right =
      .ok (decide (endpointView left = endpointView right)) := by
  unfold sandbox_controller_core.model.Endpoint.Insts.CoreCmpPartialEqEndpoint.eq
  rw [protocol_eq_refines]
  by_cases samePrefix : left.prefix_length = right.prefix_length <;>
    by_cases samePort : left.port = right.port <;>
    by_cases sameAddress : left.address = right.address <;>
      simp [samePrefix, samePort, sameAddress, endpointView, Endpoint.mk.injEq,
        ← UScalar.eq_equiv]
  all_goals rfl

theorem static_loop_refines
    (endpoints : alloc.vec.Vec sandbox_controller_core.model.Endpoint)
    (destination : sandbox_controller_core.model.Endpoint)
    (start : Usize) (allowed : Bool) :
    sandbox_controller_core.network.NetworkPolicy.destination_allowed_loop
      endpoints destination start allowed
      ⦃ result => result = (allowed ||
        ((endpoints.val.drop start.val).map endpointView).any
          (fun endpoint => decide (endpoint = endpointView destination))) ⦄ := by
  unfold sandbox_controller_core.network.NetworkPolicy.destination_allowed_loop
  apply Aeneas.Std.loop.spec_decr_nat
    (fun state : Usize × Bool => endpoints.val.length - state.1.val)
    (fun state : Usize × Bool =>
      (state.2 || ((endpoints.val.drop state.1.val).map endpointView).any
        (fun endpoint => decide (endpoint = endpointView destination))) =
      (allowed || ((endpoints.val.drop start.val).map endpointView).any
        (fun endpoint => decide (endpoint = endpointView destination))))
    (fun result => result = (allowed ||
      ((endpoints.val.drop start.val).map endpointView).any
        (fun endpoint => decide (endpoint = endpointView destination))))
  · rintro ⟨cursor, found⟩ invariant
    unfold sandbox_controller_core.network.NetworkPolicy.destination_allowed_loop.body
    dsimp only
    split
    · rename_i inBounds
      by_cases alreadyFound : found = true
      · simp [alreadyFound]
        simpa [alreadyFound] using invariant.symm
      · have foundFalse : found = false := by cases found <;> simp_all
        simp [foundFalse]
        step as ⟨current, currentPost⟩
        have indexBound : cursor.val < endpoints.val.length := by simpa using inBounds
        have dropEq : endpoints.val.drop cursor.val =
            current :: endpoints.val.drop (cursor.val + 1) := by
          rw [List.drop_eq_getElem_cons indexBound, ← currentPost]
        rw [endpoint_eq_refines current destination]
        by_cases same : endpointView current = endpointView destination
        · simp [same]
          step as ⟨next, nextPost⟩
          have targetTrue :
              (allowed || ((endpoints.val.drop start.val).map endpointView).any
                (fun endpoint => decide (endpoint = endpointView destination))) = true := by
            rw [dropEq] at invariant
            simpa [foundFalse, same] using invariant.symm
          constructor
          · simpa using targetTrue.symm
          · omega
        · simp [same]
          step as ⟨next, nextPost⟩
          constructor
          · rw [nextPost]
            rw [dropEq] at invariant
            simpa [foundFalse, same] using invariant
          · omega
    · rename_i outside
      simp only [WP.spec_ok]
      have lower : endpoints.val.length ≤ cursor.val := by simpa using outside
      rw [List.drop_eq_nil_of_le lower] at invariant
      simpa using invariant
  · rfl

theorem static_policy_refines
    (endpoints : alloc.vec.Vec sandbox_controller_core.model.Endpoint)
    (destination : sandbox_controller_core.model.Endpoint) :
    sandbox_controller_core.network.NetworkPolicy.destination_allowed
        (.StaticAllowlist endpoints) destination =
      .ok (allows (policyView (.StaticAllowlist endpoints)) (endpointView destination)) := by
  obtain ⟨out, he, hr⟩ := Aeneas.Std.WP.spec_imp_exists
    (static_loop_refines endpoints destination 0#usize false)
  subst out
  simpa [sandbox_controller_core.network.NetworkPolicy.destination_allowed,
    policyView, allows] using he

end SandboxModel.Network
