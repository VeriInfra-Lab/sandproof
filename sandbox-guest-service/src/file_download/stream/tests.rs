use super::*;
mod errors;
mod negotiation;
use futures_util::FutureExt;
use http_body_util::BodyExt;
use sandproof_guest_runtime::file_download::{self as wire, date::Utc};
use std::{
    io::Cursor,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::oneshot;

fn limits() -> IdentityLimits {
    IdentityLimits {
        plan: Limits {
            request: wire::Limits {
                header_bytes: 8192,
                items: 32,
            },
            response_header_bytes: 8192,
            wire_bytes: 1 << 20,
        },
        chunk_bytes: 3,
        file_bytes: 1 << 20,
        path_bytes: 4096,
    }
}
fn request(range: &'static str) -> IdentityRequest {
    let mut headers = HeaderMap::new();
    headers.insert("range", HeaderValue::from_static(range));
    IdentityRequest {
        method: Method::GET,
        headers,
        response: HeaderMap::new(),
        content_type: HeaderValue::from_static("application/octet-stream").into(),
        disposition_path: None,
        boundary: Some("a".repeat(60)),
    }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}
struct Guard(Option<oneshot::Sender<()>>);
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(());
        }
    }
}
fn owner() -> (Owner, oneshot::Receiver<()>) {
    let (tx, rx) = oneshot::channel();
    (Arc::new(Guard(Some(tx))), rx)
}
async fn signal<T>(rx: oneshot::Receiver<T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .unwrap()
        .unwrap()
}
async fn drain(mut download: IdentityDownload) -> (Vec<u8>, Option<DownloadError>) {
    let mut out = Vec::new();
    loop {
        match download.next().await {
            Ok(Some(chunk)) => {
                assert!(chunk.as_ref().len() <= 3);
                out.extend(chunk.as_ref());
            }
            Ok(None) => {
                assert!(download.next().await.unwrap().is_none());
                return (out, None);
            }
            Err(error) => {
                assert!(matches!(
                    download.next().await,
                    Err(DownloadError::Unavailable)
                ));
                return (out, Some(error));
            }
        }
    }
}

