use super::*;
use std::io;

// Real temporary-file prefix, with a controlled ENOSPC after one short write.
// Does not require filling the host filesystem or changing a device node.
struct FailAfterPrefix {
    file: File,
    calls: usize,
}
impl Write for FailAfterPrefix {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        match self.calls {
            1 => Err(io::Error::from(io::ErrorKind::Interrupted)),
            2 => self.file.write(&bytes[..1]),
            _ => Err(io::Error::from_raw_os_error(28)),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
#[test]
fn partial_failure_keeps_original_errno_prefix_and_rejects_replay() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("partial");
    let name = path.to_str().unwrap();
    let mut writer = FailAfterPrefix {
        file: File::create(&path).unwrap(),
        calls: 0,
    };
    let mut accounting = Transfer::new(32, 8, Some(3)).unwrap();
    assert_eq!(
        write_chunk_to(&mut writer, &mut accounting, name, b"abc"),
        Err(fault(Stage::Write, name, Some(LinuxErrno(28)), None))
    );
    assert_eq!(accounting.written_bytes(), 1);
    assert_eq!(fs::read(&path).unwrap(), b"a");
    assert_eq!(
        write_chunk_to(&mut writer, &mut accounting, name, b"bc"),
        Err(UploadError::Rejected(TransferIoError::Incomplete))
    );
    assert_eq!(writer.calls, 3);
    assert!(accounting.finish().is_err());
}
#[test]
fn actual_readonly_descriptor_failure_is_retained_without_changing_contents() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("readonly");
    fs::write(&path, b"original").unwrap();
    let mut file = File::open(&path).unwrap();
    let mut accounting = Transfer::new(32, 8, None).unwrap();
    let error =
        write_chunk_to(&mut file, &mut accounting, path.to_str().unwrap(), b"abc").unwrap_err();
    assert_eq!(
        error,
        fault(
            Stage::Write,
            path.to_str().unwrap(),
            Some(LinuxErrno(9)),
            None
        )
    );
    assert_eq!(accounting.written_bytes(), 0);
    assert_eq!(error.into_legacy(), TransferIoError::Internal);
    assert_eq!(fs::read(path).unwrap(), b"original");
}
#[test]
fn zero_write_is_distinct_from_errno_and_poisoned_for_later_input() {
    let mut output = [].as_mut_slice();
    let mut accounting = Transfer::new(32, 8, None).unwrap();
    assert_eq!(
        write_chunk_to(&mut output, &mut accounting, "/file", b"x"),
        Err(fault(Stage::WriteZero, "/file", None, None))
    );
    assert_eq!(
        write_chunk_to(&mut output, &mut accounting, "/file", b"x"),
        Err(UploadError::Rejected(TransferIoError::Incomplete))
    );
}
#[test]
fn metadata_list_error_retains_descriptor_errno_and_original_path() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("metadata");
    fs::write(&path, b"persisted").unwrap();
    // O_PATH is owned and valid, but cannot be used by flistxattr.
    let file = File::from(
        rustix::fs::open(
            &path,
            rustix::fs::OFlags::PATH | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .unwrap(),
    );
    let error =
        replace_metadata(&file, path.to_str().unwrap(), &BTreeMap::new(), 4096).unwrap_err();
    assert_eq!(
        error,
        fault(
            Stage::MetadataList,
            path.to_str().unwrap(),
            Some(LinuxErrno(9)),
            None
        )
    );
    assert_eq!(fs::read(path).unwrap(), b"persisted");
}
