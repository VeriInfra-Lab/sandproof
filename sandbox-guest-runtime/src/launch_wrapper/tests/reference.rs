use super::*;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Oracle(Child, Option<ChildStdin>, BufReader<ChildStdout>);
impl Drop for Oracle {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
impl Oracle {
    fn new() -> Self {
        let mut child = Command::new(
            std::env::var_os("SANDPROOF_GUEST_CONNECT_REFERENCE")
                .expect("fixed Go reference required"),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Self(child, Some(input), output)
    }
    fn check(&mut self, input: &serde_json::Value, expected: &serde_json::Value) {
        let stdin = self.1.as_mut().unwrap();
        write!(stdin, "L\t").unwrap();
        for byte in serde_json::to_vec(input).unwrap() {
            write!(stdin, "{byte:02x}").unwrap();
        }
        writeln!(stdin).unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        assert!(self.2.read_line(&mut line).unwrap() > 0);
        assert_eq!(
            &serde_json::from_str::<serde_json::Value>(&line).unwrap(),
            expected,
            "{input}"
        );
    }
}

#[test]
#[ignore = "explicit fixed Go wrapper construction; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
fn fixed_go_optional_helpers_priority_and_lexical_candidates_match() {
    let mut oracle = Oracle::new();
    let mut count = 0;
    for ionice in [None, Some("/bin/ionice"), Some("/opt/a b/ionice")] {
        for nice in [None, Some("/bin/nice"), Some("/opt/$value/nice")] {
            for current in [-20, -1, 0, 19] {
                for directory in [
                    "",
                    ".",
                    "/",
                    "/bin/../sbin//",
                    "bin/../",
                    "../bin/..",
                    "../../",
                    "/../../bin",
                ] {
                    let input = serde_json::json!({"Ionice":ionice,"Nice":nice,"CurrentNice":current,"Directory":directory,"Name":"nice"});
                    oracle.check(
                        &input,
                        &serde_json::json!({
                            "Script":script(ionice, nice, -current, 4096).unwrap(),
                            "Candidate":candidate(directory, "nice")
                        }),
                    );
                    count += 1;
                }
            }
        }
    }
    drop(oracle.1.take());
    assert!(oracle.0.wait().unwrap().success());
    eprintln!("{count} fixed Go wrapper construction cases; no original handler execution.");
}
