use super::*;
use std::io::{BufRead, BufReader, Write};

fn limits() -> Limits {
    Limits {
        header_bytes: 65536,
        items: 1024,
    }
}
fn selected(value: &[u8]) -> Result<Encoding, Error> {
    download(value, false, limits())
}

#[test]
fn content_encoding_keeps_empty_semicolon_case_and_opaque_byte_rules() {
    for value in [b"".as_slice(), b"identity", b" IDENTITY ;q=0"] {
        assert_eq!(content(value, limits()), Ok(Encoding::Identity));
    }
    for value in [
        b"gzip".as_slice(),
        b"\xc2\xa0GZIP\xe3\x80\x80; Q=invalid",
        b"gz\xc4\xb0p",
    ] {
        assert_eq!(content(value, limits()), Ok(Encoding::Gzip));
    }
    for value in [
        b" ".as_slice(),
        b"*",
        b"gzip,identity",
        b"gzip\xff",
        b"\xc0\xa0gzip",
    ] {
        assert_eq!(content(value, limits()), Err(Error::Unsupported));
    }
}

#[test]
fn accepts_original_weights_wildcards_duplicates_and_identity_veto() {
    for (value, expected) in [
        ("", Encoding::Identity),
        ("br", Encoding::Identity),
        ("gzip", Encoding::Gzip),
        ("gzip;q=0", Encoding::Identity),
        ("identity;q=0,gzip", Encoding::Gzip),
        ("gzip;q=-1,br", Encoding::Gzip),
        ("gzip;q=bad;q=0", Encoding::Identity),
        ("gzip;q=0;q=bad", Encoding::Identity),
        ("gzip;q=0;q=.2", Encoding::Gzip),
        ("gzip;q= 0", Encoding::Gzip),
        ("*", Encoding::Identity),
        ("gzip;q=0,*;q=1,identity;q=0", Encoding::Gzip),
        ("*;q=0,identity;q=-1", Encoding::Identity),
        ("gzip;q=NaN,identity;q=0", Encoding::Gzip),
        ("gzip;q=NaN,identity;q=-Inf", Encoding::Identity),
        ("gzip;q=+NaN,identity;q=.5", Encoding::Gzip),
        ("gzip;q=0x1p1,identity;q=1", Encoding::Gzip),
        ("gzip;q=1e9999,identity;q=2", Encoding::Identity),
    ] {
        assert_eq!(selected(value.as_bytes()), Ok(expected), "{value}");
    }
    for value in ["*;q=0", "identity;q=0,br", "identity;q=0,gzip;q=-0"] {
        assert_eq!(selected(value.as_bytes()), Err(Error::NotAcceptable));
    }
    let value = b"identity;q=0,identity;q=-1";
    assert_eq!(selected(value), Ok(Encoding::Identity));
    assert!(!accept(value, limits()).unwrap().identity_acceptable);
    assert_eq!(download(value, true, limits()), Err(Error::NotAcceptable));
    assert_eq!(download(b"gzip", true, limits()), Ok(Encoding::Identity));
}

#[test]
fn byte_and_item_limits_precede_allocation_and_do_not_manufacture_selection() {
    assert_eq!(selected(b" , , "), Ok(Encoding::Identity));
    assert_eq!(
        accept(
            b"gzip,identity",
            Limits {
                items: 1,
                ..limits()
            }
        ),
        Err(Error::ItemLimit)
    );
    assert_eq!(
        content(
            b"gzip",
            Limits {
                header_bytes: 3,
                ..limits()
            }
        ),
        Err(Error::HeaderLimit)
    );
    assert_eq!(
        content(
            b"gzip",
            Limits {
                header_bytes: 4,
                ..limits()
            }
        ),
        Ok(Encoding::Gzip)
    );
    assert!(matches!(
        accept(
            b"",
            Limits {
                items: 0,
                ..limits()
            }
        ),
        Err(Error::InvalidConfiguration)
    ));
    assert!(matches!(
        content(
            b"",
            Limits {
                header_bytes: 0,
                ..limits()
            }
        ),
        Err(Error::InvalidConfiguration)
    ));
}

