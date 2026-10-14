//! Opt-in comparison with original Go endpoint methods, not an SDK/HTTP or VM
//! acceptance. The pinned reference adapter is built using a source overlay.
#![cfg(target_os = "linux")]
use sandproof_guest_runtime::{
    file_protocol::{Request, encode_response},
    filesystem::{FileLimits, Ownership, execute},
};
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::os::unix::{
    ffi::OsStrExt,
    fs::{PermissionsExt, symlink},
};
use std::path::Path;
use std::process::{Command, Stdio};

fn prepare(root: &Path) {
    if root.exists() {
        fs::remove_dir_all(root).unwrap();
    }
    fs::create_dir_all(root.join("dir/sub")).unwrap();
    fs::create_dir(root.join("invalid")).unwrap();
    fs::write(
        root.join("invalid")
            .join(std::ffi::OsStr::from_bytes(b"bad\xff")),
        b"",
    )
    .unwrap();
    for (name, value) in [
        ("file", b"\0\xffdata".as_slice()),
        ("dir/a", b"a"),
        ("replacement", b"old"),
        ("epoch", b""),
    ] {
        fs::write(root.join(name), value).unwrap();
    }
    fs::set_permissions(root.join("file"), fs::Permissions::from_mode(0o6750)).unwrap();
    for (name, target) in [
        ("link", "file"),
        ("alias", "dir"),
        ("broken", "missing"),
        ("loop", "loop"),
        ("dir/child-link", "../dir"),
    ] {
        symlink(target, root.join(name)).unwrap();
    }
    rustix::fs::mkfifoat(rustix::fs::CWD, root.join("fifo"), rustix::fs::Mode::RUSR).unwrap();
    rustix::fs::setxattr(
        root.join("file"),
        "user.e2b.note",
        b"snow \xe9\x9b\xaa",
        rustix::fs::XattrFlags::empty(),
    )
    .unwrap();
    rustix::fs::setxattr(
        root.join("file"),
        "user.e2b.bad",
        b"\xff",
        rustix::fs::XattrFlags::empty(),
    )
    .unwrap();
    for name in [
        "file",
        "link",
        "dir",
        "dir/sub",
        "dir/a",
        "dir/child-link",
        "alias",
        "broken",
        "loop",
        "fifo",
        "replacement",
        "epoch",
        "invalid",
    ] {
        let times = rustix::fs::Timestamps {
            last_access: rustix::fs::Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            last_modification: rustix::fs::Timespec {
                tv_sec: if name == "epoch" { 0 } else { 1_700_000_000 },
                tv_nsec: if name == "epoch" { 0 } else { 123_456_789 },
            },
        };
        rustix::fs::utimensat(
            rustix::fs::CWD,
            root.join(name),
            &times,
            rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
        )
        .unwrap();
    }
}

fn prepare_case(root: &Path, request: &Request) {
    prepare(root);
    if let Request::Remove(path) = request
        && path.ends_with("/unreadable")
    {
        let child = Path::new(path).join("empty");
        fs::create_dir_all(&child).unwrap();
        fs::set_permissions(child, fs::Permissions::from_mode(0o000)).unwrap();
    }
}

fn normalize(value: &mut Value, mutation: bool) {
    match value {
        Value::Object(fields) => {
            if fields.get("metadata") == Some(&json!({})) {
                fields.remove("metadata");
            }
            if mutation {
                fields.remove("modifiedTime");
            }
            for v in fields.values_mut() {
                normalize(v, mutation);
            }
        }
        Value::Array(values) => {
            for v in values {
                normalize(v, mutation);
            }
        }
        _ => {}
    }
}

