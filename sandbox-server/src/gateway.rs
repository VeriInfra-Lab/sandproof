//! Fail-closed envd gateway. The user sees neither the VM network namespace
//! nor a host port: each request first checks the durable core authority.

use std::{
    error::Error,
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, Response, StatusCode, header},
};
use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;
use percent_encoding::percent_decode_str;
use sandproof_api::{
    PortError,
    managed::{DataKind, ManagedPort},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf},
    process::{ChildStdin, ChildStdout, Command},
};

use crate::linux::NodeHost;

const FILE_REQUEST_LIMIT: usize = 8 * 1024 * 1024;
const PROCESS_REQUEST_LIMIT: usize = 1024 * 1024;
const RESPONSE_LIMIT: usize = 32 * 1024 * 1024;
const URL_LIMIT: usize = 4096;
const ENVD_ADDRESS: &str = "172.16.0.2:49983";

struct StdioTunnel {
    read: ChildStdout,
    write: ChildStdin,
}

impl AsyncRead for StdioTunnel {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.read).poll_read(cx, buf)
    }
}

impl AsyncWrite for StdioTunnel {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.write).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.write).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.write).poll_shutdown(cx)
    }
}

fn response(status: StatusCode) -> Response<Body> {
    Response::builder()
        .status(status)
        .body(Body::empty())
        .expect("static status")
}

fn operation(method: &Method, path: &str) -> Option<(DataKind, bool)> {
    if path == "/health" && method == Method::GET {
        return Some((DataKind::Exec, false));
    }
    if path.starts_with("/process.Process/") && method == Method::POST {
        return Some((DataKind::Exec, false));
    }
    if path == "/files" {
        return match *method {
            Method::GET => Some((DataKind::FileRead, true)),
            Method::POST | Method::DELETE => Some((DataKind::FileWrite, true)),
            _ => None,
        };
    }
    if let Some(name) = path.strip_prefix("/filesystem.Filesystem/") {
        if method != Method::POST {
            return None;
        }
        let kind = match name {
            "ListDir" | "Stat" | "ReadFile" | "GetInfo" => DataKind::FileRead,
            "MakeDir" | "Remove" | "Move" | "WriteFile" | "Copy" => DataKind::FileWrite,
            _ => return None,
        };
        return Some((kind, true));
    }
    None
}

fn contains_nul_escape(bytes: &[u8]) -> bool {
    bytes.contains(&0)
        || bytes
            .windows(6)
            .any(|window| window.eq_ignore_ascii_case(br"\u0000"))
}

fn spawn_relay(namespace_name: &str) -> io::Result<tokio::process::Child> {
    Command::new("/usr/bin/ip")
        .args(["netns", "exec", namespace_name])
        .arg(std::env::current_exe()?)
        .arg("relay")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
}

