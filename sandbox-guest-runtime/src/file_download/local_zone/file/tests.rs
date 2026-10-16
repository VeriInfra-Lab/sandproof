use super::*;
use crate::file_download::date::LocalZone;
use std::{
    io::Cursor,
    os::unix::{ffi::OsStringExt, fs::symlink},
    sync::mpsc,
    time::Duration,
};

fn material() -> Vec<u8> {
    let mut bytes = b"TZif\0".to_vec();
    bytes.extend_from_slice(&[0; 15]);
    for count in [0_u32, 0, 0, 0, 1, 4] {
        bytes.extend_from_slice(&count.to_be_bytes());
    }
    bytes.extend_from_slice(&3661_i32.to_be_bytes());
    bytes.extend_from_slice(&[0, 0]);
    bytes.extend_from_slice(b"XYZ\0");
    bytes
}
fn limits() -> LoadLimits {
    LoadLimits {
        path_bytes: 4096,
        snapshot: Limits {
            input_bytes: material().len(),
            transitions: 1,
            zones: 1,
        },
    }
}

#[test]
fn actual_regular_file_and_trusted_symlink_make_independent_snapshots() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("zone");
    let link = root.path().join("localtime");
    std::fs::write(&path, material()).unwrap();
    symlink("zone", &link).unwrap();
    for source in [&path, &link] {
        let snapshot = Snapshot::load_file(
            source,
            LoadLimits {
                path_bytes: source.as_os_str().len(),
                ..limits()
            },
        )
        .unwrap();
        assert_eq!(snapshot.offset(b"XYZ", 0), Some(3661));
        assert_eq!(snapshot.offset(b"OLD", 0), None);
    }
    let snapshot = Snapshot::load_file(&link, limits()).unwrap();
    std::fs::write(&path, b"replaced material").unwrap();
    assert_eq!(snapshot.offset(b"XYZ", 0), Some(3661));
    assert!(matches!(
        Snapshot::load_file(&link, limits()),
        Err(LoadError::Snapshot(Error::InvalidData))
    ));
    std::fs::remove_file(&path).unwrap();
    assert!(matches!(
        Snapshot::load_file(&link, limits()),
        Err(LoadError::Io(io::ErrorKind::NotFound))
    ));
    assert_eq!(snapshot.offset(b"XYZ", 0), Some(3661));
}

#[test]
fn budgets_and_paths_reject_before_open_and_nonregular_sources_never_parse() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing");
    for field in 0..4 {
        for value in [0, usize::MAX] {
            let mut caps = limits();
            match field {
                0 => caps.path_bytes = value,
                1 => caps.snapshot.input_bytes = value,
                2 => caps.snapshot.transitions = value,
                _ => caps.snapshot.zones = value,
            }
            assert!(matches!(
                Snapshot::load_file(&missing, caps),
                Err(LoadError::InvalidConfiguration)
            ));
        }
    }
    assert!(matches!(
        Snapshot::load_file(
            &missing,
            LoadLimits {
                path_bytes: missing.as_os_str().len() - 1,
                ..limits()
            }
        ),
        Err(LoadError::InvalidPath)
    ));
    for path in [Path::new("relative"), Path::new("")] {
        assert!(matches!(
            Snapshot::load_file(path, limits()),
            Err(LoadError::InvalidPath)
        ));
    }
    let nul = std::ffi::OsString::from_vec(b"/bad\0path".to_vec());
    assert!(matches!(
        Snapshot::load_file(Path::new(&nul), limits()),
        Err(LoadError::InvalidPath)
    ));
    assert!(!missing.exists());
    assert!(matches!(
        Snapshot::load_file(root.path(), limits()),
        Err(LoadError::NotRegular)
    ));
    std::fs::write(&missing, vec![0; limits().snapshot.input_bytes + 1]).unwrap();
    assert!(matches!(
        Snapshot::load_file(&missing, limits()),
        Err(LoadError::Snapshot(Error::InputLimit))
    ));
    std::fs::write(&missing, []).unwrap();
    assert!(matches!(
        Snapshot::load_file(&missing, limits()),
        Err(LoadError::Snapshot(Error::InvalidData))
    ));
}

