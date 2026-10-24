//! Two actual fixed Redis children: the shared password and PONG are not identity.
use super::{Engine, connect, engine::FixtureIdentityError};

#[test]
fn readiness_matches_the_original_live_redis_child_pid() {
    let mut fixture = Engine::new();
    let original = fixture.child.as_ref().unwrap().id();
    let mut connection = connect(fixture.address);
    assert_eq!(fixture.verify_ready_identity(&mut connection), Ok(()));
    assert_eq!(fixture.child.as_ref().unwrap().id(), original);
    assert!(
        fixture
            .child
            .as_mut()
            .unwrap()
            .try_wait()
            .unwrap()
            .is_none()
    );
}

#[test]
fn another_real_redis_with_the_same_password_and_pong_is_rejected() {
    let mut original = Engine::new();
    let mut other = Engine::new();
    let expected = original.child.as_ref().unwrap().id();
    let observed = other.child.as_ref().unwrap().id();
    assert_ne!(expected, observed);
    let mut connection = connect(other.address);
    assert_eq!(
        redis::cmd("PING").query::<String>(&mut connection).unwrap(),
        "PONG"
    );
    assert_eq!(
        original.verify_ready_identity(&mut connection),
        Err(FixtureIdentityError::WrongProcess { expected, observed })
    );
    assert!(
        original
            .child
            .as_mut()
            .unwrap()
            .try_wait()
            .unwrap()
            .is_none()
    );
    assert!(other.child.as_mut().unwrap().try_wait().unwrap().is_none());
}
