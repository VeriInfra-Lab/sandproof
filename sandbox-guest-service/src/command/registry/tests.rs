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
    let mut reserved = registry
        .reserve(&ProcessConfig::default(), Some("unknown"))
        .unwrap();
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
        retired: Notify::new(),
        _slot: reserved.slot,
    });
    {
        let mut table = registry.inner.table.lock().unwrap();
        reserved.tag_claim.as_mut().unwrap().release(&mut table);
        table.records.insert(identity.pid, record.clone());
    }
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
    assert!(matches!(
        registry.live_tag("unknown"),
        Err(RegistryError::Unavailable)
    ));
    assert!(matches!(
        registry.connectable_tag("unknown"),
        Err(RegistryError::Unavailable)
    ));
    assert!(matches!(
        registry.reserve(&ProcessConfig::default(), Some("unknown")),
        Err(RegistryError::Unavailable)
    ));
    assert_eq!(registry.available_slots(), 0);
    assert!(matches!(
        registry.reserve(&ProcessConfig::default(), None),
        Err(RegistryError::ResourceExhausted)
    ));
    assert_eq!(registry.list_json(2).unwrap(), b"{}"); // not proof of no guest processes
}

#[test]
fn concurrent_tag_reservations_have_one_winner_and_release_without_history() {
    let registry = PidRegistry::new(
        binding(),
        1,
        RegistryLimits {
            records: 2,
            ..limits()
        },
    )
    .unwrap();
    let barrier = std::sync::Barrier::new(2);
    let reservations = std::thread::scope(|scope| {
        let attempts: Vec<_> = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    let result = registry.reserve(&ProcessConfig::default(), Some("shared"));
                    // Neither winner can release its claim before both have attempted.
                    barrier.wait();
                    result
                })
            })
            .collect();
        attempts
            .into_iter()
            .map(|attempt| attempt.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        reservations
            .iter()
            .filter(|attempt| attempt.is_ok())
            .count(),
        1
    );
    assert_eq!(
        reservations
            .iter()
            .filter(|attempt| matches!(attempt, Err(RegistryError::AlreadyExists)))
            .count(),
        1
    );
    assert_eq!(registry.available_slots(), 1);
    assert!(matches!(
        registry.live_tag("shared"),
        Err(RegistryError::NotReady)
    ));
    drop(reservations);
    assert_eq!(registry.available_slots(), 2);
    assert!(matches!(
        registry.connectable_tag("shared"),
        Err(RegistryError::NotFound)
    ));
    assert!(registry.inner.table.lock().unwrap().pending_tags.is_empty());
    drop(
        registry
            .reserve(&ProcessConfig::default(), Some("shared"))
            .unwrap(),
    );
    assert!(registry.inner.table.lock().unwrap().pending_tags.is_empty());
}

#[test]
fn empty_tag_is_present_untagged_reservations_do_not_conflict_and_limits_do_not_leak_tags() {
    let registry = PidRegistry::new(
        binding(),
        1,
        RegistryLimits {
            records: 3,
            ..limits()
        },
    )
    .unwrap();
    let config = ProcessConfig::default();
    assert!(matches!(
        registry.reserve(&config, Some(&"x".repeat(1024))),
        Err(RegistryError::ResourceExhausted)
    ));
    let empty = registry.reserve(&config, Some("")).unwrap();
    let first = registry.reserve(&config, None).unwrap();
    let second = registry.reserve(&config, None).unwrap();
    assert!(matches!(
        registry.reserve(&config, Some("")),
        Err(RegistryError::AlreadyExists)
    ));
    assert!(matches!(
        registry.reserve(&config, Some("later")),
        Err(RegistryError::ResourceExhausted)
    ));
    assert!(matches!(
        registry.connectable_tag("later"),
        Err(RegistryError::NotFound)
    ));
    assert!(matches!(
        registry.connectable_tag(""),
        Err(RegistryError::NotReady)
    ));
    assert_eq!(registry.list_json(2).unwrap(), b"{}");
    drop((empty, first, second));
    assert_eq!(registry.available_slots(), 3);
    assert!(registry.inner.table.lock().unwrap().pending_tags.is_empty());
}

#[test]
fn pending_tag_keeps_capacity_until_its_cleanup_even_if_the_publication_record_drops_first() {
    let registry = PidRegistry::new(binding(), 1, limits()).unwrap();
    let RegistryReservation {
        inner,
        metadata,
        slot,
        tag_claim,
    } = registry
        .reserve(&ProcessConfig::default(), Some("pending"))
        .unwrap();
    // A rejected publication can drop the would-be record before TagClaim's
    // destructor obtains the mutex. Its still-indexed tag must keep the permit.
    drop((inner, metadata, slot));
    assert_eq!(registry.available_slots(), 0);
    assert!(matches!(
        registry.connectable_tag("pending"),
        Err(RegistryError::NotReady)
    ));
    assert!(matches!(
        registry.reserve(&ProcessConfig::default(), Some("next")),
        Err(RegistryError::ResourceExhausted)
    ));
    drop(tag_claim);
    assert_eq!(registry.available_slots(), 1);
    assert!(matches!(
        registry.connectable_tag("pending"),
        Err(RegistryError::NotFound)
    ));
    assert!(registry.inner.table.lock().unwrap().pending_tags.is_empty());
}
