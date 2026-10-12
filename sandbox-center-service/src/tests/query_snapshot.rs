//! Queries use the bindings in their ONE confirmed view, never another
//! provider read or an alias fallback. Real Redis revision checks are separate.
use super::*;

#[test]
fn queries_reuse_one_projection_without_per_instance_template_reads() {
    let f = fixture();
    let first = f.app.create("1", command()).unwrap();
    f.app.create("1", command()).unwrap();
    f.app.create("2", command()).unwrap();
    let before = f.app.authority.lock().unwrap().projection_reads;
    let page = f
        .app
        .list(
            "1",
            ListQuery {
                limit: Some(1),
                ..ListQuery::default()
            }
            .validate()
            .unwrap()
            .application,
        )
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert!(page.next_cursor.is_some());
    assert_eq!(f.app.authority.lock().unwrap().projection_reads - before, 1);
    assert_eq!(f.app.templates.bound_reads.load(Ordering::SeqCst), 0);
    let before = f.app.authority.lock().unwrap().projection_reads;
    assert_eq!(
        f.app.get("1", &first.sandbox_id).unwrap().info.template_id,
        "base"
    );
    assert_eq!(f.app.authority.lock().unwrap().projection_reads - before, 1);
    assert_eq!(f.app.get("2", &first.sandbox_id), Err(PortError::NotFound));
    assert_eq!(f.app.templates.bound_reads.load(Ordering::SeqCst), 0);
}

#[test]
fn missing_snapshot_binding_cannot_fall_back_to_a_healthy_provider() {
    let f = fixture();
    let session = f.app.create("1", command()).unwrap();
    f.app.authority.lock().unwrap().query_templates.clear();
    // The provider still knows the binding, but querying it would mix views.
    assert!(
        f.app
            .templates
            .bound(&template().revision, &template().digest)
            .is_ok()
    );
    assert_eq!(
        f.app.get("1", &session.sandbox_id),
        Err(PortError::NotFound)
    );
    assert!(
        f.app
            .list("1", ListQuery::default().validate().unwrap().application)
            .is_err()
    );
    assert_eq!(f.app.templates.bound_reads.load(Ordering::SeqCst), 1);
}

#[test]
fn snapshot_template_still_checks_the_original_resource_shape() {
    let f = fixture();
    let session = f.app.create("1", command()).unwrap();
    f.app.authority.lock().unwrap().query_templates[0]
        .resources
        .cpu_count = 2;
    assert_eq!(f.app.get("1", &session.sandbox_id), Err(PortError::Busy));
    assert_eq!(f.app.templates.bound_reads.load(Ordering::SeqCst), 0);
}
