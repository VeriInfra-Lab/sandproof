use super::*;
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

pub(crate) struct Reference {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
}
impl Reference {
    pub(crate) fn new() -> Self {
        Self::from_env("SANDPROOF_GUEST_DOWNLOAD_REFERENCE")
    }
    pub(crate) fn from_env(name: &str) -> Self {
        let mut child = Command::new(std::env::var_os(name).expect("reference required"))
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
    pub(crate) fn check(&mut self, input: &str, observed: &str) {
        assert_eq!(observed, self.observe(input), "oracle input {input}");
    }
    pub(crate) fn observe(&mut self, input: &str) -> String {
        let writer = self.input.as_mut().unwrap();
        writeln!(writer, "{input}").unwrap();
        writer.flush().unwrap();
        let mut line = String::new();
        assert!(self.output.read_line(&mut line).unwrap() > 0);
        line.strip_suffix('\n').unwrap().to_owned()
    }
    pub(crate) fn finish(&mut self) {
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
pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut value, byte| {
        write!(value, "{byte:02x}").unwrap();
        value
    })
}
struct Zone<'a> {
    name: &'a [u8],
    offset: i32,
}
impl wire::date::LocalZone for Zone<'_> {
    fn offset(&self, name: &[u8], _: i64) -> Option<i32> {
        (!self.name.is_empty() && self.name == name).then_some(self.offset)
    }
}
fn step(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    *seed
}
fn pick(seed: &mut u64, length: usize) -> usize {
    usize::try_from(step(seed) % length as u64).unwrap()
}

fn dates() -> Vec<Vec<u8>> {
    let base = [
        "Sun, 06 Nov 1994 08:49:37 GMT",
        "Sunday, 06-Nov-94 08:49:37 GMT",
        "Sun Nov  6 08:49:37 1994",
    ];
    let mut dates = vec![vec![], b"bad".to_vec()];
    for value in base {
        dates.push(value.as_bytes().to_vec());
        for pos in 0..value.len() {
            for byte in [0, 9, 10, 11, 13, b' ', b'0', b'1', b'9', b'a', b'Z', 255] {
                let mut bytes = value.as_bytes().to_vec();
                bytes[pos] = byte;
                dates.push(bytes);
            }
        }
        for fraction in [".1", ",000000001", ".123456789987654321", ".0", ",", ",x"] {
            dates.push(value.replace("37", &format!("37{fraction}")).into_bytes());
        }
        for year in [
            "00", "68", "69", "99", "-1", "+1", "-0", "0000", "0001", "1969", "1970", "1999",
            "9999",
        ] {
            dates.push(
                value
                    .replace(if value.contains("-94") { "94" } else { "1994" }, year)
                    .into_bytes(),
            );
        }
        dates.push(format!(" {value}").into_bytes());
        dates.push(format!("{value} ").into_bytes());
        dates.push(value.replace(' ', "   ").into_bytes());
    }
    for zone in [
        "UTC", "utc", "GMT", "GMT+0", "GMT+03", "GMT-23", "GMT+24", "+03", "+0", "+000", "-23",
        "XYZ", "ChST", "MeST", "WITA", "ABCDE", "ABCDT", "ABCDEF", "ABC", "XYZjunk",
    ] {
        dates.push(format!("Sunday, 06-Nov-94 08:49:37 {zone}").into_bytes());
    }
    for (date, time) in [
        ("29 Feb 2000", "00:00:00"),
        ("29 Feb 1900", "00:00:00"),
        ("01 Jan 0000", "00:00:00"),
        ("01 Jan 0001", "00:00:00"),
        ("01 Jan 1970", "00:00:00"),
        ("31 Dec 9999", "23:59:59"),
    ] {
        dates.push(format!("Thu, {date} {time} GMT").into_bytes());
    }
    dates
}

