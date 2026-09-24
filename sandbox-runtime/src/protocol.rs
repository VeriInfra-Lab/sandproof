//! Bounded host-to-guest command channel over Firecracker vsock.
//!
//! The guest is untrusted from the host's perspective. All frame lengths and
//! response fields must be validated before writing a returned artifact to
//! any host-owned destination.

use serde_json::Value;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

pub const AGENT_PORT: u32 = 52;
pub const MAX_HEADER_BYTES: usize = 64 * 1024;
pub const MAX_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub enum ProtocolError {
    InvalidFrame,
    FrameTooLarge,
    InvalidHandshake,
    Io(io::Error),
}

impl From<io::Error> for ProtocolError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidFrame => write!(formatter, "invalid guest protocol frame"),
            Self::FrameTooLarge => write!(formatter, "guest protocol frame exceeds limit"),
            Self::InvalidHandshake => write!(formatter, "invalid Firecracker vsock handshake"),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ProtocolError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub header: Value,
    pub payload: Vec<u8>,
}

/// Write a JSON header and one binary payload using bounded length prefixes.
///
/// # Errors
///
/// Rejects oversized frames or returns a transport error.
pub fn write_frame(writer: &mut impl Write, frame: &Frame) -> Result<(), ProtocolError> {
    let header = serde_json::to_vec(&frame.header).map_err(|_| ProtocolError::InvalidFrame)?;
    if header.is_empty()
        || header.len() > MAX_HEADER_BYTES
        || frame.payload.len() > MAX_PAYLOAD_BYTES
    {
        return Err(ProtocolError::FrameTooLarge);
    }
    let header_len = u32::try_from(header.len()).map_err(|_| ProtocolError::FrameTooLarge)?;
    let payload_len =
        u64::try_from(frame.payload.len()).map_err(|_| ProtocolError::FrameTooLarge)?;
    writer.write_all(&header_len.to_be_bytes())?;
    writer.write_all(&payload_len.to_be_bytes())?;
    writer.write_all(&header)?;
    writer.write_all(&frame.payload)?;
    writer.flush()?;
    Ok(())
}

/// Reject announced sizes before allocating or reading untrusted payloads.
///
/// # Errors
///
/// Rejects invalid or oversized frames, or returns a transport error.
pub fn read_frame(reader: &mut impl Read) -> Result<Frame, ProtocolError> {
    let mut sizes = [0_u8; 12];
    reader.read_exact(&mut sizes)?;
    let header_len = u32::from_be_bytes(
        sizes[..4]
            .try_into()
            .map_err(|_| ProtocolError::InvalidFrame)?,
    ) as usize;
    let payload_len = u64::from_be_bytes(
        sizes[4..]
            .try_into()
            .map_err(|_| ProtocolError::InvalidFrame)?,
    );
    if header_len == 0 || header_len > MAX_HEADER_BYTES || payload_len > MAX_PAYLOAD_BYTES as u64 {
        return Err(ProtocolError::FrameTooLarge);
    }
    let payload_len = usize::try_from(payload_len).map_err(|_| ProtocolError::FrameTooLarge)?;
    let mut header = vec![0_u8; header_len];
    reader.read_exact(&mut header)?;
    let header = serde_json::from_slice(&header).map_err(|_| ProtocolError::InvalidFrame)?;
    let mut payload = vec![0_u8; payload_len];
    reader.read_exact(&mut payload)?;
    Ok(Frame { header, payload })
}

/// Connected host-initiated vsock channel. One connection handles one request.
#[derive(Debug)]
pub struct HostChannel {
    stream: UnixStream,
}

impl HostChannel {
    /// Connect to the guest agent through the jailed VM's host-side UDS.
    ///
    /// # Errors
    ///
    /// Returns an I/O or handshake error when the guest agent is unavailable.
    pub fn connect(vsock_socket: &Path, timeout: Duration) -> Result<Self, ProtocolError> {
        if timeout.is_zero() {
            return Err(ProtocolError::InvalidHandshake);
        }
        let mut stream = UnixStream::connect(vsock_socket)?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        writeln!(stream, "CONNECT {AGENT_PORT}")?;
        stream.flush()?;
        read_acknowledgement(&mut stream)?;
        Ok(Self { stream })
    }

    /// Send one request and receive one bounded response.
    ///
    /// # Errors
    ///
    /// Returns an I/O or framing error. A timed-out command has an unknown
    /// outcome; the caller must stop the sandbox rather than blindly retry it.
    pub fn request(&mut self, frame: &Frame) -> Result<Frame, ProtocolError> {
        write_frame(&mut self.stream, frame)?;
        read_frame(&mut self.stream)
    }
}

fn read_acknowledgement(reader: &mut impl Read) -> Result<u32, ProtocolError> {
    let mut line = Vec::new();
    loop {
        if line.len() >= 32 {
            return Err(ProtocolError::InvalidHandshake);
        }
        let mut byte = [0_u8; 1];
        if reader.read_exact(&mut byte).is_err() {
            return Err(ProtocolError::InvalidHandshake);
        }
        line.push(byte[0]);
        if byte[0] == b'\n' {
            break;
        }
    }
    let text = std::str::from_utf8(&line).map_err(|_| ProtocolError::InvalidHandshake)?;
    let port = text
        .strip_prefix("OK ")
        .and_then(|suffix| suffix.strip_suffix('\n'))
        .and_then(|digits| digits.parse::<u32>().ok())
        .filter(|port| *port > 0)
        .ok_or(ProtocolError::InvalidHandshake)?;
    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    #[test]
    fn binary_frames_round_trip() {
        let frame = Frame {
            header: json!({"op": "read", "path": "result.bin"}),
            payload: vec![0, 255, 10],
        };
        let mut wire = Vec::new();
        write_frame(&mut wire, &frame).unwrap();
        assert_eq!(read_frame(&mut Cursor::new(wire)).unwrap(), frame);
    }

    #[test]
    fn rejects_oversized_and_truncated_frames() {
        let mut wire = Vec::new();
        wire.extend_from_slice(&u32::MAX.to_be_bytes());
        wire.extend_from_slice(&0_u64.to_be_bytes());
        assert!(matches!(
            read_frame(&mut Cursor::new(wire)),
            Err(ProtocolError::FrameTooLarge)
        ));
        assert!(read_frame(&mut Cursor::new([0_u8; 3])).is_err());
    }

    #[test]
    fn accepts_assigned_host_port_not_guest_port() {
        assert_eq!(
            read_acknowledgement(&mut Cursor::new(b"OK 1073741824\n")).unwrap(),
            1_073_741_824
        );
        assert!(read_acknowledgement(&mut Cursor::new(b"OK 0\n")).is_err());
        assert!(read_acknowledgement(&mut Cursor::new(b"ERR 52\n")).is_err());
    }
}
