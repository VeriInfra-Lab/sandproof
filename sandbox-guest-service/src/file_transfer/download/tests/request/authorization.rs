use super::*;
use crate::file_authorization::{Error as AuthError, Limits as AuthLimits, Operation};
use sandproof_guest_runtime::file_query::Query;

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
    let auth = session
        .authorize_file(
            &headers,
            Query {
                path: Some(b"~/file".to_vec()),
                username: Some(b"explicit".to_vec()),
                signature: None,
                signature_expiration: None,
            },
            Operation::Read,
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
    drop(bytes);
    quiesce(&streams).await;
}
