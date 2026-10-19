//! Controlled SDK fixture only. The preselected worker is wrapped in the real
//! execution owner and async body; no public Start admission/registry is claimed.
use super::*;
use sandproof_guest_service::command::{
    CommandBodyLimits, CommandOwners, ExecutionDeadline, ManagedCommand, OwnerLimits,
};
use sandproof_guest_service::{InitLimits, Metadata, MetadataSource, ServiceError, Session};
use sha2::{Digest, Sha512};
use tokio::time::Instant;

struct Source;
impl MetadataSource for Source {
    async fn fetch(&self) -> Result<Metadata, ServiceError> {
        Ok(Metadata {
            instance_id: "fixture-vm".into(),
            template_id: "fixture-template".into(),
            access_token_sha512: Sha512::digest(b"fixture-token").into(),
        })
    }
}
struct Stop(ManagedCommand);
impl Drop for Stop {
    fn drop(&mut self) {
        let _ = self.0.signal(self.0.identity(), ProcessSignal::Kill);
    }
}

pub(super) async fn settled(fixture: &Fixture) {
    assert_eq!(
        fixture.body_requests.available_permits(),
        1,
        "HTTP bytes retained the original response permit"
    );
    timeout(Duration::from_secs(5), async {
        while !fixture.completed.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("independent command observer did not settle");
}

pub(super) async fn response(process: CommandProcess, fixture: &Fixture) -> Response {
    let session = Session::new(
        Source,
        InitLimits {
            body_bytes: 1024,
            token_bytes: 128,
            state_bytes: 2048,
            env_entries: 16,
            in_flight: 1,
            timeout: Duration::from_secs(5),
        },
    )
    .unwrap();
    session
        .initialize(br#"{"accessToken":"fixture-token"}"#)
        .await
        .unwrap();
    let authorization = session.authorize(Some("fixture-token")).await.unwrap();
    let owners = CommandOwners::new(OwnerLimits {
        records: 1,
        subscribers: 1,
        queued_chunks: 8,
        output_bytes: 8192,
        input_chunk_bytes: 1024,
    })
    .unwrap();
    let (command, subscription) = owners.reserve().unwrap().start_registered(
        process,
        authorization,
        ExecutionDeadline::from_duration(Instant::now(), Duration::from_secs(10)).unwrap(),
    );
    let command = Stop(command);
    command.0.started().await.unwrap();
    let completed = Arc::clone(&fixture.completed);
    // Independent owner records actual wait/EOF even after the SDK disconnects.
    tokio::spawn(async move {
        assert_eq!(command.0.wait().await.unwrap().code, 17);
        completed.store(true, Ordering::SeqCst);
    });
    let request = Arc::new(fixture.body_requests.clone().try_acquire_owned().unwrap());
    let body = subscription
        .into_body(
            request,
            CommandBodyLimits {
                message_bytes: LIMIT,
                keepalive: Duration::from_secs(90),
            },
            Instant::now() + Duration::from_secs(10),
        )
        .unwrap();
    ([("content-type", "application/connect+json")], body).into_response()
}
