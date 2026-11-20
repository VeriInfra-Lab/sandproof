
fn assert_source_flow_execution_binding(
    world: &core::World,
    expected: core::maintenance::slots::HostBinding,
) {
    let actual_binding = crate::controller::checkpoint::execution_binding(
        world, expected.sandbox, expected.generation,
    ).unwrap().unwrap();
    assert_eq!(actual_binding, expected);
    for generation in [0, expected.generation.checked_add(1).unwrap()] {
        assert!(matches!(crate::controller::checkpoint::execution_binding(
            world, expected.sandbox, generation,
        ), Err(JournalError::CorruptStore)));
    }
}

#[test]
fn proof_source_flow_original_request_reaches_same_source_question() {
    use crate::controller::lifecycle_inbox::readiness::snapshot::native_snapshot;
    let directory = tempfile::tempdir().unwrap();
    let (mut journal, state) = ready(&directory.path().join("source-flow.db"));
    let request = unload();
    let namespace = journal.namespace.clone();
    let active = journal.active_host_epoch.clone();
    let expected_source = state.lock().unwrap().source.clone().unwrap();
    let tx = transaction::WorldTransaction::begin(
        &mut journal.journal.connection,
        &journal.committed_world,
    ).unwrap();
    let inbox = crate::controller::lifecycle_inbox::codec::load(&tx).unwrap().unwrap();
    let sources = super::codec::load(&tx).unwrap().unwrap();
    let world = tx.world();
    let (session_index, entry_index) = inbox.find(&request).unwrap().unwrap();
    let selected = &inbox.sessions[session_index];
    let selected_state = inbox.state(session_index, entry_index);
    let local = selected_state.local.unwrap();
    let origin = selected.current_execution_origin().unwrap().intent.clone();
    let record = world.sandbox_record(local.sandbox).unwrap();
    assert_source_flow_execution_binding(&world, core::maintenance::slots::HostBinding {
        owner: record.owner, sandbox: local.sandbox, generation: record.generation,
        vm: record.current_vm.unwrap(), network: record.net_slot.unwrap(),
    });
    let expected_native = native_snapshot(&tx, &namespace, &world, local, record).unwrap();
    let expected_options = crate::controller::admission::read_create_options(
        &tx, &namespace, local.sandbox,
    ).unwrap();
    let expected_credential = crate::controller::data_credentials::snapshot(
        &tx, &world, &namespace, local.sandbox, true,
    ).unwrap().unwrap();
    let work = super::snapshot::current::<Evidence>(
        &tx, &namespace, &request, 10,
        super::snapshot::CurrentSource {
            world: &world, inbox: &inbox, limits: &sources.config, active: active.as_ref(),
        },
    ).unwrap();
    assert_eq!(work.anchor.request, request, "question must retain original full Unload");
    assert_eq!(work.anchor.execution_origin, origin);
    assert_eq!(work.anchor.local, local);
    assert_eq!(work.anchor.native, expected_native);
    assert_eq!(work.anchor.native.current_vm, record.current_vm);
    assert_eq!(work.anchor.native.generation, record.generation);
    assert_eq!(work.anchor.continued_from, None);
    assert_eq!(work.anchor.source, expected_source);
    assert_eq!(work.anchor.node, inbox.config.node);
    assert_eq!(work.anchor.host_bootstrap, inbox.host_epoch_observation.as_ref().unwrap().registration);
    assert_eq!(work.anchor.options_sha256, super::digest(&serde_json::to_vec(&expected_options).unwrap()));
    assert_eq!(work.anchor.credential, expected_credential);
    assert_eq!(work.anchor.credential.0, work.anchor.native.generation);
    assert_eq!(work.anchor.original_actions.len(), 5);
    assert_eq!(work.anchor.original_receipts.len(), 5);
    assert_eq!(work.requirement, SourceObservationRequirement {
        work_binding: super::digest(&serde_json::to_vec(&work.anchor).unwrap()),
        node: inbox.config.node.clone(),
        host_bootstrap: inbox.host_epoch_observation.as_ref().unwrap().registration.clone(),
        resource: work.anchor.original_actions[1].resource.clone(),
        source: expected_source,
        limits: sources.config,
    });
}