fn go(binary: &Path, input: &Value) -> Value {
    let mut process = Command::new(binary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut stdin = process.stdin.take().unwrap();
    writeln!(stdin, "{input}").unwrap();
    drop(stdin);
    let output = process.wait_with_output().unwrap();
    assert!(output.status.success());
    serde_json::from_slice(&output.stdout).unwrap()
}

fn rust(request: &Request) -> Value {
    match execute(
        request,
        Ownership {
            uid: rustix::process::geteuid().as_raw(),
            gid: rustix::process::getegid().as_raw(),
        },
        FileLimits {
            path_bytes: 4096,
            entries: 4096,
            retained_bytes: 1_048_576,
            visited: 16384,
            xattr_bytes: 65536,
            account_bytes: 1_048_576,
        },
    ) {
        Ok(response) => {
            serde_json::from_slice(&encode_response(response.as_wire(), 4_194_304).unwrap())
                .unwrap()
        }
        Err(error) => json!({"error":error.code()}),
    }
}

fn observe_effects(root: &Path) -> Value {
    let mut rows = Vec::new();
    let mut pending = vec![root.to_owned()];
    while let Some(path) = pending.pop() {
        let Ok(info) = fs::symlink_metadata(&path) else {
            continue;
        };
        if info.is_dir() {
            for entry in fs::read_dir(&path).unwrap() {
                pending.push(entry.unwrap().path());
            }
        }
        let content = if info.is_file() {
            Some(fs::read(&path).unwrap())
        } else if info.file_type().is_symlink() {
            Some(
                fs::read_link(&path)
                    .unwrap()
                    .as_os_str()
                    .as_bytes()
                    .to_vec(),
            )
        } else {
            None
        };
        rows.push((
            path.strip_prefix(root)
                .unwrap()
                .as_os_str()
                .as_bytes()
                .to_vec(),
            info.permissions().mode(),
            info.len(),
            content,
        ));
    }
    rows.sort();
    json!(rows)
}

fn cases(root: &Path) -> Vec<(String, Value, Request, bool)> {
    let path = |name: &str| format!("{}/{name}", root.display());
    let mut cases = Vec::new();
    for name in [
        "file", "link", "broken", "loop", "fifo", "epoch", "missing", "file/", "dir/.", "file\0",
    ] {
        cases.push((
            format!("Stat {name:?}"),
            json!({"Method":"Stat","Path":path(name)}),
            Request::Stat(path(name)),
            false,
        ));
    }
    for (name, depth) in [
        ("dir", 0),
        ("dir", 1),
        ("dir", 2),
        ("dir", u32::MAX),
        ("alias", 2),
        ("file", 1),
        ("broken", 1),
        ("loop", 1),
        ("invalid", 1),
        ("alias/..", 1),
    ] {
        cases.push((
            format!("ListDir {name} {depth}"),
            json!({"Method":"ListDir","Path":path(name),"Depth":depth}),
            Request::ListDir {
                path: path(name),
                depth,
            },
            false,
        ));
    }
    for name in [
        "new/nested",
        "dir",
        "file",
        "broken",
        "absent/../new",
        "file/child",
        "new/",
        "new\0",
    ] {
        cases.push((
            format!("MakeDir {name:?}"),
            json!({"Method":"MakeDir","Path":path(name)}),
            Request::MakeDir(path(name)),
            true,
        ));
    }
    for (source, destination) in [
        ("file", "new/target"),
        ("link", "new/moved"),
        ("missing", "partial/target"),
        ("file", "dir"),
        ("file", "replacement"),
    ] {
        cases.push((
            format!("Move {source} {destination}"),
            json!({"Method":"Move","Source":path(source),"Destination":path(destination)}),
            Request::Move {
                source: path(source),
                destination: path(destination),
            },
            false,
        ));
    }
    for name in [
        "dir",
        "unreadable",
        "alias",
        "broken",
        "missing",
        "file/child",
        "dir/.",
        "dir/./",
        "file/",
        "dir/",
        "dir\0",
    ] {
        cases.push((
            format!("Remove {name:?}"),
            json!({"Method":"Remove","Path":path(name)}),
            Request::Remove(path(name)),
            false,
        ));
    }
    cases
}

#[test]
#[ignore = "requires the exact pinned Go source/toolchain adapter; see testing-plan"]
fn original_go_and_rust_unary_file_operations() {
    let binary = std::env::var_os("SANDPROOF_GUEST_GO_FILES_REFERENCE")
        .expect("provide pinned Go adapter executable");
    let binary = Path::new(&binary);
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("fixture");
    let cases = cases(&root);
    let count = cases.len();
    for (name, input, request, mutation) in cases {
        prepare_case(&root, &request);
        let mut expected = go(binary, &input);
        let effects = observe_effects(&root);
        prepare_case(&root, &request);
        let mut actual = rust(&request);
        let actual_effects = observe_effects(&root);
        normalize(&mut expected, mutation);
        normalize(&mut actual, mutation);
        assert_eq!(actual, expected, "reference mismatch: {name}");
        assert_eq!(actual_effects, effects, "partial effects mismatch: {name}");
        println!("Go/Rust file component matched: {name}");
    }
    println!(
        "{count} original Go/Rust unary cases matched; fresh mutation mtime excluded, fixed read mtime exact; no SDK/HTTP/KVM"
    );
}
