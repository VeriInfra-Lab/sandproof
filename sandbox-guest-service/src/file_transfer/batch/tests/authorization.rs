use super::*;
use crate::file_authorization::{
    Authorization, Error as AuthError, Limits as AuthLimits, Operation,
};
use crate::file_transfer::{AuthorizedUpload, UploadHttpError, UploadRequestError};
use axum::{body::Body, http::HeaderMap};
use http_body_util::BodyExt;
use sandproof_guest_runtime::file_query::Query;

async fn authorize(fixture: &Fixture, op: Operation) -> Authorization {
    let mut headers = HeaderMap::new();
    headers.insert("x-access-token", "token".parse().unwrap());
    fixture
        .session
        .authorize_file(
            &headers,
            Query {
                path: Some(b"~/bound".to_vec()),
                username: None,
                signature: None,
                signature_expiration: None,
            },
            op,
            AuthLimits {
                headers: crate::file_upload::HeaderLimits {
                    bytes: 4096,
                    fields: 16,
                },
                query_bytes: 4096,
            },
            tokio::time::Instant::now() + Duration::from_secs(10),
        )
        .await
        .unwrap()
}
fn request(auth: Authorization) -> AuthorizedUpload {
    let req = upload::request(b"written", false, "application/octet-stream", None);
    AuthorizedUpload {
        authorization: auth,
        headers: req.headers,
        body: req.body,
        expected: req.expected,
    }
}
fn response_limits() -> crate::upload_response::Limits {
    crate::upload_response::Limits {
        wire_bytes: 4096,
        chunk_bytes: 4096,
    }
}

#[tokio::test]
async fn upload_only_consumes_write_capability_and_uses_its_immutable_path() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let auth = authorize(&fixture, Operation::Read).await;
    assert!(matches!(
        fixture
            .streams
            .upload_authorized(
                request(auth),
                &fixture.accounts,
                &fixture.settings,
                upload::settings(),
                response_limits()
            )
            .await,
        Err(UploadHttpError::Request(UploadRequestError::Authorization(
            AuthError::WrongOperation
        )))
    ));
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
    assert!(!root.path().join("bound").exists());
    let auth = authorize(&fixture, Operation::Write).await;
    let mut body = fixture
        .streams
        .upload_authorized(
            request(auth),
            &fixture.accounts,
            &fixture.settings,
            upload::settings(),
            response_limits(),
        )
        .await
        .unwrap()
        .into_body();
    while let Some(frame) = body.frame().await {
        drop(frame.unwrap());
    }
    assert_eq!(
        std::fs::read(root.path().join("bound")).unwrap(),
        b"written"
    );
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    multipart::quiesce(&fixture).await;
}

#[tokio::test]
async fn upload_body_wait_cannot_restart_authenticated_request_deadline() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let auth = authorize(&fixture, Operation::Write).await;
    let mut req = request(auth);
    req.expected = None;
    let started = Arc::new(tokio::sync::Notify::new());
    let signal = Arc::clone(&started);
    req.body = Body::from_stream(futures_util::stream::poll_fn(move |_| {
        signal.notify_one();
        std::task::Poll::Pending::<Option<Result<bytes::Bytes, std::io::Error>>>
    }));
    let mut transfer = Box::pin(fixture.streams.upload_authorized(
        req,
        &fixture.accounts,
        &fixture.settings,
        upload::settings(),
        response_limits(),
    ));
    tokio::select! {()=started.notified()=>{}, result=&mut transfer=>panic!("body did not wait: {:?}",result.err())}
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(11)).await;
    assert!(matches!(
        transfer.await,
        Err(UploadHttpError::Request(UploadRequestError::Body(
            UploadBodyError::Transfer(FileStreamError::DeadlineExceeded)
        )))
    ));
    tokio::time::resume();
    multipart::quiesce(&fixture).await;
    assert_eq!(std::fs::read(root.path().join("bound")).unwrap(), b"");
}
