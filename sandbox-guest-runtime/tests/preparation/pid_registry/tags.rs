use super::*;
use sandproof_guest_service::command::CommandSubscription;

async fn start(
    fixture: &Fixture,
    registry: &PidRegistry,
    registrar: &PidRegistrar,
    tag: &str,
    number: u64,
) -> (KernelGuard, Stop, CommandSubscription) {
    let reservation = registry.reserve(&config(fixture), Some(tag)).unwrap();
    let mut spec = fixture.spec();
    spec.command = number;
    let ticket = fixture
        .pending(spec)
        .await
        .register_pid(registrar.clone(), binding())
        .await
        .unwrap();
    let kernel = kernel_guard(ticket.identity());
    let (command, output) = reservation
        .publish(ticket)
        .unwrap()
        .started()
        .await
        .unwrap();
    (kernel, Stop(command), output)
}

async fn available(registry: &PidRegistry, expected: usize) {
    timeout(Duration::from_secs(5), async {
        while registry.available_slots() != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("original registry owners did not retire");
}

async fn expire_tag(registry: &PidRegistry, command: &ManagedCommand, tag: &str) {
    let completed = command.status().completed_at.unwrap();
    tokio::time::pause();
    tokio::time::advance(
        (completed + Duration::from_secs(29)).saturating_duration_since(Instant::now()),
    )
    .await;
    assert_eq!(
        registry.connectable_tag(tag).unwrap().identity(),
        command.identity()
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(matches!(
        registry.connectable_tag(tag),
        Err(RegistryError::NotFound)
    ));
    tokio::time::resume();
}

fn duplicate_has_no_effect(registry: &PidRegistry, fixture: &Fixture, tag: &str) {
    assert!(matches!(
        registry.reserve(&config(fixture), Some(tag)),
        Err(RegistryError::AlreadyExists)
    ));
    fixture.assert_no_effect();
    assert_eq!(fixture.gate_count(), 0);
}

#[tokio::test]
async fn unique_tags_ignore_other_entries_and_reuse_retires_only_the_original_terminal_record() {
    let first = Fixture::new().await;
    let other = Fixture::new().await;
    let next = Fixture::new().await;
    let registry = PidRegistry::new(
        binding(),
        1,
        RegistryLimits {
            records: 3,
            metadata_bytes: 4096,
            retention: Duration::from_secs(30),
        },
    )
    .unwrap();
    let registrar = PidRegistrar::new(store(&first));
    let (_first_kernel, first_command, first_output) =
        start(&first, &registry, &registrar, "same", 1).await;
    let (_other_kernel, other_command, other_output) =
        start(&other, &registry, &registrar, "other", 2).await;
    let original = first_command.0.identity();
    let other_identity = other_command.0.identity();
    for (tag, identity) in [("same", original), ("other", other_identity)] {
        assert_eq!(registry.live_tag(tag).unwrap().identity(), identity);
        assert_eq!(registry.connectable_tag(tag).unwrap().identity(), identity);
    }
    assert!(matches!(
        registry.live_tag("absent"),
        Err(RegistryError::NotFound)
    ));
    duplicate_has_no_effect(&registry, &next, "same");
    assert_eq!(registry.available_slots(), 1);

    first_command.0.close_input(original).unwrap();
    assert_eq!(first_command.0.wait().await.unwrap().code, 0);
    drop(first_output);
    assert!(matches!(
        registry.live_tag("same"),
        Err(RegistryError::NotFound)
    ));
    assert_eq!(
        registry.connectable_tag("same").unwrap().identity(),
        original
    );
    let cancelled = registry.reserve(&config(&next), Some("same")).unwrap();
    assert_eq!(
        registry.connectable_tag("same").unwrap().identity(),
        original
    );
    drop(cancelled);
    assert_eq!(
        registry.connectable(original.pid).unwrap().identity(),
        original
    );

    let (_next_kernel, next_command, next_output) =
        start(&next, &registry, &registrar, "same", 3).await;
    let replacement = next_command.0.identity();
    assert_ne!(replacement.pid, original.pid);
    assert_eq!(registry.live_tag("same").unwrap().identity(), replacement);
    assert_eq!(
        registry.live_tag("other").unwrap().identity(),
        other_identity
    );
    assert!(matches!(
        registry.connectable(original.pid),
        Err(RegistryError::NotFound)
    ));
    available(&registry, 1).await; // wake the old timer without waiting 30 seconds
    assert!(matches!(
        first.owners.reserve(),
        Err(CommandError::ResourceExhausted)
    ));
    assert_eq!(first_command.0.wait().await.unwrap().code, 0); // old handle retains only the old process
    drop(first_command);
    capacity_restored(&first.owners).await;

    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(30)).await;
    assert_eq!(registry.live_tag("same").unwrap().identity(), replacement);
    assert_eq!(
        registry.live_tag("other").unwrap().identity(),
        other_identity
    );
    tokio::time::resume();
    other_command.0.close_input(other_identity).unwrap();
    next_command.0.close_input(replacement).unwrap();
    assert_eq!(other_command.0.wait().await.unwrap().code, 0);
    assert_eq!(next_command.0.wait().await.unwrap().code, 0);
    drop((other_output, next_output));
    expire_tag(&registry, &next_command.0, "same").await;
    available(&registry, 3).await;
    drop((other_command, next_command));
    capacity_restored(&other.owners).await;
    capacity_restored(&next.owners).await;
    for fixture in [&first, &other, &next] {
        assert_eq!(fixture.gate_count(), 0);
        assert_eq!(
            std::fs::read(fixture.directory.path().join("effect")).unwrap(),
            b"once"
        );
    }
}

#[tokio::test]
async fn rejected_publication_releases_tag_without_clearing_the_retained_command() {
    let first = Fixture::new().await;
    let rejected = Fixture::new().await;
    let registry = PidRegistry::new(
        binding(),
        1,
        RegistryLimits {
            records: 2,
            metadata_bytes: 4096,
            retention: Duration::from_secs(30),
        },
    )
    .unwrap();
    let registrar = PidRegistrar::new(store(&first));
    let (_kernel, command, output) = start(&first, &registry, &registrar, "", 1).await;
    let original = command.0.identity();
    command.0.close_input(original).unwrap();
    assert_eq!(command.0.wait().await.unwrap().code, 0);
    drop(output);
    let reservation = registry.reserve(&config(&rejected), Some("")).unwrap();
    let mut wrong = rejected.spec();
    wrong.session = 2;
    let ticket = rejected
        .pending(wrong)
        .await
        .register_pid(registrar, binding())
        .await
        .unwrap();
    let _rejected_kernel = kernel_guard(ticket.identity());
    let mut observer = ticket.observer();
    assert!(matches!(
        reservation.publish(ticket),
        Err(RegistryError::StaleIdentity)
    ));
    assert_eq!(stopped(&mut observer).await, CommandError::Cancelled);
    capacity_restored(&rejected.owners).await;
    assert_eq!(registry.connectable_tag("").unwrap().identity(), original);
    drop(registry.reserve(&config(&rejected), Some("")).unwrap());
    assert_eq!(registry.available_slots(), 1);
    rejected.assert_no_effect();
    assert_eq!(rejected.gate_count(), 0);
    expire_retention(
        &registry,
        original,
        command.0.status().completed_at.unwrap(),
    )
    .await;
    available(&registry, 2).await;
    drop(command);
    capacity_restored(&first.owners).await;
}
