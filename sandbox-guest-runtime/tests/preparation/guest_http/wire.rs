//! Small bounded HTTP/1 test client. Every connection asks for close; chunk
//! boundaries and Connect frame boundaries are deliberately handled separately.
use super::*;
use std::{fmt::Write as _, net::SocketAddr};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::TcpStream,
};

pub(super) struct Wire {
    socket: BufReader<TcpStream>,
    pub status: u16,
    chunked: bool,
    remaining: usize,
    pending: Vec<u8>,
}
impl Wire {
    pub async fn open(address: SocketAddr, request: HttpRequest<Body>) -> Self {
        timeout(Duration::from_secs(5), Self::connect(address, request))
            .await
            .unwrap()
    }
    async fn connect(address: SocketAddr, request: HttpRequest<Body>) -> Self {
        let (parts, body) = request.into_parts();
        let bytes = axum::body::to_bytes(body, 8192).await.unwrap();
        let mut socket = BufReader::new(TcpStream::connect(address).await.unwrap());
        let mut head = format!(
            "{} {} HTTP/1.1\r\nHost: local\r\nConnection: close\r\nContent-Length: {}\r\n",
            parts.method,
            parts.uri,
            bytes.len()
        );
        for (name, value) in &parts.headers {
            write!(head, "{name}: {}\r\n", value.to_str().unwrap()).unwrap();
        }
        head.push_str("\r\n");
        socket.write_all(head.as_bytes()).await.unwrap();
        socket.write_all(&bytes).await.unwrap();
        let mut line = String::new();
        socket.read_line(&mut line).await.unwrap();
        let status = line
            .split_ascii_whitespace()
            .nth(1)
            .expect("missing HTTP status")
            .parse()
            .unwrap();
        let (mut chunked, mut remaining, mut size) = (false, 0, 0);
        loop {
            line.clear();
            assert_ne!(socket.read_line(&mut line).await.unwrap(), 0);
            size += line.len();
            assert!(size < 8192);
            if line == "\r\n" {
                break;
            }
            let (name, value) = line.trim().split_once(':').unwrap();
            if name.eq_ignore_ascii_case("transfer-encoding") {
                assert_eq!(value.trim(), "chunked");
                chunked = true;
            }
            if name.eq_ignore_ascii_case("content-length") {
                remaining = value.trim().parse().unwrap();
                assert!(remaining <= 8192);
            }
        }
        Self {
            socket,
            status,
            chunked,
            remaining,
            pending: Vec::new(),
        }
    }
    async fn chunk(&mut self) -> Option<Vec<u8>> {
        timeout(Duration::from_secs(5), async {
            let size = if self.chunked {
                let mut line = String::new();
                assert_ne!(self.socket.read_line(&mut line).await.unwrap(), 0);
                usize::from_str_radix(line.trim(), 16).unwrap()
            } else {
                std::mem::take(&mut self.remaining)
            };
            assert!(size <= 8192);
            let mut data = vec![0; size];
            self.socket.read_exact(&mut data).await.unwrap();
            if self.chunked {
                let mut crlf = [0; 2];
                self.socket.read_exact(&mut crlf).await.unwrap();
                assert_eq!(&crlf, b"\r\n");
            }
            (size != 0).then_some(data)
        })
        .await
        .unwrap()
    }
    pub async fn body(mut self) -> Vec<u8> {
        let mut data = Vec::new();
        while let Some(chunk) = self.chunk().await {
            data.extend(chunk);
            assert!(data.len() <= 8192);
        }
        self.eof().await;
        data
    }
    pub async fn frame(&mut self, flag: u8) -> Value {
        loop {
            if self.pending.len() >= 5 {
                let size = u32::from_be_bytes(self.pending[1..5].try_into().unwrap()) as usize;
                assert!(size <= 8192);
                if self.pending.len() >= size + 5 {
                    let frame: Vec<_> = self.pending.drain(..size + 5).collect();
                    return json_frame(&frame, flag);
                }
            }
            let chunk = self.chunk().await.expect("incomplete Connect frame");
            self.pending.extend(chunk);
            assert!(self.pending.len() <= 16384);
        }
    }
    pub async fn finish(mut self) {
        assert!(self.pending.is_empty());
        assert!(self.chunk().await.is_none());
        self.eof().await;
    }
    async fn eof(&mut self) {
        assert_eq!(
            timeout(Duration::from_secs(5), self.socket.read(&mut [0; 1]))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    }
}

pub(super) async fn rejected(address: SocketAddr, path: &str, body: &[u8]) {
    timeout(Duration::from_secs(5), async {
        let mut socket = TcpStream::connect(address).await.unwrap();
        socket.write_all(format!("POST {path} HTTP/1.1\r\nHost: local\r\nConnection: close\r\nContent-Type: application/proto\r\nX-Access-Token: token\r\nContent-Length: {}\r\n\r\n",body.len()).as_bytes()).await.unwrap();
        socket.write_all(body).await.unwrap();
        let mut response = Vec::new();
        // A reset is also a connection rejection, never a successful HTTP reply.
        let _ = socket.read_to_end(&mut response).await;
        assert!(response.is_empty());
    }).await.unwrap();
}
