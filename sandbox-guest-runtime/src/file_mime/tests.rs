use super::*;
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};
pub(super) fn limits() -> CatalogLimits {
    CatalogLimits {
        input_bytes: 65536,
        line_bytes: 1024,
        entries: 512,
        retained_bytes: 65536,
        value_bytes: 1024,
    }
}
#[test]
fn disposition_keeps_unix_basename_quoting_and_rfc2231_bytes_under_exact_budget() {
    for (path, expected) in [
        (b"".as_slice(), "inline; filename=."),
        (b"///", "inline; filename=\"/\""),
        (b"/dir/a b/", "inline; filename=\"a b\""),
        (b"/dir/a\\b\"", "inline; filename=\"a\\\\b\\\"\""),
        (
            "/dir/雪.txt".as_bytes(),
            "inline; filename*=utf-8''%E9%9B%AA.txt",
        ),
        (b"/dir/\xff\n", "inline; filename*=utf-8''%FF%0A"),
        (b"/dir/a\tb", "inline; filename=\"a\tb\""),
    ] {
        assert_eq!(
            disposition(path, 1024, expected.len()).unwrap(),
            expected.as_bytes()
        );
        assert_eq!(
            disposition(path, 1024, expected.len() - 1),
            Err(Error::RetainedLimit)
        );
    }
    assert_eq!(disposition(b"/x", 1, 128), Err(Error::InputLimit));
    assert_eq!(extension("/a.b/c.TXT/"), "");
    assert_eq!(extension("/a.b/.env"), ".env");
    assert_eq!(extension("/a.b/c\\name.foo"), ".foo");
}
#[test]
fn catalog_keeps_first_globs_builtin_priority_exact_case_and_fallback_order() {
    let globs = b"50:text/custom:*.txt\n50:application/custom:*.TXT\n50:text/custom:*.mine\n10:application/ignored:*.mine\n50:broken:*.bad\n50:text/no:*.[ab]\n";
    let catalog = Catalog::from_sources(
        [Some(globs), None],
        [Some(b"text/ignored mine"), None, None, None],
        limits(),
    )
    .unwrap();
    assert_eq!(
        catalog.lookup(".txt").unwrap(),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(catalog.lookup(".TXT").unwrap(), Some("application/custom"));
    assert_eq!(catalog.lookup(".TxT").unwrap(), Some("application/custom"));
    assert_eq!(
        catalog.lookup(".mine").unwrap(),
        Some("text/custom; charset=utf-8")
    );
    let catalog = Catalog::from_sources(
        [None, None],
        [
            Some(b"text/first MiNe\napplication/x-override txt"),
            None,
            Some(b"text/second mine # comment"),
            None,
        ],
        limits(),
    )
    .unwrap();
    assert_eq!(
        catalog.lookup(".mine").unwrap(),
        Some("text/second; charset=utf-8")
    );
    assert_eq!(
        catalog.lookup(".MiNe").unwrap(),
        Some("text/first; charset=utf-8")
    );
    assert_eq!(
        catalog.lookup(".txt").unwrap(),
        Some("application/x-override")
    );
    let empty = Catalog::from_sources(
        [Some(b""), Some(globs)],
        [Some(b"text/ignored txt"), None, None, None],
        limits(),
    )
    .unwrap();
    assert_eq!(empty.lookup(".mine").unwrap(), None);
}
#[test]
fn sniff_uses_only_prefix_with_original_whitespace_bom_and_mp4_order() {
    assert_eq!(detect(b""), "text/plain; charset=utf-8");
    assert_eq!(detect(b"\t\n\x0c <hTmL>"), "text/html; charset=utf-8");
    assert_eq!(detect(b"<html\t>"), "text/plain; charset=utf-8");
    assert_eq!(detect(b"\xff\xfe"), "text/plain; charset=utf-8");
    assert_eq!(detect(b"\xff\xfeAB"), "text/plain; charset=utf-16le");
    assert_eq!(detect(b"\0\0\0\x18ftypisom\0\0\0\0mp42mp41"), "video/mp4");
    let mut data = vec![b'a'; 512];
    data.push(0);
    assert_eq!(detect(&data), "text/plain; charset=utf-8");
    data[511] = 0;
    assert_eq!(detect(&data), "application/octet-stream");
}
#[test]
fn catalog_limits_refuse_partial_material_and_builtins_are_charged() {
    assert!(matches!(
        Catalog::from_sources(
            [None; 2],
            [None; 4],
            CatalogLimits {
                line_bytes: 65536,
                ..limits()
            }
        ),
        Err(Error::InvalidConfiguration)
    ));
    assert!(matches!(
        Catalog::from_sources(
            [None, None],
            [None; 4],
            CatalogLimits {
                entries: 1,
                ..limits()
            }
        ),
        Err(Error::RetainedLimit)
    ));
    assert!(matches!(
        Catalog::from_sources(
            [Some(b"12345"), None],
            [None; 4],
            CatalogLimits {
                line_bytes: 4,
                ..limits()
            }
        ),
        Err(Error::InputLimit)
    ));
    assert!(matches!(
        Catalog::from_sources(
            [None, None],
            [Some(b"1234"), Some(b"5678"), None, None],
            CatalogLimits {
                input_bytes: 7,
                ..limits()
            }
        ),
        Err(Error::InputLimit)
    ));
    assert!(matches!(
        Catalog::from_sources([Some(b"\xff"), None], [None; 4], limits()),
        Err(Error::UnsupportedText)
    ));
    assert!(matches!(
        Catalog::from_sources(
            [Some(b"50:text/plain;charset*=utf-8''%FF:*.broken"), None],
            [None; 4],
            limits()
        ),
        Err(Error::UnsupportedText)
    ));
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        write!(s, "{b:02x}").unwrap();
        s
    })
}
struct Reference {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
}
impl Reference {
    fn new() -> Self {
        let mut child = Command::new(
            std::env::var_os("SANDPROOF_GUEST_FILE_MIME_REFERENCE").expect("Go reference required"),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
        Self {
            input: child.stdin.take(),
            output: BufReader::new(child.stdout.take().unwrap()),
            child,
        }
    }
    fn check(&mut self, request: &str, observed: &[u8]) {
        let input = self.input.as_mut().unwrap();
        writeln!(input, "{request}").unwrap();
        input.flush().unwrap();
        let mut value = String::new();
        assert!(self.output.read_line(&mut value).unwrap() > 0);
        assert_eq!(
            hex(observed),
            value.trim_end_matches('\n'),
            "oracle input {request}"
        );
    }
    fn finish(&mut self) {
        drop(self.input.take());
        assert!(self.child.wait().unwrap().success());
    }
}
impl Drop for Reference {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn step(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    *seed
}

#[test]
#[ignore = "explicit Go 1.26.8 MIME oracle; set SANDPROOF_GUEST_FILE_MIME_REFERENCE"]
fn fixed_go_file_mime_reference() {
    let mut oracle = Reference::new();
    let mut samples = vec![vec![], vec![b'a'; 513], vec![0; 512]];
    for sig in sniff::SIGNATURES {
        let bytes = match sig {
            sniff::Sig::Html(bytes) => [bytes.to_vec(), b">content".to_vec()].concat(),
            sniff::Sig::Exact(bytes, _) => bytes.to_vec(),
            sniff::Sig::Masked { pattern, .. } => pattern.to_vec(),
            _ => continue,
        };
        for n in 0..=bytes.len() {
            samples.push(bytes[..n].to_vec());
        }
        samples.push([b" \t\n\x0c".to_vec(), bytes.clone()].concat());
        for i in 0..bytes.len() {
            for b in [0, b' ', b'a', b'Z', 255] {
                let mut value = bytes.clone();
                value[i] = b;
                samples.push(value);
            }
        }
    }
    for size in [0_u32, 4, 8, 12, 16, 20, 24, 32, 512, 516, u32::MAX] {
        let mut mp4 = size.to_be_bytes().to_vec();
        mp4.extend(b"ftypmp42mp41mp4aisom");
        samples.push(mp4);
    }
    let mut seed = 0x311;
    for _ in 0..512 {
        let len = usize::try_from(step(&mut seed) % 600).unwrap();
        samples.push((0..len).map(|_| step(&mut seed).to_le_bytes()[3]).collect());
    }
    for data in &samples {
        oracle.check(&format!("S\t{}", hex(data)), detect(data).as_bytes());
    }
    let mut paths = vec![
        vec![],
        b"///".to_vec(),
        b"/a/b/../".to_vec(),
        "/dir/雪.txt".as_bytes().to_vec(),
    ];
    for byte in 0..=255 {
        paths.push([b"/d/a".as_slice(), &[byte], b".txt/"].concat());
    }
    for _ in 0..256 {
        let len = usize::try_from(step(&mut seed) % 96).unwrap();
        paths.push((0..len).map(|_| step(&mut seed).to_le_bytes()[4]).collect());
    }
    for path in &paths {
        oracle.check(
            &format!("D\t{}", hex(path)),
            &disposition(path, 4096, 16384).unwrap(),
        );
    }
    for path in [
        "", "/", ".", "..", "/a.b/c", "/a.b/c.", "a\\b.txt", "/a/.env", "/a.TXT/", "雪.İ",
    ] {
        oracle.check(
            &format!("E\t{}", hex(path.as_bytes())),
            extension(path).as_bytes(),
        );
    }
    let catalog_cases = check_catalogs(&mut oracle);
    oracle.finish();
    eprintln!(
        "Fixed Go MIME: {} sniff inputs, {} dispositions, 10 extensions, {catalog_cases} catalog lookups.",
        samples.len(),
        paths.len()
    );
}
fn check_catalogs(oracle: &mut Reference) -> usize {
    let configs: [[Option<&[u8]>;6];6] = [
        [None;6],
        [Some("50:\u{212a}/text:*.kelvin\n50:text/x; name*=utf-8''%E9%9B%AA:*.encoded".as_bytes()), None, None, None, None, None],
        [Some(b""),Some(b"50:text/custom:*.mine"),Some(b"text/ignored txt"),None,None,None],
        [Some(b"50:text/custom:*.txt\n50:application/up:*.TXT\n50:text/one:*.mine\n10:text/two:*.mine\n50:TEXT/plain:*.upper\n50:text/plain; charset=\"\":*.empty\n50:text/no:*.[ab]\n50:application/json:*.\xc4\xb0"),None,None,None,None,None],
        [None,Some(b"50:text/second:*.mine"),None,None,None,None],
        [None,None,Some(b"text/first MiNe\napplication/x-override txt\ninvalid; mine\nTEXT/plain upper\napplication/json \xc4\xb0\ntext/plain;charset=utf-16 wide"),Some(b"text/second mine # ignored"),Some(b"text/third\tmine\r\n"),None],
    ];
    let mut count = 0;
    for input in configs {
        let catalog = Catalog::from_sources(
            [input[0], input[1]],
            [input[2], input[3], input[4], input[5]],
            limits(),
        )
        .unwrap();
        let mut extensions: Vec<&str> = builtins::TYPES.iter().map(|(ext, _)| *ext).collect();
        extensions.extend([
            ".mine", ".MiNe", ".MINE", ".txt", ".TXT", ".TxT", ".upper", ".empty", ".wide", ".İ",
            ".i", ".I", ".unknown", "", ".kelvin", ".encoded",
        ]);
        for ext in extensions {
            let mut line = format!("C\t{}", hex(ext.as_bytes()));
            for file in input {
                line.push('\t');
                line.push_str(&file.map_or_else(|| "-".into(), hex));
            }
            oracle.check(&line, catalog.lookup(ext).unwrap().unwrap_or("").as_bytes());
            count += 1;
        }
    }
    count
}
