// Real application and SQLite tests. The original fixture above controls only
// physical effects and time; the existing commit hook exercises both outcomes.

struct DataAuthorityPilotFixture {
    _directory: TempDir,
    app: ControllerApplication<TestHost>,
    host: TestHost,
    session: sandproof_controller_service::application::Session,
}

impl DataAuthorityPilotFixture {
    fn running() -> Self {
        let directory = TempDir::new().unwrap();
        let host = TestHost::new(TestClock::at(1000));
        let app = application(&directory, host.clone());
        let session = app.create("1", request()).unwrap();
        Self {
            _directory: directory,
            app,
            host,
            session,
        }
    }

    fn permit(&self) -> sandproof_controller_service::application::DataPermit {
        self.app
            .data_permit(&self.session.envd_access_token, DataKind::Exec)
            .unwrap()
    }

    fn revoke_with_rolled_back_timeout(
        &self,
    ) -> sandproof_controller_service::application::DataPermit {
        let old = self.permit();
        let before = self.app.get("1", &self.session.sandbox_id).unwrap();
        crate::controller::data_authority_pilot_fail_next_commit(false);
        assert_eq!(
            self.app.set_timeout("1", &self.session.sandbox_id, 0),
            Err(ApplicationError::OutcomeUnknown)
        );
        assert!(*old.revocation.borrow());
        let after = self.app.get("1", &self.session.sandbox_id).unwrap();
        assert_eq!(after.info.end_at_ms, before.info.end_at_ms);
        assert_eq!(after.info.started_at_ms, before.info.started_at_ms);
        assert_eq!(
            self.app
                .authorize_data(&self.session.envd_access_token, DataKind::Exec)
                .unwrap(),
            old.target
        );
        old
    }
}

#[test]
fn da_base_gate_failure_does_not_register_any_route() {
    for missing_gate in [false, true] {
        let fixture = DataAuthorityPilotFixture::running();
        {
            let mut state = fixture.host.state.lock().unwrap();
            state.dp_gate_fails = !missing_gate;
            state.dp_gate_missing = missing_gate;
        }
        assert!(matches!(
            fixture.app.data_permit(&fixture.session.envd_access_token, DataKind::Exec),
            Err(ApplicationError::Busy)
        ));
        assert_eq!(
            fixture.app.data_authority_pilot_registration_snapshot(),
            (Vec::new(), 0)
        );
        let state = fixture.host.state.lock().unwrap();
        assert_eq!(state.dp_gate_calls.len(), 1);
        assert!(state.dp_relay_inputs.is_empty());
        println!("DA_RECEIPT gate missing={missing_gate} calls=1 signals=0 drains=0");
    }
}

#[test]
fn da_base_revoked_watch_refuses_even_after_real_timeout_rollback() {
    let fixture = DataAuthorityPilotFixture::running();
    let old = fixture.revoke_with_rolled_back_timeout();
    let fresh = fixture.permit();
    assert!(*old.revocation.borrow());
    assert!(!*fresh.revocation.borrow());
    assert_eq!(old.target, fresh.target);
    assert_eq!(
        fixture.app.data_authority_pilot_registration_snapshot(),
        (vec![(1, true), (1, false)], 1)
    );
    let old_result = fixture.app.spawn_data_relay(&old, false);
    assert_eq!(
        fixture.host.relay_calls.load(Ordering::SeqCst),
        0,
        "an old revoked permit must not reach the real host interface after rollback"
    );
    assert!(matches!(old_result, Err(ApplicationError::NotFound)));
    // The fresh permit passes the remaining real guards. The controlled host
    // records that actual invocation and returns Busy without starting a child.
    assert!(matches!(
        fixture.app.spawn_data_relay(&fresh, true),
        Err(ApplicationError::Busy)
    ));
    assert_eq!(fixture.host.relay_calls.load(Ordering::SeqCst), 1);
    let state = fixture.host.state.lock().unwrap();
    assert_eq!(state.dp_relay_inputs.len(), 1);
    let (sandbox, vm, generation, control) = state.dp_relay_inputs[0];
    assert_eq!(vm, fresh.target.vm);
    assert_eq!(generation, fresh.target.generation);
    assert_eq!(state.dp_gate_calls[0], (sandbox, vm, generation));
    assert!(control);
    println!("DA_RECEIPT rollback old_revoked=true fresh_revoked=false old_host_calls=0 fresh_host_calls=1 exact_target=true");
}

