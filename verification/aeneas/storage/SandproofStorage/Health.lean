import SandproofStorage.Results

open Aeneas Aeneas.Std
namespace SandproofStorageRust.Results

theorem read_reply_error_unavailable {T : Type} (project : LibraryReply → Option T)
    (reply : LibraryReply) (connection next : Option redis.connection.Connection)
    (error : sandproof_center_store.StoreError)
    (run : readReply project reply connection = .ok (.Err error, next)) :
    error = .Unavailable := by
  cases reply <;> simp_all [readReply]
  all_goals split at run <;> simp_all

theorem invocation_error_unavailable {T : Type} (project : LibraryReply → Option T)
    (connection next : Option redis.connection.Connection) (request : LibraryRequest)
    (error : sandproof_center_store.StoreError)
    (run : (let (reply, updated) := invoke connection request
            readReply project reply updated) = .ok (.Err error, next)) :
    error = .Unavailable := by
  rcases call : invoke connection request with ⟨reply, updated⟩
  simp only [call] at run
  exact read_reply_error_unavailable _ _ _ _ _ run

theorem info_error_unavailable (connection next : Option redis.connection.Connection)
    (sectionName : Str) (deadline : std.time.Instant) (phase : store.diagnostics.Phase)
    (error : sandproof_center_store.StoreError)
    (run : store.storage_io.info connection sectionName deadline phase = .ok (.Err error, next)) :
    error = .Unavailable := by
  exact invocation_error_unavailable _ _ _ _ _ run

theorem settings_error_unavailable (connection next : Option redis.connection.Connection)
    (keys : Slice Str) (deadline : std.time.Instant) (phase : store.diagnostics.Phase)
    (error : sandproof_center_store.StoreError)
    (run : store.storage_io.settings connection keys deadline phase = .ok (.Err error, next)) :
    error = .Unavailable := by
  exact invocation_error_unavailable _ _ _ _ _ run

theorem configuration_error_kind (output next : store.storage_io.Diagnostics)
    (phase : store.diagnostics.Phase) (error : sandproof_center_store.StoreError)
    (run : store.configuration_error output phase = .ok (error, next)) :
    error = .Configuration := by
  unfold store.configuration_error at run
  simp only [core.time.Duration.ZERO, bind_tc_ok] at run
  obtain ⟨updated, _emit, run⟩ := bind_returns _ _ run
  simp only [Result.ok.injEq, Prod.mk.injEq] at run
  exact run.1.symm

theorem persistence_error_kind (output next : store.storage_io.Diagnostics)
    (error : sandproof_center_store.StoreError)
    (run : store.persistence_error output = .ok (error, next)) :
    error = .Unavailable := by
  unfold store.persistence_error at run
  simp only [core.time.Duration.ZERO, bind_tc_ok] at run
  obtain ⟨updated, _emit, run⟩ := bind_returns _ _ run
  simp only [Result.ok.injEq, Prod.mk.injEq] at run
  exact run.1.symm

def HealthShape {T : Type} (before : store.RedisLedger T)
    (result : core.result.Result Unit sandproof_center_store.StoreError)
    (after : store.RedisLedger T) : Prop :=
  after = { before with connection := after.connection, diagnostics := after.diagnostics } ∧
  match result with
  | .Ok _ => True
  | .Err error => error = .Configuration ∨ error = .Unavailable

