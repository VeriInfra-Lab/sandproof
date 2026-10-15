use super::*;
use crate::{
    Metadata, MetadataSource, ServiceError,
    startup::{Startup, VmIdentity},
};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use futures_util::StreamExt;
use sandproof_guest_core::CommandIdentity;
use sandproof_guest_runtime::{
    file_download::local_zone::Snapshot,
    file_mime::Catalog,
    pid_claims::{self, PidClaims},
};
use sha2::{Digest, Sha512};
use std::{fs::File, os::unix::fs::PermissionsExt};

struct MetadataFixture;
impl MetadataSource for MetadataFixture {
    async fn fetch(&self) -> Result<Metadata, ServiceError> {
        Ok(Metadata {
            instance_id: "vm".into(),
            template_id: "template".into(),
            access_token_sha512: Sha512::digest(b"token").into(),
        })
    }
}
fn request(path: &str, method: &str, body: &'static str, token: &'static str) -> Request<Body> {
    Request::builder()
        .uri(path)
        .method(method)
        .header("x-access-token", token)
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap()
}
async fn consume(response: axum::response::Response) -> (StatusCode, Vec<u8>) {
    let status = response.status();
    let mut stream = response.into_body().into_data_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        bytes.extend_from_slice(&chunk.unwrap());
        assert!(bytes.len() <= 16384);
    }
    (status, bytes)
}

#[tokio::test]
async fn startup_identity_and_original_handlers_share_one_uninitialized_session() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let owner = sandproof_guest_runtime::startup_files::current_owner();
    let config = decode(&value());
    let startup = Startup::new(MetadataFixture, config.startup).unwrap();
    let identity = VmIdentity::new(
        "vm".into(),
        "template".into(),
        config.startup.identity_bytes,
    )
    .unwrap();
    let command = CommandIdentity {
        session: 1,
        command: 1,
        pid: 42,
        start_ticks: 10,
    };
    let maximum = config.paths.max_pid;
    let mut state = PidClaims::create(
        File::open(root.path()).unwrap(),
        owner,
        identity.binding(),
        maximum,
    )
    .unwrap();
    state.claim(command).unwrap();
    drop(state);
    let opened = startup
        .open(
            File::open(root.path()).unwrap(),
            owner,
            maximum,
            config.initialization,
        )
        .await
        .unwrap();
    assert_eq!(opened.service_session(), 1);
    let mime = Catalog::from_sources([None; 2], [None; 4], config.snapshots.mime).unwrap();
    let material = crate::file_download::tests::reference::timezone::fixed_material();
    let zone = Snapshot::from_tzif(&material, config.snapshots.timezone_limits.snapshot).unwrap();
    let (http, _) = Prepared::new(config)
        .unwrap()
        .assemble(opened, mime, zone)
        .unwrap();
    assert_uninitialized(&http).await;
    assert_eq!(
        consume(
            http.handle(request("/init", "POST", r#"{"accessToken":"token"}"#, ""))
                .await
                .unwrap()
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let (status, bytes) = consume(
        http.handle(request("/process.Process/List", "POST", "{}", "token"))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), json!({}));
    assert_eq!(
        consume(
            http.handle(request("/process.Process/List", "POST", "{}", "wrong"))
                .await
                .unwrap()
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    drop(http);
    let mut reopened = PidClaims::open(
        File::open(root.path()).unwrap(),
        owner,
        identity.binding(),
        maximum,
    )
    .unwrap();
    assert_eq!(reopened.allocate_session().unwrap(), 2);
    assert!(matches!(
        reopened.claim(command),
        Err(pid_claims::Error::Reused)
    ));
}

async fn assert_uninitialized(http: &crate::daemon::assembly::Http<MetadataFixture>) {
    assert_eq!(
        consume(
            http.handle(request("/health", "GET", "", ""))
                .await
                .unwrap()
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    for (path, method, body) in [
        ("/process.Process/List", "POST", "{}"),
        ("/filesystem.Filesystem/Stat", "POST", r#"{"path":"/data"}"#),
        ("/files?path=/data", "GET", ""),
    ] {
        assert_eq!(
            consume(
                http.handle(request(path, method, body, "token"))
                    .await
                    .unwrap()
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
}
