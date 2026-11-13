//! One owned Python child exercising the original relay against real Rust TLS.
use super::{Certificates, NodeIdentity, SocketAddr};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
    process::{Child, ChildStdout, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

pub(super) struct PythonRelay {
    child: Child,
    output: Option<BufReader<ChildStdout>>,
    _files: tempfile::TempDir,
    pub address: SocketAddr,
}

impl PythonRelay {
    pub fn start(certs: &Certificates, backend: SocketAddr, node: &NodeIdentity) -> Self {
        Self::start_gate(certs, backend, node, false)
    }

    pub fn start_unload_gate(
        certs: &Certificates,
        backend: SocketAddr,
        node: &NodeIdentity,
    ) -> Self {
        Self::start_gate(certs, backend, node, true)
    }

    fn start_gate(
        certs: &Certificates,
        backend: SocketAddr,
        node: &NodeIdentity,
        unload_gate: bool,
    ) -> Self {
        let files = tempfile::tempdir().unwrap();
        // macOS /var is a symlink; the same original private_path contract
        // requires canonical file paths on both supported development OSes.
        let root = files.path().canonicalize().unwrap();
        let operator = root.join("operator");
        let control = root.join("control");
        for directory in [&operator, &control] {
            fs::create_dir(directory).unwrap();
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let receipt = material_files(certs, &operator);
        let port = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = port.local_addr().unwrap();
        drop(port);
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let child = Command::new("bash")
            .arg(repo.join("scripts/python-entry.sh"))
            .arg(repo.join("tests/system/_internal_relay_tls_peer.py"))
            .current_dir(repo)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut peer = Self {
            child,
            output: None,
            _files: files,
            address,
        };
        let value = serde_json::json!({
            "operator": operator, "control": control, "receipt": receipt,
            "listen_port": address.port(), "backend_port": backend.port(), "node": node,
            "unload_gate": unload_gate,
        });
        let input = peer.child.stdin.as_mut().unwrap();
        serde_json::to_writer(&mut *input, &value).unwrap();
        input.write_all(b"\n").unwrap();
        input.flush().unwrap();
        peer.output = Some(BufReader::new(peer.child.stdout.take().unwrap()));
        assert_eq!(peer.read_line(), b"READY\n");
        peer
    }

    fn read_line(&mut self) -> Vec<u8> {
        let mut stdout = self.output.take().unwrap();
        let (send, receive) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut line = Vec::new();
            let result = (&mut stdout)
                .take(2 * 1024 * 1024 + 1)
                .read_until(b'\n', &mut line);
            let _ = send.send((result, line, stdout));
        });
        let (result, line, stdout) = receive.recv_timeout(Duration::from_secs(10)).unwrap();
        result.unwrap();
        reader.join().unwrap();
        self.output = Some(stdout);
        assert!(line.len() <= 2 * 1024 * 1024 && line.ends_with(b"\n"));
        line
    }

    fn gate_command(&mut self, command: &[u8]) -> serde_json::Value {
        let input = self.child.stdin.as_mut().unwrap();
        input.write_all(command).unwrap();
        input.flush().unwrap();
        serde_json::from_slice(&self.read_line()).unwrap()
    }

    pub fn wait_unload(&mut self) -> serde_json::Value {
        self.gate_command(b"WAIT-UNLOAD\n")
    }

    pub fn discard_unload(&mut self) -> serde_json::Value {
        self.gate_command(b"DROP-UNLOAD\n")
    }

    pub fn stop(&mut self) {
        self.child
            .stdin
            .take()
            .unwrap()
            .write_all(b"STOP\n")
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "owned Python relay failed");
                return;
            }
            assert!(
                Instant::now() < deadline,
                "owned Python relay shutdown expired"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for PythonRelay {
    fn drop(&mut self) {
        // Panic/cancellation still stops only our exact Child. No global PID
        // matching or deployed service/material cleanup is performed.
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn material_files(
    certs: &Certificates,
    operator: &Path,
) -> serde_json::Map<String, serde_json::Value> {
    let payloads = [
        ("ca.der", certs.ca.der().to_vec()),
        ("center.der", certs.center.certificate.der().to_vec()),
        ("center-key.der", certs.center.key.serialize_der()),
        ("host-runtime.der", certs.host.certificate.der().to_vec()),
        ("host-runtime-key.der", certs.host.key.serialize_der()),
    ];
    let mut receipt = serde_json::Map::new();
    for (name, data) in payloads {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(operator.join(name))
            .unwrap();
        file.write_all(&data).unwrap();
        file.sync_all().unwrap();
        receipt.insert(name.into(), format!("{:x}", Sha256::digest(&data)).into());
    }
    receipt
}
