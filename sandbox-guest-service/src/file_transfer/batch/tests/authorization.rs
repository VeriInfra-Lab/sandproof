use super::*;
use crate::file_authorization::{Authorization, Error as AuthError, Operation};
use crate::file_requests::{
    FileRequests,
    tests::{full, limits as request_limits, released},
};
use crate::file_transfer::{AuthorizedUpload, UploadHttpError, UploadRequestError};
use axum::{body::Body, http::HeaderMap};
use http_body_util::BodyExt;

async fn authorize(fixture: &Fixture, op: Operation) -> (Authorization, FileRequests) {
    let mut headers = HeaderMap::new();
    headers.insert("x-access-token", "token".parse().unwrap());
    let mut limits = request_limits();
    limits.timeout = Duration::from_secs(10);
    let requests = FileRequests::new(limits).unwrap();
    let method = if op == Operation::Read {
        axum::http::Method::GET
    } else {
        axum::http::Method::POST
    };
    let auth = requests
        .admit()
        .unwrap()
        .authorize(&fixture.session, &method, &headers, b"path=~/bound")
        .await
        .unwrap();
    (auth, requests)
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
async fn upload_header_rejection_keeps_request_slot_through_last_response_clone() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let (auth, requests) = authorize(&fixture, Operation::Write).await;
    let mut req = request(auth);
    req.headers
        .insert("content-type", "text/plain".parse().unwrap());
    let response = fixture
        .streams
        .upload_authorized(
            req,
            &fixture.accounts,
            &fixture.settings,
            upload::settings(),
            response_limits(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let mut body = response.into_body();
    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let retained = bytes.clone();
    drop(bytes);
    drop(body);
    full(&requests);
    assert!(!root.path().join("bound").exists());
    drop(retained);
    released(&requests).await;
    multipart::quiesce(&fixture).await;
}

#[tokio::test]
async fn upload_only_consumes_write_capability_and_uses_its_immutable_path() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let (auth, requests) = authorize(&fixture, Operation::Read).await;
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
    drop(requests.admit().unwrap());
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
    assert!(!root.path().join("bound").exists());
    let (auth, requests) = authorize(&fixture, Operation::Write).await;
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
    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let retained = bytes.clone();
    drop(bytes);
    drop(body);
    full(&requests);
    drop(retained);
    released(&requests).await;
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
    let (auth, requests) = authorize(&fixture, Operation::Write).await;
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
    released(&requests).await;
    assert_eq!(std::fs::read(root.path().join("bound")).unwrap(), b"");
}