const LONG_TIE: &[u8] = b"gzip;q=1,br;q=0,gzip;q=1,br;q=0,br;q=1,gzip;q=1,br;q=1,br;q=0,br;q=2,br;q=0,identity;q=1,br;q=1,identity;q=1";

#[test]
fn long_quality_tie_uses_go_permutation_instead_of_the_first_supported_maximum() {
    // Direct fixed Go counterexample: stable sort or first-max selects gzip.
    assert_eq!(selected(LONG_TIE), Ok(Encoding::Identity));
}

fn quality_result(raw: &[u8]) -> String {
    match quality::parse(raw) {
        None => "ERR".into(),
        Some(value) if value.is_nan() => "NAN".into(),
        Some(value) => format!("{:016x}", value.to_bits()),
    }
}
#[test]
fn quality_retains_go_float_grammar_signed_zero_rounding_and_overflow_errors() {
    for (raw, expected) in [
        ("-0", -0.0),
        ("0x_1p0", 1.0),
        ("1_0e-1", 1.0),
        ("0x1.00000000000008p0", 1.0),
        ("0x1.0000000000000801p0", 1.0 + f64::EPSILON),
        ("0x1p-1074", f64::from_bits(1)),
        ("0x1p-1075", 0.0),
        ("-0x1p-1075", -0.0),
    ] {
        assert_eq!(
            quality_result(raw.as_bytes()),
            format!("{:016x}", expected.to_bits())
        );
    }
    for raw in [
        "", "++1", "--1", "-+1", "+NaN", "-NaN", "1e99999", "0x1p1024", "1__0", "1e_2", "0x.p1",
        "0x1", " 1", "1 ",
    ] {
        assert_eq!(quality_result(raw.as_bytes()), "ERR", "{raw}");
    }
}

fn step(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    *seed
}
fn pick(seed: &mut u64, length: usize) -> usize {
    usize::try_from(step(seed) % length as u64).unwrap()
}
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut output, byte| {
        write!(output, "{byte:02x}").unwrap();
        output
    })
}