#[test]
fn da_base_new_authorization_keeps_old_channel_until_last_receiver_drops() {
    let fixture = DataAuthorityPilotFixture::running();
    let old = fixture.revoke_with_rolled_back_timeout();
    let fresh = fixture.permit();
    assert!(*old.revocation.borrow());
    assert!(old.revocation.has_changed().is_ok(), "old sender must remain tracked");
    assert!(!*fresh.revocation.borrow());
    assert_eq!(
        fixture.app.data_authority_pilot_registration_snapshot(),
        (vec![(1, true), (1, false)], 1)
    );
    drop(old);
    assert_eq!(
        fixture.app.data_authority_pilot_registration_snapshot(),
        (vec![(0, true), (1, false)], 1)
    );
    let next = fixture.permit();
    assert!(!*fresh.revocation.borrow());
    assert!(!*next.revocation.borrow());
    assert_eq!(
        fixture.app.data_authority_pilot_registration_snapshot(),
        (vec![(2, false)], 1)
    );
    assert_eq!(fixture.host.relay_calls.load(Ordering::SeqCst), 0);
    println!("DA_RECEIPT channels old_live_kept=true old_zero_removed=true fresh_reused=true drains=1");
}

#[test]
fn da_base_unknown_but_durable_expiry_does_not_grant_a_new_permit() {
    let fixture = DataAuthorityPilotFixture::running();
    let old = fixture.permit();
    crate::controller::data_authority_pilot_fail_next_commit(true);
    assert_eq!(
        fixture.app.set_timeout("1", &fixture.session.sandbox_id, 0),
        Err(ApplicationError::OutcomeUnknown)
    );
    assert!(*old.revocation.borrow());
    assert!(matches!(
        fixture.app.data_permit(&fixture.session.envd_access_token, DataKind::Exec),
        Err(ApplicationError::NotFound)
    ));
    assert_eq!(
        fixture.app.data_authority_pilot_registration_snapshot(),
        (vec![(1, true)], 1)
    );
    assert_eq!(fixture.host.relay_calls.load(Ordering::SeqCst), 0);
    println!("DA_RECEIPT durable_unknown old_revoked=true fresh_refused=true host_calls=0");
}

#[test]
fn da_base_changed_permit_target_is_rejected_before_host_spawn() {
    for change_vm in [true, false] {
        let fixture = DataAuthorityPilotFixture::running();
        let mut permit = fixture.permit();
        assert!(!*permit.revocation.borrow());
        if change_vm {
            permit.target.vm = core::VmId(permit.target.vm.0 + 1);
        } else {
            permit.target.generation += 1;
        }
        assert!(matches!(
            fixture.app.spawn_data_relay(&permit, false),
            Err(ApplicationError::NotFound)
        ));
        assert_eq!(fixture.host.relay_calls.load(Ordering::SeqCst), 0);
        println!("DA_RECEIPT target changed_vm={change_vm} refused=true host_calls=0");
    }
}