#[test]
fn fifo_without_writer_is_refused_without_waiting_for_a_peer() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fifo");
    // rustix does not expose mkfifoat on macOS; use the Unix fixture tool on
    // both platforms so this rejection is also exercised by portable checks.
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let source = path.clone();
    let (send, receive) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = Snapshot::load_file(&source, limits()).map(|_| ());
        send.send(result).unwrap();
    });
    let result = receive.recv_timeout(Duration::from_secs(5));
    if result.is_err() {
        // Release a mistakenly blocking open before joining the failed test.
        let _ = rustix::fs::open(&path, OFlags::WRONLY | OFlags::NONBLOCK, Mode::empty());
    }
    worker.join().unwrap();
    assert_eq!(result.unwrap(), Err(LoadError::NotRegular));
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn opened_source_is_retained_when_the_original_name_is_replaced() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("zone");
    let retired = root.path().join("retired");
    std::fs::write(&path, material()).unwrap();
    let original = File::open(&path).unwrap();
    std::fs::rename(&path, &retired).unwrap();
    std::fs::write(&path, b"invalid replacement").unwrap();
    let snapshot = from_file(original, limits().snapshot).unwrap();
    assert_eq!(snapshot.offset(b"XYZ", 0), Some(3661));
    assert!(matches!(
        Snapshot::load_file(&path, limits()),
        Err(LoadError::Snapshot(Error::InvalidData))
    ));
}

struct Reader {
    input: Cursor<Vec<u8>>,
    chunk: usize,
    interrupted: bool,
    fail_at: Option<u64>,
    requests: Vec<usize>,
}
impl Reader {
    fn new(bytes: Vec<u8>, chunk: usize) -> Self {
        Self {
            input: Cursor::new(bytes),
            chunk,
            interrupted: true,
            fail_at: None,
            requests: vec![],
        }
    }
}
impl Read for Reader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.requests.push(output.len());
        if self.interrupted {
            self.interrupted = false;
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.fail_at.is_some_and(|at| self.input.position() >= at) {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let count = output.len().min(self.chunk);
        self.input.read(&mut output[..count])
    }
}

#[test]
fn short_reads_retry_interrupted_and_require_actual_eof_at_the_exact_budget() {
    for chunk in [1, 7, 4096] {
        let mut source = Reader::new(material(), chunk);
        let snapshot = read_snapshot(&mut source, limits().snapshot).unwrap();
        assert_eq!(snapshot.offset(b"XYZ", 0), Some(3661));
        assert_eq!(source.requests.last(), Some(&1));
        assert!(source.requests.iter().all(|size| *size <= 4096));
        assert_eq!(source.input.position(), material().len() as u64);
    }
    let mut source = Reader::new(material(), 7);
    source.fail_at = Some(material().len() as u64);
    assert!(matches!(
        read_snapshot(&mut source, limits().snapshot),
        Err(LoadError::Io(io::ErrorKind::PermissionDenied))
    ));
    let mut source = Reader::new(material(), 1);
    source.fail_at = Some(3);
    assert!(matches!(
        read_snapshot(&mut source, limits().snapshot),
        Err(LoadError::Io(io::ErrorKind::PermissionDenied))
    ));
}

#[test]
fn growth_and_truncation_never_become_a_partial_or_fallback_snapshot() {
    let mut data = material();
    data.extend_from_slice(&[0; 8192]);
    let mut source = Reader::new(data, 4096);
    assert!(matches!(
        read_snapshot(&mut source, limits().snapshot),
        Err(LoadError::Snapshot(Error::InputLimit))
    ));
    assert_eq!(
        source.input.position(),
        limits().snapshot.input_bytes as u64 + 1
    );
    let data = material();
    for end in [0, 4, 43, data.len() - 1] {
        let mut source = Reader::new(data[..end].to_vec(), 7);
        assert!(matches!(
            read_snapshot(&mut source, limits().snapshot),
            Err(LoadError::Snapshot(Error::InvalidData))
        ));
    }
}
