//! Firecracker v1.16.1 API client for an already-created, fresh jailed VM.
//!
//! This module does not launch or jail Firecracker. The socket and every path
//! inside the jail must come from a separate trusted launcher. It configures
//! no network interface; the guest has only block devices and a vsock channel.

use serde_json::{Value, json};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

const BOOT_ARGS: &str =
    "console=ttyS0 reboot=k panic=1 pci=off root=/dev/vda ro init=/sbin/sandbox-init";

#[derive(Debug)]
pub enum FirecrackerError {
    InvalidConfiguration,
    UnexpectedStatus(u16),
    InvalidResponse,
    Io(io::Error),
}

impl From<io::Error> for FirecrackerError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl std::fmt::Display for FirecrackerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfiguration => write!(formatter, "invalid Firecracker configuration"),
            Self::UnexpectedStatus(status) => {
                write!(formatter, "Firecracker returned HTTP {status}")
            }
            Self::InvalidResponse => write!(formatter, "invalid Firecracker API response"),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for FirecrackerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

/// All paths are absolute *inside the jail*, except `api_socket` on the host.
#[derive(Clone, Debug)]
pub struct VmConfig {
    pub api_socket: PathBuf,
    pub kernel: PathBuf,
    pub rootfs: PathBuf,
    pub input: PathBuf,
    pub workspace: PathBuf,
    pub vsock_socket: PathBuf,
    pub memory_mib: u32,
}

impl VmConfig {
    fn validate(&self) -> Result<(), FirecrackerError> {
        let paths = [
            &self.api_socket,
            &self.kernel,
            &self.rootfs,
            &self.input,
            &self.workspace,
            &self.vsock_socket,
        ];
        if !(128..=4096).contains(&self.memory_mib)
            || paths
                .iter()
                .any(|path| !path.is_absolute() || path.to_str().is_none())
            || self.kernel == self.rootfs
            || self.rootfs == self.input
            || self.input == self.workspace
        {
            return Err(FirecrackerError::InvalidConfiguration);
        }
        Ok(())
    }
}

/// A narrowly scoped API client. Each request uses a fresh Unix connection.
#[derive(Clone, Debug)]
pub struct FirecrackerApi {
    config: VmConfig,
    timeout: Duration,
}

impl FirecrackerApi {
    /// Validates paths and creates a client; no I/O occurs yet.
    ///
    /// # Errors
    ///
    /// Returns `InvalidConfiguration` if any required path or size is invalid.
    pub fn new(config: VmConfig, timeout: Duration) -> Result<Self, FirecrackerError> {
        config.validate()?;
        if timeout.is_zero() {
            return Err(FirecrackerError::InvalidConfiguration);
        }
        Ok(Self { config, timeout })
    }

    /// Configure a fresh pre-boot instance, with no network device attached.
    ///
    /// # Errors
    ///
    /// Returns a transport, response, or API error. A partially configured VM
    /// must be killed by the caller; retrying on the same instance is unsafe.
    pub fn configure_fresh(&self) -> Result<(), FirecrackerError> {
        for (path, body) in self.configuration_requests()? {
            self.put(path, &body)?;
        }
        Ok(())
    }

    /// Start only after every configuration request succeeded.
    ///
    /// # Errors
    ///
    /// Returns a transport, response, or API error. An unknown result requires
    /// process-level reconciliation, not a blind second start request.
    pub fn start(&self) -> Result<(), FirecrackerError> {
        self.put("/actions", &json!({"action_type": "InstanceStart"}))
    }

    fn configuration_requests(&self) -> Result<Vec<(&'static str, Value)>, FirecrackerError> {
        Ok(vec![
            (
                "/machine-config",
                json!({"vcpu_count": 1, "mem_size_mib": self.config.memory_mib}),
            ),
            (
                "/boot-source",
                json!({
                    "kernel_image_path": path_text(&self.config.kernel)?,
                    "boot_args": BOOT_ARGS,
                }),
            ),
            (
                "/drives/rootfs",
                drive("rootfs", &self.config.rootfs, true, true)?,
            ),
            (
                "/drives/inputs",
                drive("inputs", &self.config.input, false, true)?,
            ),
            (
                "/drives/workspace",
                drive("workspace", &self.config.workspace, false, false)?,
            ),
            (
                "/vsock",
                json!({
                    "guest_cid": 3,
                    "uds_path": path_text(&self.config.vsock_socket)?,
                }),
            ),
        ])
    }