struct Reference {
    child: std::process::Child,
    input: Option<std::process::ChildStdin>,
    output: BufReader<std::process::ChildStdout>,
}
impl Reference {
    fn new() -> Self {
        use std::process::{Command, Stdio};
        let mut child = Command::new(
            std::env::var_os("SANDPROOF_GUEST_ENCODING_REFERENCE").expect("reference required"),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
        let input = child.stdin.take();
        let output = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            input,
            output,
        }
    }
    fn check(&mut self, operation: &str, input: &[u8], expected: &str) {
        let writer = self.input.as_mut().unwrap();
        writeln!(writer, "{operation}\t{}", hex(input)).unwrap();
        writer.flush().unwrap();
        let mut line = String::new();
        assert!(self.output.read_line(&mut line).unwrap() > 0);
        assert_eq!(
            expected,
            line.trim_end(),
            "operation {operation}, input {}",
            hex(input)
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

#[test]
#[ignore = "explicit fixed envd / Go 1.26.8 oracle; set SANDPROOF_GUEST_ENCODING_REFERENCE"]
fn fixed_go_encoding_reference() {
    let mut oracle = Reference::new();
    let qualities = [
        "0", "-0", "1", "0.5", "2", "-1", ".5", "0x1p-1", "NaN", "Inf", "-Inf", "+NaN", "bad",
        "1e9999", "1_0", "0x_1p0", " 0", "1.",
    ];
    let mut seed = 0x311_u64;
    let numbers = quality_cases(&qualities, &mut seed);
    for number in &numbers {
        oracle.check("Q", number, &quality_result(number));
    }
    let mut headers: Vec<Vec<u8>> = vec![
        vec![],
        LONG_TIE.to_vec(),
        b" ".to_vec(),
        b"gzip,identity".to_vec(),
        b"identity;q=0,identity;q=-1".to_vec(),
    ];
    for name in [
        "gzip",
        "GZIP",
        "identity",
        "*",
        "br",
        "",
        "gzİp",
        "IDENTİTY",
    ] {
        for q in qualities {
            headers.push(format!("{name};q={q}").into_bytes());
            headers.push(format!("{name};q={q};q=bad,identity;q=.25").into_bytes());
            headers.push(format!("{name};q=1;q={q},*;q=0").into_bytes());
        }
    }
    for byte in 0..=255 {
        headers.push([vec![byte], b"gzip".to_vec()].concat());
        headers.push([b"gzip; q=".to_vec(), vec![byte]].concat());
    }
    for space in [
        '\u{85}', '\u{a0}', '\u{1680}', '\u{2000}', '\u{200a}', '\u{2028}', '\u{2029}', '\u{202f}',
        '\u{205f}', '\u{3000}', '\u{200b}',
    ] {
        headers.push(format!("{space}gzip{space};{space}q=0{space},identity").into_bytes());
    }
    let names = ["gzip", "identity", "*", "br", "", "deflate"];
    for length in [2, 5, 12, 13, 17, 49, 50, 51, 64, 127, 256] {
        for _ in 0..100 {
            let values: Vec<_> = (0..length)
                .map(|_| {
                    format!(
                        "{};q={}",
                        names[pick(&mut seed, names.len())],
                        qualities[pick(&mut seed, qualities.len())]
                    )
                })
                .collect();
            headers.push(values.join(",").into_bytes());
        }
    }
    for header in &headers {
        for (operation, result) in [
            ("C", content(header, limits())),
            ("A", selected(header)),
            ("D", download(header, true, limits())),
        ] {
            let expected = match result {
                Ok(Encoding::Identity) => "identity",
                Ok(Encoding::Gzip) => "gzip",
                Err(_) => "ERR",
            };
            oracle.check(operation, header, expected);
        }
    }
    oracle.finish();
    eprintln!(
        "Fixed envd encoding / Go 1.26.8: {} float inputs; {} headers × content/accept/conditional.",
        numbers.len(),
        headers.len()
    );
}

fn quality_cases(qualities: &[&str], seed: &mut u64) -> Vec<Vec<u8>> {
    let mut numbers: Vec<Vec<u8>> = qualities.iter().map(|s| s.as_bytes().to_vec()).collect();
    for value in [
        "0x0.0000000000001p-1022",
        "0x1.fffffffffffffp1023",
        "0x1.fffffffffffff8p1023",
        "0x1.00000000000008p0",
        "0x1.00000000000008001p0",
        "0x1p-1075",
        "-0x0p99999",
        "1.7976931348623157e308",
        "1.7976931348623159e308",
        "2.4703282292062327e-324",
        "2.4703282292062328e-324",
        "1e-100000000",
        "++1",
        "--1",
        "-_1",
        "0x1p_1",
        "0x1_p1",
        "1_e1",
        "1e+_1",
    ] {
        numbers.push(value.as_bytes().to_vec());
    }
    for _ in 0..2000 {
        let sign = if step(seed) & 1 == 0 { "-" } else { "" };
        let mantissa = step(seed);
        let exponent = i32::try_from(step(seed) % 2300).unwrap() - 1150;
        numbers
            .push(format!("{sign}0x{mantissa:016x}.{:016x}p{exponent}", step(seed)).into_bytes());
        numbers.push(format!("{sign}{mantissa}.{}e{}", step(seed), exponent / 3).into_bytes());
        let alphabet = b"0123456789+-eEpPxXabcdefNaInFty._ \t";
        let length = pick(seed, 32);
        numbers.push(
            (0..length)
                .map(|_| alphabet[pick(seed, alphabet.len())])
                .collect(),
        );
    }
    numbers
}