#[test]
#[ignore = "explicit Go 1.26.8 ParseTime/ServeContent oracle; set SANDPROOF_GUEST_DOWNLOAD_REFERENCE"]
fn fixed_go_download_reference() {
    let mut oracle = Reference::new();
    let dates = dates();
    for zone in [
        Zone {
            name: b"",
            offset: 0,
        },
        Zone {
            name: b"XYZ",
            offset: 3661,
        },
        Zone {
            name: b"GMT+03",
            offset: -3600,
        },
    ] {
        for date in &dates {
            let observed = wire::date::parse(date, &zone).map_or_else(
                || "ERR".into(),
                |t| format!("{}\t{}", t.seconds(), t.nanos()),
            );
            oracle.check(
                &format!("T\t{}\t{}\t{}", hex(date), hex(zone.name), zone.offset),
                &observed,
            );
        }
    }
    let mut cases = range_cases();
    cases.extend(condition_cases());
    for case in &cases {
        case.check(&mut oracle);
    }
    oracle.finish();
    eprintln!(
        "Fixed Go 1.26.8: {} HTTP dates × 3 local-zone contexts; {} ServeContent status/header/body cases.",
        dates.len(),
        cases.len()
    );
}

#[derive(Clone)]
struct Case {
    method: &'static str,
    size: u64,
    modified: Timestamp,
    etag: Vec<u8>,
    headers: [Vec<u8>; 6],
}
impl Case {
    fn new() -> Self {
        Self {
            method: "GET",
            size: 5,
            modified: Timestamp::new(784_111_777, 123).unwrap(),
            etag: vec![],
            headers: std::array::from_fn(|_| vec![]),
        }
    }
    fn check(&self, oracle: &mut Reference) {
        let names = [
            "if-match",
            "if-unmodified-since",
            "if-none-match",
            "if-modified-since",
            "if-range",
            "range",
        ];
        let mut request = HeaderMap::new();
        let mut response = initial();
        for (name, value) in names.into_iter().zip(&self.headers) {
            request.insert(name, HeaderValue::from_bytes(value).unwrap());
        }
        if !self.etag.is_empty() {
            response.insert("etag", HeaderValue::from_bytes(&self.etag).unwrap());
        }
        let result = match prepare(
            &Method::from_bytes(self.method.as_bytes()).unwrap(),
            &request,
            response,
            self.modified,
            limits(),
            &Utc,
        )
        .unwrap()
        {
            Preparation::Finished(plan) => plan,
            Preparation::Continue(next) => next
                .finish(
                    self.size,
                    HeaderValue::from_static("application/octet-stream"),
                    Some(BOUNDARY),
                )
                .unwrap(),
        };
        let data: Vec<_> = (0..self.size)
            .map(|n| u8::try_from(n % 251).unwrap())
            .collect();
        let body = render(&result.body, &data);
        let mut observed = vec![result.status.as_u16().to_string()];
        for name in [
            "last-modified",
            "content-range",
            "accept-ranges",
            "content-type",
            "content-length",
            "etag",
            "cache-control",
            "content-encoding",
            "x-content-type-options",
            "vary",
            "content-disposition",
        ] {
            observed.push(hex(first(&result.headers, name)));
        }
        observed.push(hex(&body));
        let mut input = vec![
            "P".into(),
            self.size.to_string(),
            self.modified.seconds().to_string(),
            self.modified.nanos().to_string(),
            self.method.into(),
            hex(&self.etag),
        ];
        input.extend(self.headers.iter().map(|value| hex(value)));
        input.extend([String::new(), "0".into()]);
        oracle.check(&input.join("\t"), &observed.join("\t"));
    }
}
fn range_cases() -> Vec<Case> {
    let values = [
        "",
        "bytes=",
        "bytes=, ,",
        "bytes=0-0",
        "bytes=1-3",
        "bytes=1-",
        "bytes=-0",
        "bytes=-1",
        "bytes=-999",
        "bytes=0--0",
        "bytes=+0-+1",
        "bytes=4-99",
        "bytes=5-bad",
        "bytes=9-",
        "bytes=0-bad",
        "bytes=0-1,3-4",
        "bytes=0-,0-",
        "bytes=-0,-0",
        "bytes=4-3",
        "bytes=bad",
        " bytes=0-1",
        "Bytes=0-1",
        "bytes=0_0-1",
        "bytes=0-1,,",
        "bytes=9223372036854775807-",
        "bytes=9223372036854775808-",
        "bytes=-9223372036854775808",
    ];
    let mut cases = Vec::new();
    for size in [0, 1, 5, 256] {
        for method in ["GET", "HEAD", "POST"] {
            for value in values {
                let mut case = Case::new();
                case.size = size;
                case.method = method;
                case.headers[5] = value.as_bytes().to_vec();
                cases.push(case);
            }
        }
    }
    let alphabet = b"01239+-=, bytesBAD\t";
    let mut seed = 0x311;
    for _ in 0..512 {
        let mut case = Case::new();
        let length = pick(&mut seed, 32);
        case.headers[5] = b"bytes="
            .iter()
            .copied()
            .chain((0..length).map(|_| alphabet[pick(&mut seed, alphabet.len())]))
            .collect();
        cases.push(case);
    }
    cases
}
fn condition_cases() -> Vec<Case> {
    let tags = [
        "",
        "*",
        "*junk",
        ",*",
        "bad",
        "\"tag\"",
        "W/\"tag\"",
        "\"other\", \"tag\"",
        "\"other\"\"tag\"",
        "\"tag\"junk",
        "w/\"tag\"",
        " \"tag\" ",
        "\"tag",
        ",,,",
        "W/",
        "\"\"",
        "\"ta\\g\"",
    ];
    let dates = [
        "",
        "bad",
        "Sun, 06 Nov 1994 08:49:36 GMT",
        "Sun, 06 Nov 1994 08:49:37 GMT",
        "Sun, 06 Nov 1994 08:49:37.1 GMT",
        "Sunday, 06-Nov-94 08:49:38 GMT",
        "Thu, 01 Jan 1970 00:00:00 GMT",
        "Mon, 01 Jan 0001 00:00:00 GMT",
    ];
    let mut cases = Vec::new();
    for method in ["GET", "HEAD", "POST"] {
        for etag in [b"".as_slice(), b"\"tag\"", b"W/\"tag\""] {
            for field in [0, 2, 4] {
                for tag in tags {
                    let mut case = Case::new();
                    case.method = method;
                    case.etag = etag.to_vec();
                    case.headers[field] = tag.as_bytes().to_vec();
                    case.headers[5] = b"bytes=0-1".to_vec();
                    cases.push(case);
                }
            }
            for field in [1, 3, 4] {
                for date in dates {
                    let mut case = Case::new();
                    case.method = method;
                    case.etag = etag.to_vec();
                    case.headers[field] = date.as_bytes().to_vec();
                    case.headers[5] = b"bytes=bad".to_vec();
                    cases.push(case);
                }
            }
        }
    }
    for seconds in [-62_135_596_800, 0, 1, 784_111_777] {
        for nanos in [0, 1, 999_999_999] {
            for field in [1, 3, 4] {
                for date in dates {
                    let mut case = Case::new();
                    case.modified = Timestamp::new(seconds, nanos).unwrap();
                    case.headers[field] = date.as_bytes().to_vec();
                    case.headers[5] = b"bytes=0-1".to_vec();
                    cases.push(case);
                }
            }
        }
    }
    let mut seed = 0x311;
    for _ in 0..512 {
        let mut case = Case::new();
        case.etag = b"\"tag\"".to_vec();
        for field in 0..6 {
            case.headers[field] = if field % 2 == 0 {
                tags[pick(&mut seed, tags.len())]
            } else {
                dates[pick(&mut seed, dates.len())]
            }
            .as_bytes()
            .to_vec();
        }
        case.headers[5] = b"bytes=0-1".to_vec();
        cases.push(case);
    }
    cases
}
