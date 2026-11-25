//! Bounded request/reply framing after the Firecracker Unix socket connection.
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

const MAX_MESSAGE: usize = 16 * 1024;
const CONNECT: &[u8] = b"CONNECT 770\n";

pub(crate) fn time_left(deadline: Instant) -> io::Result<Duration> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "hot fan-out control deadline",
        ));
    }
    Ok(remaining)
}

fn read(stream: &mut UnixStream, mut data: &mut [u8], deadline: Instant) -> io::Result<()> {
    while !data.is_empty() {
        stream.set_read_timeout(Some(time_left(deadline)?))?;
        match stream.read(data) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
            Ok(length) => data = &mut data[length..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => (),
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn write(stream: &mut UnixStream, mut data: &[u8], deadline: Instant) -> io::Result<()> {
    while !data.is_empty() {
        stream.set_write_timeout(Some(time_left(deadline)?))?;
        match stream.write(data) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero)),
            Ok(length) => data = &data[length..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => (),
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn acknowledgement(stream: &mut UnixStream, deadline: Instant) -> io::Result<()> {
    let mut line = [0; 16];
    for length in 1..=line.len() {
        read(stream, &mut line[length - 1..length], deadline)?;
        if line[length - 1] != b'\n' {
            continue;
        }
        let port = line[..length - 1]
            .strip_prefix(b"OK ")
            .and_then(|digits| std::str::from_utf8(digits).ok())
            .filter(|digits| {
                !digits.starts_with('0') && digits.bytes().all(|byte| byte.is_ascii_digit())
            })
            .and_then(|digits| digits.parse::<u32>().ok());
        if port.is_some_and(|port| port > 0) {
            return Ok(());
        }
        break;
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "Firecracker vsock acknowledgement",
    ))
}

pub(crate) fn request_length(request: &[u8]) -> io::Result<[u8; 4]> {
    u32::try_from(request.len())
        .ok()
        .filter(|_| (1..=MAX_MESSAGE).contains(&request.len()))
        .map(u32::to_be_bytes)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "hot fan-out request size"))
}

pub(crate) fn exchange(
    stream: &mut UnixStream,
    request: &[u8],
    deadline: Instant,
) -> io::Result<Vec<u8>> {
    let length = request_length(request)?;
    write(stream, CONNECT, deadline)?;
    acknowledgement(stream, deadline)?;
    write(stream, &length, deadline)?;
    write(stream, request, deadline)?;
    let mut length = [0; 4];
    read(stream, &mut length, deadline)?;
    let length = u32::from_be_bytes(length) as usize;
    if !(1..=MAX_MESSAGE).contains(&length) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "hot fan-out response size",
        ));
    }
    let mut response = vec![0; length];
    read(stream, &mut response, deadline)?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(2)
    }

    #[test]
    fn sends_a_single_request_after_connect_and_reads_exact_reply_bytes() {
        let (mut host, mut peer) = UnixStream::pair().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let (received, hold) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let mut connect = [0; CONNECT.len()];
            peer.read_exact(&mut connect).unwrap();
            assert_eq!(connect, CONNECT);
            peer.write_all(b"OK 1073741824\n").unwrap();
            let mut request = [0; 6];
            peer.read_exact(&mut request).unwrap();
            assert_eq!(&request, b"\0\0\0\x02{}");
            peer.write_all(b"\0\0\0\x02[]").unwrap();
            hold.recv_timeout(Duration::from_secs(2)).unwrap();
        });
        let response = exchange(&mut host, b"{}", deadline());
        received.send(()).unwrap();
        server.join().unwrap();
        assert_eq!(response.unwrap(), b"[]");
    }

    #[test]
    fn refuses_invalid_acknowledgement_and_oversized_reply_without_raw_details() {
        for (bytes, expected) in [
            (
                b"guest secret\n".as_slice(),
                "Firecracker vsock acknowledgement",
            ),
            (
                b"OK 1073741824\n\xff\xff\xff\xff",
                "hot fan-out response size",
            ),
        ] {
            let (mut host, mut peer) = UnixStream::pair().unwrap();
            peer.write_all(bytes).unwrap();
            let error = exchange(&mut host, b"{}", deadline()).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn refuses_empty_or_oversized_requests_and_checks_original_deadline() {
        let (mut host, _peer) = UnixStream::pair().unwrap();
        assert_eq!(
            exchange(&mut host, &[], deadline()).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(request_length(&vec![0; MAX_MESSAGE + 1]).is_err());
        assert_eq!(
            exchange(&mut host, b"{}", Instant::now())
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
    }
}