    fn put(&self, path: &str, body: &Value) -> Result<(), FirecrackerError> {
        let body = body.to_string();
        let mut stream = UnixStream::connect(&self.config.api_socket)?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;
        write!(
            stream,
            "PUT {path} HTTP/1.1\r\nHost: localhost\r\nAccept: application/json\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len(),
        )?;
        stream.flush()?;
        parse_response(&mut BufReader::new(stream))
    }
}

fn drive(id: &str, path: &Path, root: bool, readonly: bool) -> Result<Value, FirecrackerError> {
    Ok(json!({
        "drive_id": id,
        "path_on_host": path_text(path)?,
        "is_root_device": root,
        "is_read_only": readonly,
    }))
}

fn parse_response(reader: &mut impl BufRead) -> Result<(), FirecrackerError> {
    let status = read_line_limited(reader, 1024)?;
    let code = status
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or(FirecrackerError::InvalidResponse)?;
    let mut header_bytes = 0;
    loop {
        let header = read_line_limited(reader, 16 * 1024 - header_bytes)?;
        header_bytes += header.len();
        if header == "\r\n" || header == "\n" {
            break;
        }
    }
    if code == 204 {
        Ok(())
    } else {
        Err(FirecrackerError::UnexpectedStatus(code))
    }
}

fn read_line_limited(reader: &mut impl BufRead, limit: usize) -> Result<String, FirecrackerError> {
    let mut line = Vec::new();
    loop {
        if line.len() >= limit {
            return Err(FirecrackerError::InvalidResponse);
        }
        let mut byte = [0_u8; 1];
        if reader.read_exact(&mut byte).is_err() {
            return Err(FirecrackerError::InvalidResponse);
        }
        line.push(byte[0]);
        if byte[0] == b'\n' {
            return String::from_utf8(line).map_err(|_| FirecrackerError::InvalidResponse);
        }
    }
}

fn path_text(path: &Path) -> Result<&str, FirecrackerError> {
    path.to_str().ok_or(FirecrackerError::InvalidConfiguration)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn client(socket: &Path) -> FirecrackerApi {
        FirecrackerApi::new(
            VmConfig {
                api_socket: socket.to_path_buf(),
                kernel: "/kernel".into(),
                rootfs: "/rootfs.img".into(),
                input: "/input.img".into(),
                workspace: "/workspace.img".into(),
                vsock_socket: "/vsock.sock".into(),
                memory_mib: 512,
            },
            Duration::from_secs(2),
        )
        .unwrap()
    }

    #[test]
    fn configures_only_expected_devices_before_start() {
        let requests = client(Path::new("/api.sock"))
            .configuration_requests()
            .unwrap();
        assert_eq!(
            requests.iter().map(|(path, _)| *path).collect::<Vec<_>>(),
            [
                "/machine-config",
                "/boot-source",
                "/drives/rootfs",
                "/drives/inputs",
                "/drives/workspace",
                "/vsock"
            ]
        );
        assert_eq!(requests[3].1["is_read_only"], true);
        assert_eq!(requests[4].1["is_read_only"], false);
        assert_eq!(requests[1].1["boot_args"], BOOT_ARGS);
    }

    #[test]
    fn rejects_non_success_response() {
        assert!(matches!(
            parse_response(&mut Cursor::new(
                b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n"
            )),
            Err(FirecrackerError::UnexpectedStatus(400))
        ));
        assert!(parse_response(&mut Cursor::new(b"HTTP/1.1 204 No Content\r\n\r\n")).is_ok());
        let oversized = format!("HTTP/1.1 204 OK\r\nX: {}\r\n\r\n", "x".repeat(20_000));
        assert!(matches!(
            parse_response(&mut Cursor::new(oversized)),
            Err(FirecrackerError::InvalidResponse)
        ));
    }
}
