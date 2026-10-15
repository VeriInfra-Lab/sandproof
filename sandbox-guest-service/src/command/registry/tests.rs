use super::*;

fn limits() -> RegistryLimits {
    RegistryLimits {
        records: 1,
        metadata_bytes: 1024,
        retention: Duration::from_secs(30),
    }
}
fn binding() -> Binding {
    Binding::new([1; 32]).unwrap()
}

#[test]
fn absent_process_metadata_stays_absent_and_uses_its_own_wire_budget() {
    let entries = [ProcessInfo {
        config: None,
        pid: u32::MAX,
        tag: None,
    }];
    let budget = encode_response(Response::List(&entries), 1024)
        .unwrap()
        .len();
    let registry = PidRegistry::new(
        binding(),
        1,
        RegistryLimits {
            metadata_bytes: budget,
            ..limits()
        },
    )
    .unwrap();
    assert!(matches!(
        registry.reserve(&ProcessConfig::default(), None),
        Err(RegistryError::ResourceExhausted)
    ));
    let reservation = registry.reserve_optional(None, None).unwrap();
    assert!(reservation.metadata.config.is_none());
    assert_eq!(registry.available_slots(), 0);
    drop(reservation);
    assert_eq!(registry.available_slots(), 1);
}

#[test]
fn invalid_configuration_and_oversized_metadata_never_consume_capacity() {
    assert!(matches!(
        PidRegistry::new(binding(), 0, limits()),
        Err(RegistryError::InvalidArgument)
    ));
    for bad in [
        RegistryLimits {
            records: 0,
            ..limits()
        },
        RegistryLimits {
            records: usize::MAX,
            ..limits()
        },
        RegistryLimits {
            metadata_bytes: 0,
            ..limits()
        },
        RegistryLimits {
            metadata_bytes: usize::MAX,
            ..limits()
        },
        RegistryLimits {
            retention: Duration::ZERO,
            ..limits()
        },
        RegistryLimits {
            retention: Duration::MAX,
            ..limits()
        },
    ] {
        assert!(matches!(
            PidRegistry::new(binding(), 1, bad),
            Err(RegistryError::InvalidArgument)
        ));
    }
    let registry = PidRegistry::new(binding(), 1, limits()).unwrap();
    let config = ProcessConfig::default();
    for oversized in [
        ProcessConfig {
            args: vec![String::new(); 1024],
            ..config.clone()
        },
        ProcessConfig {
            cwd: Some("\0".repeat(200)),
            ..config.clone()
        },
    ] {
        assert!(matches!(
            registry.reserve(&oversized, None),
            Err(RegistryError::ResourceExhausted)
        ));
        assert_eq!(registry.available_slots(), 1);
    }
    assert!(matches!(
        registry.reserve(&config, Some(&"x".repeat(1024))),
        Err(RegistryError::ResourceExhausted)
    ));
    assert_eq!(registry.available_slots(), 1);
    let reserved = registry.reserve(&config, None).unwrap();
    assert_eq!(registry.available_slots(), 0);
    assert!(matches!(
        registry.reserve(&config, None),
        Err(RegistryError::ResourceExhausted)
    ));
    assert_eq!(registry.list_json(2).unwrap(), b"{}");
    drop(reserved);
    assert_eq!(registry.available_slots(), 1);
}

#[test]
fn unpolled_registry_owner_loss_keeps_original_record_unavailable_and_charged() {
    let registry = PidRegistry::new(binding(), 1, limits()).unwrap();
    let reserved = registry.reserve(&ProcessConfig::default(), None).unwrap();
    let identity = CommandIdentity {
        session: 1,
        command: 2,
        pid: 3,
        start_ticks: 4,
    };
    let record = Arc::new(Record {
        identity,
        metadata: reserved.metadata,
        state: Mutex::new(State::default()),
        _slot: reserved.slot,
    });
    registry
        .inner
        .records
        .lock()
        .unwrap()
        .insert(identity.pid, record.clone());
    assert!(matches!(
        registry.live(identity.pid),
        Err(RegistryError::NotReady)
    ));
    // This guard is constructed before spawning the registry task. Aborting
    // even an unpolled future must not delete a potentially dispatched record.
    drop(Guard {
        inner: reserved.inner,
        record,
        removed: false,
    });
    assert!(matches!(
        registry.live(identity.pid),
        Err(RegistryError::Unavailable)
    ));
    assert!(matches!(
        registry.connectable(identity.pid),
        Err(RegistryError::Unavailable)
    ));
    assert_eq!(registry.available_slots(), 0);
    assert!(matches!(
        registry.reserve(&ProcessConfig::default(), None),
        Err(RegistryError::ResourceExhausted)
    ));
    assert_eq!(registry.list_json(2).unwrap(), b"{}"); // not proof of no guest processes
}
