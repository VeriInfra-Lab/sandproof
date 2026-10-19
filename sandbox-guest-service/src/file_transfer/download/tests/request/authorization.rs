use super::*;
use crate::file_authorization::{Error as AuthError, Limits as AuthLimits, Operation};
use crate::file_requests::{
    FileRequests,
    tests::{full, limits as request_limits, released},
};
use sandproof_guest_runtime::file_query::Query;

#[tokio::test]
async fn cancelled_account_wait_keeps_request_capacity_until_blocking_lookup_exits() {
    let root = tempfile::tempdir().unwrap();
    let session = session().await;
    let streams = pool();
    let requests = FileRequests::new(request_limits()).unwrap();
    let settings = settings(root.path());
    let gate = Arc::new(Gate::default());
    let release = Release(Arc::clone(&gate));
    let (accounts, names) = accounts(root.path(), Some(Arc::clone(&gate)), Duration::from_secs(5));
    let mut headers = HeaderMap::new();
    headers.insert("x-access-token", "token".parse().unwrap());
    let auth = requests
        .admit()
        .unwrap()
        .authorize(&session, &Method::GET, &headers, b"path=never")
        .await
        .unwrap();
    let mut download = Box::pin(streams.download_authorized(
        auth,
        http_request("", "identity"),
        &accounts,
        &settings,
        limits(),
        Utc,
    ));
    tokio::select! {
        () = gate.started.notified() => {},
        result = &mut download => panic!("lookup did not wait: {:?}", result.err()),
    }
    drop(download);
    full(&requests);
    assert_eq!(names.lock().unwrap().len(), 1);
    let mut rotation =
        Box::pin(session.initialize(br#"{"accessToken":"token","defaultUser":"bob"}"#));
    assert!(rotation.as_mut().now_or_never().is_none());
    drop(release);
    released(&requests).await;
    rotation.await.unwrap();
    quiesce(&streams).await;
}

#[tokio::test]
async fn capability_operation_and_original_deadline_reject_before_account_or_file() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("file"), b"abcdef").unwrap();
    let session = session().await;
    let streams = pool();
    let settings = settings(root.path());
    let (accounts, names) = accounts(root.path(), None, Duration::from_secs(5));
    let mut headers = HeaderMap::new();
    headers.insert("x-access-token", "token".parse().unwrap());
    let query = || Query {
        path: Some(b"~/file".to_vec()),
        username: Some(b"explicit".to_vec()),
        signature: None,
        signature_expiration: None,
    };
    let auth_limits = AuthLimits {
        headers: crate::file_upload::HeaderLimits {
            bytes: 4096,
            fields: 16,
        },
        query_bytes: 4096,
    };
    let auth = session
        .authorize_file(
            &headers,
            query(),
            Operation::Write,
            auth_limits,
            tokio::time::Instant::now() + Duration::from_secs(30),
        )
        .await
        .unwrap();
    assert!(matches!(
        streams
            .download_authorized(
                auth,
                http_request("", "identity"),
                &accounts,
                &settings,
                limits(),
                Utc
            )
            .await,
        Err(DownloadRequestError::Authorization(
            AuthError::WrongOperation
        ))
    ));
    assert!(names.lock().unwrap().is_empty());
    tokio::time::pause();
    let auth = session
        .authorize_file(
            &headers,
            query(),
            Operation::Read,
            auth_limits,
            tokio::time::Instant::now() + Duration::from_secs(1),
        )
        .await
        .unwrap();
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(matches!(
        streams
            .download_authorized(
                auth,
                http_request("", "identity"),
                &accounts,
                &settings,
                limits(),
                Utc
            )
            .await,
        Err(DownloadRequestError::Authorization(
            AuthError::DeadlineExceeded
        ))
    ));
    tokio::time::resume();
    assert!(names.lock().unwrap().is_empty());
    assert_eq!(streams.slots.available_permits(), 1);
}

#[tokio::test]
async fn download_uses_bound_path_and_user_and_keeps_original_auth_deadline_in_body() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("file"), b"abcdef").unwrap();
    let session = session().await;
    let streams = pool();
    let settings = settings(root.path());
    let (accounts, names) = accounts(root.path(), None, Duration::from_secs(5));
    let mut headers = HeaderMap::new();
    headers.insert("x-access-token", "token".parse().unwrap());
    let mut request_limits = request_limits();
    request_limits.timeout = Duration::from_secs(10);
    let requests = FileRequests::new(request_limits).unwrap();
    let auth = requests
        .admit()
        .unwrap()
        .authorize(
            &session,
            &Method::GET,
            &headers,
            b"path=~/file&username=explicit",
        )
        .await
        .unwrap();
    let mut download = streams
        .download_authorized(
            auth,
            http_request("", "identity"),
            &accounts,
            &settings,
            limits(),
            Utc,
        )
        .await
        .unwrap();
    let bytes = download.next().await.unwrap().unwrap();
    assert_eq!(bytes.as_ref(), b"abc");
    assert_eq!(*names.lock().unwrap(), vec!["explicit"]);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(11)).await;
    assert!(matches!(
        download.next().await,
        Err(DownloadError::DeadlineExceeded)
    ));
    tokio::time::resume();
    drop(download);
    assert_eq!(streams.slots.available_permits(), 0);
    full(&requests);
    drop(bytes);
    released(&requests).await;
    quiesce(&streams).await;
}
