use super::*;
mod ownership;
use sandproof_guest_core::CommandIdentity;
use sha2::Sha512;
use std::{
    os::unix::fs::{MetadataExt, PermissionsExt},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Clone)]
struct Source {
    value: Arc<Mutex<(String, String, String)>>,
    calls: Arc<AtomicUsize>,
}
impl Source {
    fn new() -> Self {
        Self {
            value: Arc::new(Mutex::new((
                "vm-1".into(),
                "template-1".into(),
                "token".into(),
            ))),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
    fn set(&self, instance: &str, template: &str, token: &str) {
        *self.value.lock().unwrap() = (instance.into(), template.into(), token.into());
    }
}
impl MetadataSource for Source {
    async fn fetch(&self) -> Result<Metadata, ServiceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let value = self.value.lock().unwrap();
        Ok(Metadata {
            instance_id: value.0.clone(),
            template_id: value.1.clone(),
            access_token_sha512: Sha512::digest(value.2.as_bytes()).into(),
        })
    }
}
fn limits() -> Limits {
    Limits {
        identity_bytes: 128,
        timeout: Duration::from_secs(5),
    }
}
fn init_limits() -> InitLimits {
    InitLimits {
        body_bytes: 1024,
        token_bytes: 64,
        state_bytes: 1024,
        env_entries: 16,
        in_flight: 1,
        timeout: Duration::from_secs(5),
    }
}
fn identity() -> VmIdentity {
    VmIdentity::new("vm-1".into(), "template-1".into(), limits().identity_bytes).unwrap()
}
fn command() -> CommandIdentity {
    CommandIdentity {
        session: 1,
        command: 1,
        pid: 42,
        start_ticks: 10,
    }
}
fn root() -> (tempfile::TempDir, u32) {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let owner = root.path().metadata().unwrap().uid();
    (root, owner)
}
fn open_dir(root: &tempfile::TempDir) -> File {
    File::open(root.path()).unwrap()
}
fn provision(root: &tempfile::TempDir, owner: u32) {
    let mut state = PidClaims::create(open_dir(root), owner, identity().binding(), 127).unwrap();
    state.claim(command()).unwrap();
}
fn assert_error<M>(result: Result<Opened<M>, Error>, expected: Error) {
    assert_eq!(result.err(), Some(expected));
}

#[test]
fn vm_binding_has_unambiguous_fields_and_bounded_valid_identity() {
    let expected = Binding::new([
        221, 134, 102, 108, 146, 16, 70, 0, 204, 247, 173, 2, 59, 146, 244, 76, 206, 241, 96, 29,
        244, 39, 208, 164, 12, 9, 123, 183, 56, 194, 108, 135,
    ])
    .unwrap();
    assert!(
        identity().binding() == expected,
        "persistent v1 binding fixture"
    );
    let a = VmIdentity::new("ab".into(), "c".into(), 3).unwrap();
    let b = VmIdentity::new("a".into(), "bc".into(), 3).unwrap();
    assert!(a.binding() != b.binding());
    assert!(
        VmIdentity::new("vm-1".into(), "template-1".into(), 14)
            .unwrap()
            .binding()
            == identity().binding()
    );
    assert!(matches!(
        VmIdentity::new("vm-1".into(), "template-1".into(), 13),
        Err(Error::InvalidMetadata)
    ));
    for (instance, template) in [("", "x"), ("x", ""), ("x\0", "y"), ("x", "y\0")] {
        assert!(matches!(
            VmIdentity::new(instance.into(), template.into(), 128),
            Err(Error::InvalidMetadata)
        ));
    }
    for maximum in [0, usize::MAX] {
        assert!(matches!(
            VmIdentity::new("x".into(), "y".into(), maximum),
            Err(Error::InvalidConfiguration)
        ));
    }
}

#[tokio::test]
async fn normal_startup_reopens_original_pid_state_without_authentication_or_adoption() {
    let (root, owner) = root();
    provision(&root, owner);
    let before = std::fs::read(root.path().join("pid-claims.bin")).unwrap();
    let source = Source::new();
    let startup = Startup::new(source.clone(), limits()).unwrap();
    for _ in 0..2 {
        let opened = startup
            .open(open_dir(&root), owner, 127, init_limits())
            .await
            .unwrap();
        assert!(opened.binding() == identity().binding());
        let (session, mut claims) = opened.into_parts();
        assert!(matches!(
            session.authorize(Some("token")).await,
            Err(ServiceError::Unauthenticated)
        ));
        assert!(matches!(
            claims.claim(command()),
            Err(pid_claims::Error::Reused)
        ));
        assert_error(
            startup
                .open(open_dir(&root), owner, 127, init_limits())
                .await,
            Error::Storage(pid_claims::Error::Busy),
        );
        session
            .initialize(br#"{"accessToken":"token"}"#)
            .await
            .unwrap();
        drop(session.authorize(Some("token")).await.unwrap());
        drop(claims);
        drop(session);
        assert_eq!(
            std::fs::read(root.path().join("pid-claims.bin")).unwrap(),
            before
        );
    }
    assert_eq!(source.calls.load(Ordering::SeqCst), 6);
}

#[tokio::test]
async fn token_rotation_keeps_pid_binding_and_rejects_foreign_metadata() {
    let (root, owner) = root();
    provision(&root, owner);
    let source = Source::new();
    let startup = Startup::new(source.clone(), limits()).unwrap();
    let (session, mut claims) = startup
        .open(open_dir(&root), owner, 127, init_limits())
        .await
        .unwrap()
        .into_parts();
    session
        .initialize(br#"{"accessToken":"token"}"#)
        .await
        .unwrap();
    for (instance, template) in [("vm-2", "template-1"), ("vm-1", "template-2")] {
        source.set(instance, template, "rotated");
        assert_eq!(
            session.initialize(br#"{"accessToken":"rotated"}"#).await,
            Err(ServiceError::Unavailable)
        );
        drop(session.authorize(Some("token")).await.unwrap());
        assert!(matches!(
            session.authorize(Some("rotated")).await,
            Err(ServiceError::Unauthenticated)
        ));
    }
    source.set("vm-1", "template-1", "rotated");
    session
        .initialize(br#"{"accessToken":"rotated"}"#)
        .await
        .unwrap();
    drop(session.authorize(Some("rotated")).await.unwrap());
    assert!(matches!(
        session.authorize(Some("token")).await,
        Err(ServiceError::Unauthenticated)
    ));
    assert!(claims.binding() == identity().binding());
    assert!(matches!(
        claims.claim(command()),
        Err(pid_claims::Error::Reused)
    ));
}

#[tokio::test]
async fn missing_corrupt_and_foreign_state_never_gets_created_reset_or_migrated() {
    let (root, owner) = root();
    let source = Source::new();
    let startup = Startup::new(source.clone(), limits()).unwrap();
    assert_error(
        startup
            .open(open_dir(&root), owner, 127, init_limits())
            .await,
        Error::Storage(pid_claims::Error::Missing),
    );
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    provision(&root, owner);
    let path = root.path().join("pid-claims.bin");
    let original = std::fs::read(&path).unwrap();
    source.set("foreign", "template-1", "token");
    assert_error(
        startup
            .open(open_dir(&root), owner, 127, init_limits())
            .await,
        Error::Storage(pid_claims::Error::BindingMismatch),
    );
    source.set("vm-1", "template-1", "token");
    assert_error(
        startup
            .open(open_dir(&root), owner, 126, init_limits())
            .await,
        Error::Storage(pid_claims::Error::RangeMismatch),
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let mut corrupt = original;
    *corrupt.last_mut().unwrap() ^= 1;
    std::fs::write(&path, &corrupt).unwrap();
    assert_error(
        startup
            .open(open_dir(&root), owner, 127, init_limits())
            .await,
        Error::Storage(pid_claims::Error::Corrupt),
    );
    assert_eq!(std::fs::read(path).unwrap(), corrupt);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[tokio::test]
async fn invalid_configuration_precedes_metadata_and_invalid_identity_precedes_storage() {
    let (root, owner) = root();
    let source = Source::new();
    for bad in [
        Limits {
            identity_bytes: 0,
            ..limits()
        },
        Limits {
            identity_bytes: usize::MAX,
            ..limits()
        },
        Limits {
            timeout: Duration::ZERO,
            ..limits()
        },
        Limits {
            timeout: Duration::MAX,
            ..limits()
        },
    ] {
        assert!(matches!(
            Startup::new(source.clone(), bad),
            Err(Error::InvalidConfiguration)
        ));
    }
    let startup = Startup::new(source.clone(), limits()).unwrap();
    for max_pid in [0, pid_claims::MAX_PID + 1] {
        assert_error(
            startup
                .open(open_dir(&root), owner, max_pid, init_limits())
                .await,
            Error::InvalidConfiguration,
        );
    }
    assert_error(
        startup
            .open(
                open_dir(&root),
                owner,
                127,
                InitLimits {
                    body_bytes: 0,
                    ..init_limits()
                },
            )
            .await,
        Error::InvalidConfiguration,
    );
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
    for (instance, template) in [("", "t"), ("v", ""), ("v\0", "t"), ("v", "t\0")] {
        source.set(instance, template, "token");
        assert_error(
            startup
                .open(open_dir(&root), owner, 127, init_limits())
                .await,
            Error::InvalidMetadata,
        );
    }
    source.set(&"v".repeat(129), "t", "token");
    assert_error(
        startup
            .open(open_dir(&root), owner, 127, init_limits())
            .await,
        Error::InvalidMetadata,
    );
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

struct Stalled;
impl MetadataSource for Stalled {
    async fn fetch(&self) -> Result<Metadata, ServiceError> {
        std::future::pending().await
    }
}

#[tokio::test(start_paused = true)]
async fn metadata_wait_is_bounded_and_shared_startup_does_not_queue() {
    let (root, owner) = root();
    let startup = Startup::new(Stalled, limits()).unwrap();
    let first = startup.open(open_dir(&root), owner, 127, init_limits());
    tokio::pin!(first);
    assert!(poll_once(first.as_mut()).await.is_pending());
    assert_error(
        startup
            .clone()
            .open(open_dir(&root), owner, 127, init_limits())
            .await,
        Error::Busy,
    );
    tokio::time::advance(Duration::from_secs(6)).await;
    assert_error(first.await, Error::DeadlineExceeded);
    assert_eq!(startup.slot.available_permits(), 1);
    let mut canceled = Box::pin(startup.open(open_dir(&root), owner, 127, init_limits()));
    assert!(poll_once(canceled.as_mut()).await.is_pending());
    assert_eq!(startup.slot.available_permits(), 0);
    drop(canceled);
    assert_eq!(startup.slot.available_permits(), 1);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

async fn poll_once<F: std::future::Future>(
    mut future: std::pin::Pin<&mut F>,
) -> std::task::Poll<F::Output> {
    std::future::poll_fn(|cx| std::task::Poll::Ready(future.as_mut().poll(cx))).await
}
