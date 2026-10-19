//! Real authenticated RPC deadlines with explicit stalled receivers. Cursor
//! fairness is per artifact, shared by clones and never a cache-success fact.
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stalled_first_host_cannot_starve_later_host_or_another_artifact() {
    let fixture = engine::Engine::new();
    let (store, first) = store(&fixture);
    let second = second_binding(&first);
    let mut later = node();
    later.node_id = "later-host".try_into().unwrap();
    let authority = authority_with_nodes(
        &fixture,
        &store,
        &[first.clone(), second.clone()],
        &[node(), later.clone()],
    );
    let release = Arc::new(tokio::sync::Notify::new());
    // Separate TLS fixtures generate distinct registered leaf certificates.
    let stalled = CacheFixture::start_on(
        authority.clone(),
        Arc::clone(&store),
        None,
        node(),
        Some(Arc::clone(&release)),
    )
    .await;
    let healthy =
        CacheFixture::start_on(authority.clone(), Arc::clone(&store), None, later, None).await;
    let hosts = RegisteredHosts::new(
        vec![stalled.host.clone(), healthy.host.clone()],
        Handle::current(),
    )
    .unwrap();
    let candidates = ProtectedCandidates::new(authority.clone(), hosts, store);
    let scan = candidates.clone();
    tokio::task::spawn_blocking(move || scan.prepare_static(Duration::from_secs(1)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stalled.requests.lock().unwrap().len(), 2);
    assert!(healthy.requests.lock().unwrap().is_empty());
    assert!(
        authority
            .snapshot()
            .unwrap()
            .center
            .nodes()
            .iter()
            .all(|node| node.templates.is_empty())
    );

    // With two artifacts a single global cursor would restart both at the
    // same bad host every scan. Each artifact instead advances independently.
    let scan = candidates.clone();
    tokio::task::spawn_blocking(move || scan.prepare_static(Duration::from_secs(1)))
        .await
        .unwrap()
        .unwrap();
    let current = authority.snapshot().unwrap();
    assert!(current.center.nodes()[0].templates.is_empty());
    let cached = &current.center.nodes()[1].templates;
    assert_eq!(cached.len(), 2);
    for binding in [&first, &second] {
        assert!(cached.iter().any(|known| known.revision.as_str()
            == format!("{}-{}", binding.template_id, binding.revision)
            && known.digest == binding.canonical_digest().unwrap()));
    }
    assert_eq!(healthy.requests.lock().unwrap().len(), 2);
    // Unblock only this fixture's intercepted work before stopping listeners.
    release.notify_waiters();
    stalled.stop().await;
    healthy.stop().await;
}