#[test]
fn da_redispatch_rechecks_the_actual_host_token_digest() {
    let fixture = DataAuthorityPilotFixture::running();
    let permit = fixture.permit();
    assert!(!*permit.revocation.borrow());
    {
        let mut state = fixture.host.state.lock().unwrap();
        let token = state.secrets.get_mut(&permit.data_authority_pilot_sandbox().0).unwrap();
        let first = if token.starts_with('0') { "1" } else { "0" };
        token.replace_range(..1, first);
        assert_eq!(token.len(), 64);
        assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
    let result = fixture.app.spawn_data_relay(&permit, false);
    assert_eq!(fixture.host.relay_calls.load(Ordering::SeqCst), 0,
        "a permit whose original host token changed must not reach spawn_data_relay");
    assert!(matches!(result, Err(ApplicationError::Busy)));
    assert!(!*permit.revocation.borrow());
    println!("DA_RECEIPT credential valid_hex=true digest_changed=true revoked=false relay_spawn_calls=0");
}

#[test]
fn da_redispatch_rejects_malformed_host_tokens() {
    for token in ["0".repeat(63), "g".repeat(64), "é".repeat(32)] {
        let fixture = DataAuthorityPilotFixture::running();
        let permit = fixture.permit();
        let byte_count = token.len();
        let ascii_hex = token.bytes().all(|byte| byte.is_ascii_hexdigit());
        fixture.host.state.lock().unwrap().secrets.insert(permit.data_authority_pilot_sandbox().0, token);
        let result = fixture.app.spawn_data_relay(&permit, false);
        assert_eq!(fixture.host.relay_calls.load(Ordering::SeqCst), 0);
        assert!(matches!(result, Err(ApplicationError::Busy)));
        assert!(!*permit.revocation.borrow());
        println!("DA_RECEIPT token_shape byte_count={byte_count} ascii_hex={ascii_hex} relay_spawn_calls=0");
    }
}

#[test]
fn da_redispatch_rechecks_the_exact_deadline_without_a_scanner() {
    let fixture = DataAuthorityPilotFixture::running();
    let permit = fixture.permit();
    let deadline = fixture.app.get("1", &fixture.session.sandbox_id).unwrap().info.end_at_ms;
    assert!(deadline > 1000);
    fixture.host.clock.set(deadline - 1);
    assert!(matches!(fixture.app.spawn_data_relay(&permit, true), Err(ApplicationError::Busy)));
    assert_eq!(fixture.host.relay_calls.load(Ordering::SeqCst), 1);
    for now in [deadline, deadline + 1] {
        fixture.host.clock.set(now);
        assert!(!*permit.revocation.borrow());
        let result = fixture.app.spawn_data_relay(&permit, false);
        assert_eq!(fixture.host.relay_calls.load(Ordering::SeqCst), 1,
            "an expired permit must not reach spawn_data_relay even before a scanner revokes its watch");
        assert!(matches!(result, Err(ApplicationError::NotFound)));
        println!("DA_RECEIPT expiry at_deadline={} revoked=false relay_spawn_calls=1", now == deadline);
    }
}

#[test]
fn da_redispatch_refuses_an_invalid_service_clock() {
    let fixture = DataAuthorityPilotFixture::running();
    let permit = fixture.permit();
    fixture.host.clock.set(-1);
    let result = fixture.app.spawn_data_relay(&permit, false);
    assert_eq!(fixture.host.relay_calls.load(Ordering::SeqCst), 0);
    assert!(matches!(result, Err(ApplicationError::Invalid)));
    assert!(!*permit.revocation.borrow());
    println!("DA_RECEIPT invalid_clock revoked=false relay_spawn_calls=0");
}

#[test]
fn da_redispatch_rechecks_closed_effective_authority_for_every_kind() {
    for (kind, name) in [(DataKind::Exec, "exec"), (DataKind::FileRead, "read"), (DataKind::FileWrite, "write")] {
        let fixture = DataAuthorityPilotFixture::running();
        let permit = fixture.app.data_permit(&fixture.session.envd_access_token, kind).unwrap();
        let world = fixture.app.data_authority_pilot_close_effective(permit.data_authority_pilot_sandbox());
        let record = world.sandbox_record(permit.data_authority_pilot_sandbox()).unwrap();
        assert_eq!(record.phase, core::Phase::Running);
        assert_eq!(record.current_vm, Some(permit.target.vm));
        assert_eq!(record.generation, permit.target.generation);
        assert!(!*permit.revocation.borrow());
        let result = fixture.app.spawn_data_relay(&permit, false);
        assert_eq!(fixture.host.relay_calls.load(Ordering::SeqCst), 0,
            "closed effective authority must not reach the real relay dispatch interface");
        assert!(matches!(result, Err(ApplicationError::NotFound)));
        let actor = core::Actor::Tenant(record.owner);
        let input = match kind {
            DataKind::Exec => core::AccessInput::Exec(actor, permit.data_authority_pilot_sandbox(), permit.target.vm),
            DataKind::FileRead => core::AccessInput::FileRead(actor, permit.data_authority_pilot_sandbox(), permit.target.generation),
            DataKind::FileWrite => core::AccessInput::FileWrite(actor, permit.data_authority_pilot_sandbox(), permit.target.generation),
        };
        assert_eq!(core::handle_access(&world, input), Err(core::AuxiliaryReject::CapabilityDenied));
        println!("DA_RECEIPT access kind={name} running=true revoked=false target_exact=true effective_closed=true relay_spawn_calls=0");
    }
}