theorem health_shape {T : Type} (ledger after : store.RedisLedger T)
    (deadline : std.time.Instant)
    (result : core.result.Result Unit sandproof_center_store.StoreError)
    (run : store.RedisLedger.check_health ledger deadline = .ok (result, after)) :
    HealthShape ledger result after := by
  unfold store.RedisLedger.check_health at run
  obtain ⟨⟨server, c1⟩, serverResult, run⟩ := bind_returns _ _ run
  simp only [uncurry] at run
  cases server with
  | Err error =>
    have same := info_error_unavailable _ _ _ _ _ _ serverResult
    subst error
    simp [core.result.Result.Insts.CoreOpsTry.branch,
      core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at run
    rcases run with ⟨rfl, rfl⟩
    simp [HealthShape]
  | Ok text =>
    simp only [core.result.Result.Insts.CoreOpsTry.branch, bind_tc_ok] at run
    obtain ⟨bytes, _bytesResult, run⟩ := bind_returns _ _ run
    obtain ⟨version, _versionResult, run⟩ := bind_returns _ _ run
    obtain ⟨badVersion, _versionCheck, run⟩ := bind_returns _ _ run
    by_cases guard0 : badVersion = true
    · rw [if_pos guard0] at run
      obtain ⟨⟨error, diagnostics⟩, emitted, run⟩ := bind_returns _ _ run
      have kind := configuration_error_kind _ _ _ _ emitted
      subst error
      rcases Prod.mk.inj (Result.ok_injective run) with ⟨rfl, rfl⟩
      simp [HealthShape]
    · rw [if_neg guard0] at run
      obtain ⟨keys, _keysResult, run⟩ := bind_returns _ _ run
      obtain ⟨⟨settings, c2⟩, settingsResult, run⟩ := bind_returns _ _ run
      simp only [uncurry] at run
      cases settings with
      | Err error =>
        have same := settings_error_unavailable _ _ _ _ _ _ settingsResult
        subst error
        simp [core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at run
        rcases run with ⟨rfl, rfl⟩
        simp [HealthShape]
      | Ok configuration =>
        simp only [bind_tc_ok] at run
        obtain ⟨memory, _memoryResult, run⟩ := bind_returns _ _ run
        obtain ⟨b1, _check1, run⟩ := bind_returns _ _ run
        by_cases guard1 : b1 = true
        · rw [if_pos guard1] at run
          obtain ⟨b2, _check2, run⟩ := bind_returns _ _ run
          by_cases guard2 : b2 = true
          · rw [if_pos guard2] at run
            obtain ⟨b3, _check3, run⟩ := bind_returns _ _ run
            by_cases guard3 : b3 = true
            · rw [if_pos guard3] at run
              obtain ⟨b4, _check4, run⟩ := bind_returns _ _ run
              by_cases guard4 : b4 = true
              · rw [if_pos guard4] at run
                obtain ⟨b5, _check5, run⟩ := bind_returns _ _ run
                by_cases guard5 : b5 = true
                · rw [if_pos guard5] at run
                  obtain ⟨memoryBytes, _memoryBytesResult, run⟩ := bind_returns _ _ run
                  obtain ⟨b6, _check6, run⟩ := bind_returns _ _ run
                  by_cases guard6 : b6 = true
                  · rw [if_pos guard6] at run
                    obtain ⟨⟨persistence, c3⟩, persistenceResult, run⟩ := bind_returns _ _ run
                    simp only [uncurry] at run
                    cases persistence with
                    | Err error =>
                      have same := info_error_unavailable _ _ _ _ _ _ persistenceResult
                      subst error
                      simp [core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at run
                      rcases run with ⟨rfl, rfl⟩
                      simp [HealthShape]
                    | Ok details =>
                      simp only [bind_tc_ok] at run
                      obtain ⟨detailBytes, _detailBytesResult, run⟩ := bind_returns _ _ run
                      obtain ⟨loading, _loadingResult, run⟩ := bind_returns _ _ run
                      obtain ⟨b7, _check7, run⟩ := bind_returns _ _ run
                      by_cases guard7 : b7 = true
                      · rw [if_pos guard7] at run
                        obtain ⟨⟨error, diagnostics⟩, emitted, run⟩ := bind_returns _ _ run
                        have kind := persistence_error_kind _ _ _ emitted
                        subst error
                        rcases Prod.mk.inj (Result.ok_injective run) with ⟨rfl, rfl⟩
                        simp [HealthShape]
                      · rw [if_neg guard7] at run
                        obtain ⟨enabled, _enabledResult, run⟩ := bind_returns _ _ run
                        obtain ⟨b8, _check8, run⟩ := bind_returns _ _ run
                        by_cases guard8 : b8 = true
                        · rw [if_pos guard8] at run
                          obtain ⟨⟨error, diagnostics⟩, emitted, run⟩ := bind_returns _ _ run
                          have kind := persistence_error_kind _ _ _ emitted
                          subst error
                          rcases Prod.mk.inj (Result.ok_injective run) with ⟨rfl, rfl⟩
                          simp [HealthShape]
                        · rw [if_neg guard8] at run
                          obtain ⟨writeStatus, _writeResult, run⟩ := bind_returns _ _ run
                          obtain ⟨b9, _check9, run⟩ := bind_returns _ _ run
                          by_cases guard9 : b9 = true
                          · rw [if_pos guard9] at run
                            obtain ⟨⟨error, diagnostics⟩, emitted, run⟩ := bind_returns _ _ run
                            have kind := persistence_error_kind _ _ _ emitted
                            subst error
                            rcases Prod.mk.inj (Result.ok_injective run) with ⟨rfl, rfl⟩
                            simp [HealthShape]
                          · rw [if_neg guard9] at run
                            obtain ⟨rewrite, _rewriteResult, run⟩ := bind_returns _ _ run
                            obtain ⟨b10, _check10, run⟩ := bind_returns _ _ run
                            by_cases finalGuard : b10 = true
                            · rw [if_pos finalGuard] at run
                              obtain ⟨⟨error, diagnostics⟩, emitted, run⟩ := bind_returns _ _ run
                              have kind := persistence_error_kind _ _ _ emitted
                              subst error
                              rcases Prod.mk.inj (Result.ok_injective run) with ⟨rfl, rfl⟩
                              simp [HealthShape]
                            · rw [if_neg finalGuard] at run
                              simp only [Result.ok.injEq, Prod.mk.injEq] at run
                              rcases run with ⟨rfl, rfl⟩
                              simp [HealthShape]
                  · rw [if_neg guard6] at run
                    obtain ⟨⟨error, diagnostics⟩, emitted, run⟩ := bind_returns _ _ run
                    have kind := configuration_error_kind _ _ _ _ emitted
                    subst error
                    rcases Prod.mk.inj (Result.ok_injective run) with ⟨rfl, rfl⟩
                    simp [HealthShape]
                · rw [if_neg guard5] at run
                  obtain ⟨⟨error, diagnostics⟩, emitted, run⟩ := bind_returns _ _ run
                  have kind := configuration_error_kind _ _ _ _ emitted
                  subst error
                  rcases Prod.mk.inj (Result.ok_injective run) with ⟨rfl, rfl⟩
                  simp [HealthShape]
              · rw [if_neg guard4] at run
                obtain ⟨⟨error, diagnostics⟩, emitted, run⟩ := bind_returns _ _ run
                have kind := configuration_error_kind _ _ _ _ emitted
                subst error
                rcases Prod.mk.inj (Result.ok_injective run) with ⟨rfl, rfl⟩
                simp [HealthShape]
            · rw [if_neg guard3] at run
              obtain ⟨⟨error, diagnostics⟩, emitted, run⟩ := bind_returns _ _ run
              have kind := configuration_error_kind _ _ _ _ emitted
              subst error
              rcases Prod.mk.inj (Result.ok_injective run) with ⟨rfl, rfl⟩
              simp [HealthShape]
          · rw [if_neg guard2] at run
            obtain ⟨⟨error, diagnostics⟩, emitted, run⟩ := bind_returns _ _ run
            have kind := configuration_error_kind _ _ _ _ emitted
            subst error
            rcases Prod.mk.inj (Result.ok_injective run) with ⟨rfl, rfl⟩
            simp [HealthShape]
        · rw [if_neg guard1] at run
          obtain ⟨⟨error, diagnostics⟩, emitted, run⟩ := bind_returns _ _ run
          have kind := configuration_error_kind _ _ _ _ emitted
          subst error
          rcases Prod.mk.inj (Result.ok_injective run) with ⟨rfl, rfl⟩
          simp [HealthShape]

theorem health_preserves_config {T : Type} (ledger after : store.RedisLedger T)
    (deadline : std.time.Instant)
    (result : core.result.Result Unit sandproof_center_store.StoreError)
    (run : store.RedisLedger.check_health ledger deadline = .ok (result, after)) :
    after.config = ledger.config := by
  have same := congrArg store.RedisLedger.config (health_shape ledger after deadline result run).1
  exact same

theorem health_preserves_fence {T : Type} (ledger after : store.RedisLedger T)
    (deadline : std.time.Instant)
    (result : core.result.Result Unit sandproof_center_store.StoreError)
    (run : store.RedisLedger.check_health ledger deadline = .ok (result, after)) :
    after.write_fenced = ledger.write_fenced := by
  have same := congrArg store.RedisLedger.write_fenced (health_shape ledger after deadline result run).1
  exact same

theorem health_cannot_report_unknown {T : Type} (ledger after : store.RedisLedger T)
    (deadline : std.time.Instant)
    (run : store.RedisLedger.check_health ledger deadline = .ok (.Err .OutcomeUnknown, after)) :
    False := by
  have kinds := (health_shape ledger after deadline (.Err .OutcomeUnknown) run).2
  rcases kinds with same | same <;> cases same

/- diagnostics is proof-side output history for a zero-sized Rust endpoint;
all five original non-connection business fields remain exactly unchanged. -/
theorem health_preserves_business_fields {T : Type} (ledger after : store.RedisLedger T)
    (deadline : std.time.Instant)
    (result : core.result.Result Unit sandproof_center_store.StoreError)
    (run : store.RedisLedger.check_health ledger deadline = .ok (result, after)) :
    after.endpoint = ledger.endpoint ∧ after.config = ledger.config ∧
      after._lease = ledger._lease ∧ after.write_fenced = ledger.write_fenced ∧
      after._state = ledger._state := by
  have frame := (health_shape ledger after deadline result run).1
  have endpoint := congrArg store.RedisLedger.endpoint frame
  have config := congrArg store.RedisLedger.config frame
  have lease := congrArg store.RedisLedger._lease frame
  have fence := congrArg store.RedisLedger.write_fenced frame
  have state := congrArg store.RedisLedger._state frame
  exact ⟨endpoint, config, lease, fence, state⟩

#print axioms health_shape
#print axioms health_cannot_report_unknown

end SandproofStorageRust.Results