/// SDK data requests carry only the sandbox's envd bearer token. It is
/// resolved to one running sandbox, and the core checks effective Exec or
/// FileRead/FileWrite permission before launching a namespace-local relay.
pub async fn handle(
    port: Arc<ManagedPort<NodeHost>>,
    namespace: String,
    mut request: Request<Body>,
) -> Response<Body> {
    let path = request.uri().path().to_owned();
    let Some((kind, file_request)) = operation(request.method(), &path) else {
        return response(StatusCode::NOT_FOUND);
    };
    let raw_url = request
        .uri()
        .path_and_query()
        .map_or("", |value| value.as_str());
    if raw_url.len() > URL_LIMIT {
        return response(StatusCode::URI_TOO_LONG);
    }
    if file_request
        && (percent_decode_str(raw_url).decode_utf8().is_err()
            || percent_decode_str(raw_url)
                .decode_utf8()
                .is_ok_and(|decoded| decoded.contains('\0')))
    {
        return response(StatusCode::BAD_REQUEST);
    }
    let mut tokens = request.headers().get_all("x-access-token").iter();
    let Some(token) = tokens.next() else {
        return response(StatusCode::UNAUTHORIZED);
    };
    if tokens.next().is_some() {
        return response(StatusCode::UNAUTHORIZED);
    }
    let Ok(token) = token.to_str() else {
        return response(StatusCode::UNAUTHORIZED);
    };
    let token = token.to_owned();
    let destination =
        match tokio::task::spawn_blocking(move || port.authorize_data(&token, kind)).await {
            Ok(Ok(value)) => value,
            Ok(Err(PortError::NotFound | PortError::Invalid)) => {
                return response(StatusCode::UNAUTHORIZED);
            }
            Ok(Err(_)) | Err(_) => return response(StatusCode::SERVICE_UNAVAILABLE),
        };
    if file_request {
        let (parts, body) = request.into_parts();
        let Ok(bytes) = to_bytes(body, FILE_REQUEST_LIMIT).await else {
            return response(StatusCode::PAYLOAD_TOO_LARGE);
        };
        if path.starts_with("/filesystem.Filesystem/") && contains_nul_escape(&bytes) {
            return response(StatusCode::BAD_REQUEST);
        }
        request = Request::from_parts(parts, Body::from(bytes));
    } else {
        // Process RPCs may be bidirectional. Preserve streaming while bounding
        // client-to-guest input; buffering the whole request can deadlock Start.
        let (parts, body) = request.into_parts();
        request = Request::from_parts(
            parts,
            Body::new(http_body_util::Limited::new(body, PROCESS_REQUEST_LIMIT)),
        );
    }
    let namespace_name = format!("vm-{namespace}-{}", destination.vm.0);
    let Ok(mut child) = spawn_relay(&namespace_name) else {
        return response(StatusCode::BAD_GATEWAY);
    };
    let tunnel = StdioTunnel {
        read: child.stdout.take().expect("piped stdout"),
        write: child.stdin.take().expect("piped stdin"),
    };
    let Ok((mut sender, connection)) = http1::Builder::new().handshake(TokioIo::new(tunnel)).await
    else {
        let _ = child.kill().await;
        return response(StatusCode::BAD_GATEWAY);
    };
    tokio::spawn(async move {
        let _ = connection.await;
        let _ = child.kill().await;
    });
    request.headers_mut().remove("x-api-key");
    request.headers_mut().insert(
        header::CONNECTION,
        "close".parse().expect("static connection"),
    );
    request.headers_mut().remove(header::TRANSFER_ENCODING);
    request.headers_mut().remove(header::CONTENT_LENGTH);
    request
        .headers_mut()
        .insert(header::HOST, ENVD_ADDRESS.parse().expect("static host"));
    let path_and_query = request
        .uri()
        .path_and_query()
        .map_or("/", |value| value.as_str());
    let Ok(uri) = path_and_query.parse() else {
        return response(StatusCode::BAD_REQUEST);
    };
    *request.uri_mut() = uri;
    match sender.send_request(request).await {
        Ok(upstream) => {
            upstream.map(|body| Body::new(http_body_util::Limited::new(body, RESPONSE_LIMIT)))
        }
        Err(_) => response(StatusCode::BAD_GATEWAY),
    }
}

/// One process per trusted, namespace-selected data connection. It transports
/// bytes only; no user-controlled target address or host path is accepted.
///
/// # Errors
/// Fails when envd is unreachable or the caller disconnects.
pub async fn relay() -> Result<(), Box<dyn Error>> {
    let stream = tokio::net::TcpStream::connect(ENVD_ADDRESS).await?;
    let (mut reader, mut writer) = stream.into_split();
    let mut input = tokio::io::stdin();
    let mut output = tokio::io::stdout();
    let upstream = async {
        tokio::io::copy(&mut input, &mut writer).await?;
        writer.shutdown().await?;
        Ok::<(), io::Error>(())
    };
    let downstream = async {
        tokio::io::copy(&mut reader, &mut output).await?;
        output.flush().await?;
        Ok::<(), io::Error>(())
    };
    tokio::try_join!(upstream, downstream)?;
    Ok(())
}
