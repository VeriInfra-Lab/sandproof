//! A full Unix listener queue must not pin a sandbox worker in connect(2).
use rustix::net::{AddressFamily, SocketAddrUnix, SocketFlags, SocketType};
use std::{io, os::unix::net::UnixStream, path::Path};

pub(super) fn connect(path: &Path) -> io::Result<UnixStream> {
    let fd = rustix::net::socket_with(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
        None,
    )
    .map_err(super::errno_to_io)?;
    let address = SocketAddrUnix::new(path).map_err(super::errno_to_io)?;
    // For AF_UNIX, a full accept queue returns EAGAIN. Keep that failure as
    // unknown; never block waiting for a VMM that terminal work must stop.
    rustix::net::connect(&fd, &address).map_err(super::errno_to_io)?;
    let stream = UnixStream::from(fd);
    stream.set_nonblocking(false)?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        os::unix::net::UnixListener,
        time::{Duration, Instant},
    };

    #[test]
    fn actual_unix_connection_preserves_peer_and_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("api");
        let listener = UnixListener::bind(&path).unwrap();
        let mut client = connect(&path).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        client.write_all(b"original").unwrap();
        let mut bytes = [0; 8];
        server.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"original");
        let peer = rustix::net::sockopt::socket_peercred(&client).unwrap();
        assert_eq!(
            u32::try_from(peer.pid.as_raw_nonzero().get()).unwrap(),
            std::process::id()
        );
    }

    #[test]
    fn full_real_accept_queue_returns_unknown_without_waiting_for_server() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("api");
        let listener = UnixListener::bind(&path).unwrap();
        rustix::net::listen(&listener, 0).unwrap();
        let first = connect(&path).unwrap();
        let start = Instant::now();
        let error = connect(&path).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(start.elapsed() < Duration::from_secs(2));
        let (_accepted, _) = listener.accept().unwrap();
        let second = connect(&path).unwrap();
        drop((first, second));
    }
}
