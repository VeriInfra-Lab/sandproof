//! Numeric guards, proc byte fixtures and one owned launch-gate child.
//! None of these fixtures asserts actual VMM/XSAVE or CPU-epoch continuity.
use super::*;
use crate::linux::PROCESS_TEST_LOCK;
use std::{process::{Child,Command,Stdio},os::unix::fs::OpenOptionsExt};

#[test]
fn canonical_domains_and_linear_subset_preserve_every_cpu() {
    assert!(covered(&[0,2,7],&[2,7]));
    assert!(covered(&[0],&[0]));
    assert!(covered(&[0,2,7],&[0,2,7]));
    for (allowed,observed) in [(vec![0,2,7],vec![1]),(vec![0,2,7],vec![2,8]),
        (vec![0,2],vec![0,2,3]),(vec![],vec![0]),(vec![0],vec![]),
        (vec![0,0],vec![0]),(vec![0,2],vec![2,0]),(vec![8192],vec![8192])] {
        assert!(!covered(&allowed,&observed));
    }
    assert!(validate(&(0..256).collect::<Vec<_>>()).is_ok());
    assert!(validate(&(0..257).collect::<Vec<_>>()).is_err());
}

#[test]
fn kernel_cpu_ranges_and_identity_fields_refuse_missing_duplicate_and_bad_values() {
    let good=b"Name:\tvm\xff\nTgid:\t10\nPid:\t11\nCpus_allowed_list:\t0-2,7\n";
    assert_eq!(status(good,10,11).unwrap(),vec![0,1,2,7]);
    assert!(status(good,10,12).is_err());
    assert!(status(good,12,11).is_err());
    for bad in [b"".as_slice(),b"Tgid:10\nPid:11\n",b"Tgid:10\nPid:11\nCpus_allowed_list:0\nPid:11\n",
        b"Tgid:10\nTgid:10\nPid:11\nCpus_allowed_list:0\n",
        b"Tgid:10\nPid:11\nCpus_allowed_list:0\nCpus_allowed_list:0\n"] {
        assert!(status(bad,10,11).is_err());
    }
    for bad in ["", "1-0", "0,0", "2,1", "0-2,2", "01", "+1", "8192", "0-8191", "0-2-3"] {
        assert!(cpu_list(bad.as_bytes()).is_err(),"accepted {bad}");
    }
}

fn stat(thread: u32,ticks:u64)->Vec<u8> {
    let mut bytes=format!("{thread} (").into_bytes();
    bytes.extend_from_slice(b"task\xff)");
    bytes.extend_from_slice(format!(" S 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 {ticks} 0").as_bytes());
    bytes
}
#[test]
fn thread_start_uses_exact_pid_and_ticks_without_assuming_utf8_comm() {
    assert_eq!(start(&stat(11,13),11).unwrap(),13);
    assert!(start(&stat(12,13),11).is_err());
    assert!(start(&stat(11,0),11).is_err());
    assert!(start(b"11 (task) S 0",11).is_err());
}

struct GateChild(Child);
impl Drop for GateChild {
    fn drop(&mut self) {let _=self.0.kill();let _=self.0.wait();}
}
#[test]
fn owned_child_domain_is_measured_before_gate_release_and_changed_source_refuses() {
    let _lock=PROCESS_TEST_LOCK.lock().unwrap();
    let child=GateChild(Command::new("/bin/sh").args(["-c","IFS= read -r release"])
        .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
    let source=RuntimeProcessIdentity::capture(child.0.id()).unwrap();
    let actual=launch_domain(&source).unwrap();
    validate(&actual).unwrap();
    let threads=original_threads(&source).unwrap();
    assert_eq!(threads.len(),1);
    assert_eq!(threads.get(&source.pid).unwrap().cpus,actual);
    let mut wrong=source.clone();wrong.start_time_ticks+=1;
    assert!(launch_domain(&wrong).is_err());
    wrong=source;
    wrong.boot_id=if wrong.boot_id=="ffffffff-ffff-ffff-ffff-ffffffffffff" {
        "00000000-0000-0000-0000-000000000000"
    } else { "ffffffff-ffff-ffff-ffff-ffffffffffff" }.into();
    assert!(launch_domain(&wrong).is_err());
}

#[test]
fn bounded_metadata_reads_reject_oversize_and_symlink() {
    let dir=tempfile::tempdir().unwrap();
    let path=dir.path().join("status");
    let mut options=std::fs::OpenOptions::new();
    options.write(true).create_new(true).mode(0o600);
    use std::io::Write;
    options.open(&path).unwrap().write_all(b"0123456789").unwrap();
    assert_eq!(read(&path,10).unwrap(),b"0123456789");
    assert!(read(&path,9).is_err());
    let link=dir.path().join("link");
    std::os::unix::fs::symlink(&path,&link).unwrap();
    assert!(read(&link,10).is_err());
    assert!(read(dir.path(),10).is_err());
}

#[test]
fn original_launch_and_nonleader_threads_each_remain_separate_required_facts() {
    let mut threads=BTreeMap::from([
        (10,Thread{start:13,cpus:vec![0]}),
        (11,Thread{start:17,cpus:vec![0,2]}),
    ]);
    assert!(require_covered_threads(&[0,2],&[0,2],&threads).is_ok());
    assert!(require_covered_threads(&[0,2],&[0,2,7],&threads).is_err());
    threads.get_mut(&11).unwrap().cpus.push(7);
    assert!(require_covered_threads(&[0,2],&[0,2],&threads).is_err());
    assert!(require_covered_threads(&[0,2],&[0,2],&BTreeMap::new()).is_err());
}

#[test]
fn an_expired_observation_is_unknown_instead_of_an_empty_success() {
    assert!(snapshot(std::process::id(),Instant::now()).is_err());
}