#[tokio::test]
async fn multipart_stream_generates_boundary_in_worker_and_matches_advertised_wire_length() {
    let (guard, released) = owner();
    let mut req = request("bytes=0-0,3-4");
    req.boundary = None;
    let download = Download::open(
        || {
            Ok((
                Cursor::new(b"01234".to_vec()),
                Timestamp::new(1, 0).unwrap(),
            ))
        },
        req,
        limits(),
        Utc,
        deadline(),
        guard,
    )
    .await
    .unwrap();
    assert_eq!(download.status(), StatusCode::PARTIAL_CONTENT);
    let boundary = download.headers()["content-type"]
        .to_str()
        .unwrap()
        .strip_prefix("multipart/byteranges; boundary=")
        .unwrap()
        .to_owned();
    assert_eq!(boundary.len(), 60);
    let length: usize = download.headers()["content-length"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    let (bytes, error) = drain(download).await;
    assert!(error.is_none());
    assert_eq!(bytes.len(), length);
    assert_eq!(bytes, format!("--{boundary}\r\nContent-Range: bytes 0-0/5\r\nContent-Type: application/octet-stream\r\n\r\n0\r\n--{boundary}\r\nContent-Range: bytes 3-4/5\r\nContent-Type: application/octet-stream\r\n\r\n34\r\n--{boundary}--\r\n").as_bytes());
    signal(released).await;
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Event {
    Stat,
    Open,
    Seek(SeekFrom),
    Read,
    Drop,
}
struct Traced {
    data: Cursor<Vec<u8>>,
    log: Arc<Mutex<Vec<Event>>>,
    seek_count: usize,
    fail_seek: usize,
    seek_errno: Option<wire_errno::LinuxErrno>,
    report_extra: u64,
    fail_read: bool,
    fail_at: Option<u64>,
}
impl Read for Traced {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        assert!(bytes.len() <= 3);
        self.log.lock().unwrap().push(Event::Read);
        if self.fail_read || self.fail_at.is_some_and(|n| self.data.position() >= n) {
            return Err(io::Error::other("private path must not leak"));
        }
        self.data.read(bytes)
    }
}
impl Seek for Traced {
    fn seek(&mut self, offset: SeekFrom) -> io::Result<u64> {
        self.log.lock().unwrap().push(Event::Seek(offset));
        self.seek_count += 1;
        if self.seek_count == self.fail_seek {
            return Err(self.seek_errno.map_or_else(
                || io::Error::other("private seek details"),
                io::Error::other,
            ));
        }
        let value = self.data.seek(offset)?;
        Ok(value
            + if matches!(offset, SeekFrom::End(_)) {
                self.report_extra
            } else {
                0
            })
    }
}
impl Drop for Traced {
    fn drop(&mut self) {
        self.log.lock().unwrap().push(Event::Drop);
    }
}
fn traced() -> (Traced, Arc<Mutex<Vec<Event>>>) {
    let log = Arc::new(Mutex::new(vec![]));
    (
        Traced {
            data: Cursor::new(b"01234".to_vec()),
            log: Arc::clone(&log),
            seek_count: 0,
            fail_seek: 0,
            seek_errno: None,
            report_extra: 0,
            fail_read: false,
            fail_at: None,
        },
        log,
    )
}
async fn opened(
    source: Traced,
    request: IdentityRequest,
    owner: Owner,
) -> Result<IdentityDownload, DownloadError> {
    IdentityDownload::open(
        move || Ok((source, Timestamp::new(784_111_777, 0).unwrap())),
        request,
        limits(),
        Utc,
        deadline(),
        owner,
    )
    .await
}

#[tokio::test]
async fn conditional_replies_close_descriptor_before_any_seek_or_read() {
    for (header, value, status) in [
        ("if-none-match", "*", StatusCode::NOT_MODIFIED),
        ("if-match", "bad", StatusCode::PRECONDITION_FAILED),
    ] {
        let (source, log) = traced();
        let (guard, released) = owner();
        let mut request = request("bytes=bad");
        request
            .headers
            .insert(header, HeaderValue::from_static(value));
        let download = opened(source, request, guard).await.unwrap();
        assert_eq!(download.status(), status);
        assert_eq!(*log.lock().unwrap(), [Event::Drop]);
        assert_eq!(drain(download).await, (vec![], None));
        signal(released).await;
    }
}

#[tokio::test]
async fn full_single_multi_zero_and_head_execute_exact_planned_bytes_without_preread() {
    for range in [
        "",
        "bytes=1-2",
        "bytes=0-0,3-4",
        "bytes=-0",
        "bytes=-0,-0",
        "bytes=99-",
        "bad",
    ] {
        for head in [false, true] {
            let (source, log) = traced();
            let (guard, released) = owner();
            let mut req = request(range);
            req.method = if head { Method::HEAD } else { Method::GET };
            let download = opened(source, req, guard).await.unwrap();
            assert!(!log.lock().unwrap().contains(&Event::Read));
            let status = download.status();
            let length = download
                .headers()
                .get("content-length")
                .map(|s| s.to_str().unwrap().parse::<usize>().unwrap());
            let (bytes, error) = drain(download).await;
            assert_eq!(error, None);
            if head {
                assert!(bytes.is_empty());
                assert!(!log.lock().unwrap().contains(&Event::Read));
            } else if let Some(length) = length {
                assert_eq!(bytes.len(), length);
            }
            if !head {
                match range {
                    "" => assert_eq!(bytes, b"01234"),
                    "bytes=1-2" => assert_eq!(bytes, b"12"),
                    "bytes=-0" => assert!(bytes.is_empty()),
                    "bytes=0-0,3-4" => {
                        let boundary = "a".repeat(60);
                        assert_eq!(bytes, format!("--{boundary}\r\nContent-Range: bytes 0-0/5\r\nContent-Type: application/octet-stream\r\n\r\n0\r\n--{boundary}\r\nContent-Range: bytes 3-4/5\r\nContent-Type: application/octet-stream\r\n\r\n34\r\n--{boundary}--\r\n").as_bytes());
                    }
                    "bad" | "bytes=99-" => assert_eq!(status, StatusCode::RANGE_NOT_SATISFIABLE),
                    _ => {}
                }
            }
            signal(released).await;
        }
    }
}

#[tokio::test]
async fn initial_seek_failures_precede_headers_and_later_failures_never_emit_success_end() {
    for (fail_seek, expected) in [
        (1, DownloadError::LengthSeek),
        (2, DownloadError::LengthSeek),
        (3, DownloadError::RangeSeek),
    ] {
        let (mut source, log) = traced();
        source.fail_seek = fail_seek;
        let (guard, released) = owner();
        assert!(
            matches!(opened(source, request("bytes=1-2"), guard).await, Err(error) if error == expected)
        );
        signal(released).await;
        assert!(!log.lock().unwrap().contains(&Event::Read));
        assert_eq!(log.lock().unwrap().last(), Some(&Event::Drop));
    }
    for (fail_seek, fail_read, report_extra, range, expected) in [
        (3, false, 0, "bytes=0-0,3-4", DownloadError::RangeSeek),
        (0, true, 0, "bytes=0-0,3-4", DownloadError::Read),
        (0, false, 3, "bytes=0-0,3-7", DownloadError::Incomplete),
        (0, false, 3, "", DownloadError::Incomplete),
    ] {
        let (mut source, _) = traced();
        source.fail_seek = fail_seek;
        source.fail_read = fail_read;
        source.report_extra = report_extra;
        let (guard, released) = owner();
        let download = opened(source, request(range), guard).await.unwrap();
        let (bytes, error) = drain(download).await;
        assert_eq!(error, Some(expected));
        assert!(!bytes.ends_with(format!("--{}--\r\n", "a".repeat(60)).as_bytes()));
        if range.is_empty() {
            assert_eq!(bytes, b"01234");
        }
        signal(released).await;
    }
}

#[tokio::test]
async fn byte_clones_retain_guard_and_prevent_further_reads_after_cancelled_demand() {
    let (source, log) = traced();
    let (guard, mut released) = owner();
    let mut body = opened(source, request(""), guard)
        .await
        .unwrap()
        .into_response()
        .into_body();
    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let clone = bytes.clone();
    let snapshot = log.lock().unwrap().clone();
    let mut next = Box::pin(body.frame());
    assert!(next.as_mut().now_or_never().is_none());
    drop(bytes);
    assert!(next.as_mut().now_or_never().is_none());
    assert_eq!(*log.lock().unwrap(), snapshot);
    drop(next);
    drop(body);
    assert!(matches!(
        released.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    drop(clone);
    signal(released).await;
    assert_eq!(log.lock().unwrap().last(), Some(&Event::Drop));
}

#[tokio::test]
async fn source_failure_is_an_http_body_error_without_private_io_details() {
    let (mut source, _) = traced();
    source.fail_read = true;
    let (guard, released) = owner();
    let mut body = opened(source, request(""), guard)
        .await
        .unwrap()
        .into_response()
        .into_body();
    let error = body.frame().await.expect("error, not EOF").unwrap_err();
    assert!(error.to_string().contains("Read"));
    assert!(!error.to_string().contains("private"));
    drop(body);
    signal(released).await;
}

struct Blocking {
    data: Cursor<Vec<u8>>,
    on_seek: bool,
    entered: Option<oneshot::Sender<()>>,
    release: std::sync::mpsc::Receiver<()>,
    dropped: Arc<AtomicBool>,
}
impl Blocking {
    fn wait(&mut self, seek: bool) {
        if seek == self.on_seek
            && let Some(tx) = self.entered.take()
        {
            let _ = tx.send(());
            self.release.recv_timeout(Duration::from_secs(5)).unwrap();
        }
    }
}
impl Read for Blocking {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.wait(false);
        self.data.read(bytes)
    }
}
impl Seek for Blocking {
    fn seek(&mut self, offset: SeekFrom) -> io::Result<u64> {
        self.wait(true);
        self.data.seek(offset)
    }
}
impl Drop for Blocking {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn blocked_seek_or_read_keeps_original_guard_until_syscall_and_source_drop_finish() {
    for on_seek in [false, true] {
        for expire in [false, true] {
            let (tx, rx) = oneshot::channel();
            let (release, wait) = std::sync::mpsc::channel();
            let dropped = Arc::new(AtomicBool::new(false));
            let source = Blocking {
                data: Cursor::new(b"abc".to_vec()),
                on_seek,
                entered: Some(tx),
                release: wait,
                dropped: Arc::clone(&dropped),
            };
            let (guard, mut released) = owner();
            let task = tokio::spawn(async move {
                let download = IdentityDownload::open(
                    move || Ok((source, Timestamp::new(1, 0).unwrap())),
                    request(""),
                    limits(),
                    Utc,
                    deadline(),
                    guard,
                )
                .await?;
                let (_, error) = drain(download).await;
                error.map_or(Ok(()), Err)
            });
            signal(rx).await;
            if expire {
                tokio::time::pause();
                tokio::time::advance(Duration::from_secs(31)).await;
                assert_eq!(task.await.unwrap(), Err(DownloadError::DeadlineExceeded));
                tokio::time::resume();
            } else {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            }
            assert!(!dropped.load(Ordering::SeqCst));
            assert!(matches!(
                released.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ));
            release.send(()).unwrap();
            signal(released).await;
            assert!(dropped.load(Ordering::SeqCst));
        }
    }
}

#[tokio::test]
async fn file_budget_checks_selected_ranges_and_head_and_wire_budget_checks_mime() {
    for (range, head, file_bytes, expected) in [
        ("bytes=1-2", false, 2, true),
        ("bytes=1-2", true, 1, false),
        ("", false, 4, false),
    ] {
        let (source, _) = traced();
        let (guard, released) = owner();
        let mut req = request(range);
        if head {
            req.method = Method::HEAD;
        }
        let result = IdentityDownload::open(
            move || Ok((source, Timestamp::new(1, 0).unwrap())),
            req,
            IdentityLimits {
                file_bytes,
                ..limits()
            },
            Utc,
            deadline(),
            guard,
        )
        .await;
        if expected {
            assert_eq!(drain(result.unwrap()).await, (b"12".to_vec(), None));
        } else {
            assert!(matches!(result, Err(DownloadError::ResourceExhausted)));
        }
        signal(released).await;
    }
    let (source, log) = traced();
    let (guard, released) = owner();
    let mut budget = limits();
    budget.plan.wire_bytes = 3;
    let result = IdentityDownload::open(
        move || Ok((source, Timestamp::new(1, 0).unwrap())),
        request("bytes=0-0,3-4"),
        budget,
        Utc,
        deadline(),
        guard,
    )
    .await;
    assert!(matches!(
        result,
        Err(DownloadError::Plan(Error::ResponseLimit))
    ));
    signal(released).await;
    assert!(!log.lock().unwrap().contains(&Event::Read));
}

fn automatic(mut req: IdentityRequest, original: &str) -> IdentityRequest {
    let catalog = sandproof_guest_runtime::file_mime::Catalog::from_sources(
        [None; 2],
        [None; 4],
        sandproof_guest_runtime::file_mime::CatalogLimits {
            input_bytes: 65536,
            line_bytes: 1024,
            entries: 512,
            retained_bytes: 65536,
            value_bytes: 1024,
        },
    )
    .unwrap();
    req.content_type = ContentType::Detect {
        original_path: original.into(),
        catalog: Arc::new(catalog),
    };
    req.disposition_path = Some("/resolved/雪.jpg".as_bytes().to_vec());
    req
}

#[tokio::test]
async fn mime_lookup_uses_original_name_and_conditions_precede_sniffing() {
    for condition in [
        None,
        Some(("if-none-match", "*")),
        Some(("if-match", "bad")),
    ] {
        let (source, log) = traced();
        let (guard, released) = owner();
        let mut req = automatic(
            request(""),
            if condition.is_some() {
                "unknown"
            } else {
                "original.CSS"
            },
        );
        if let Some((name, value)) = condition {
            req.headers.insert(name, HeaderValue::from_static(value));
        }
        let download = opened(source, req, guard).await.unwrap();
        assert_eq!(
            download.headers()["content-disposition"],
            "inline; filename*=utf-8''%E9%9B%AA.jpg"
        );
        assert!(!log.lock().unwrap().contains(&Event::Read));
        if condition.is_none() {
            assert_eq!(
                download.headers()["content-type"],
                "text/css; charset=utf-8"
            );
        } else {
            assert!(!download.headers().contains_key("content-type"));
        }
        let _ = drain(download).await;
        signal(released).await;
    }
    let (source, log) = traced();
    let (guard, released) = owner();
    let mut req = automatic(request(""), "unknown");
    req.response
        .insert("content-type", HeaderValue::from_static(""));
    let download = opened(source, req, guard).await.unwrap();
    assert_eq!(download.headers()["content-type"], "");
    assert!(!log.lock().unwrap().contains(&Event::Read));
    drop(download);
    signal(released).await;
}

#[tokio::test]
async fn sniff_reads_only_512_then_rewinds_before_length_and_body_and_preserves_read_error_prefix()
{
    let (mut source, log) = traced();
    let mut data = vec![b'a'; 512];
    data.extend(b"\0tail");
    source.data = Cursor::new(data.clone());
    let (guard, released) = owner();
    let download = opened(source, automatic(request(""), "unknown"), guard)
        .await
        .unwrap();
    assert_eq!(
        download.headers()["content-type"],
        "text/plain; charset=utf-8"
    );
    let events = log.lock().unwrap().clone();
    assert_eq!(events.iter().filter(|e| **e == Event::Read).count(), 171);
    assert_eq!(
        &events[171..],
        &[
            Event::Seek(SeekFrom::Start(0)),
            Event::Seek(SeekFrom::End(0)),
            Event::Seek(SeekFrom::Start(0))
        ]
    );
    assert_eq!(drain(download).await, (data, None));
    signal(released).await;
    let (mut source, _) = traced();
    source.fail_read = true;
    let (guard, released) = owner();
    let download = opened(source, automatic(request(""), "unknown"), guard)
        .await
        .unwrap();
    assert_eq!(
        download.headers()["content-type"],
        "text/plain; charset=utf-8"
    );
    assert_eq!(drain(download).await, (vec![], Some(DownloadError::Read)));
    signal(released).await;
    // A read error after a useful prefix must still classify that prefix.
    let (mut source, _) = traced();
    source.data = Cursor::new(b"%PDF-123456789".to_vec());
    source.fail_at = Some(6);
    let (guard, released) = owner();
    let download = opened(source, automatic(request(""), "unknown"), guard)
        .await
        .unwrap();
    assert_eq!(download.headers()["content-type"], "application/pdf");
    assert_eq!(
        drain(download).await,
        (b"%PDF-1".to_vec(), Some(DownloadError::Read))
    );
    signal(released).await;
    let (mut source, _) = traced();
    source.fail_seek = 1;
    let (guard, released) = owner();
    assert!(matches!(
        opened(source, automatic(request(""), "unknown"), guard).await,
        Err(DownloadError::SniffSeek)
    ));
    signal(released).await;
}

#[tokio::test]
async fn cancellation_during_sniff_read_or_rewind_retains_guard_until_source_released() {
    for on_seek in [false, true] {
        let (tx, rx) = oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let dropped = Arc::new(AtomicBool::new(false));
        let source = Blocking {
            data: Cursor::new(b"abc".to_vec()),
            on_seek,
            entered: Some(tx),
            release: wait,
            dropped: Arc::clone(&dropped),
        };
        let (guard, mut released) = owner();
        let task = tokio::spawn(IdentityDownload::open(
            move || Ok((source, Timestamp::new(1, 0).unwrap())),
            automatic(request(""), "unknown"),
            limits(),
            Utc,
            deadline(),
            guard,
        ));
        signal(rx).await;
        task.abort();
        assert!(task.await.is_err());
        assert!(!dropped.load(Ordering::SeqCst));
        assert!(matches!(
            released.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        release.send(()).unwrap();
        signal(released).await;
        assert!(dropped.load(Ordering::SeqCst));
    }
}
