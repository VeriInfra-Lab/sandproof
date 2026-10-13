use super::{Decoder, Error, Event, Limits, Part, QuotedPrintable, boundary};
use std::fmt::Write as _;

fn limits() -> Limits {
    Limits {
        chunk_bytes: 128,
        boundary_bytes: 128,
        header_bytes: 8192,
        header_count: 32,
        line_bytes: 4096,
        parts: 32,
        total_bytes: 1 << 20,
    }
}
fn wire(headers: &[u8], body: &[u8], ending: &[u8]) -> Vec<u8> {
    [
        b"--BOUNDARY\r\n".as_slice(),
        headers,
        b"\r\n\r\n",
        body,
        ending,
    ]
    .concat()
}
const FILE: &[u8] = b"Content-Disposition: form-data; name=\"file\"; filename=\"dir/file\"";
const END: &[u8] = b"\r\n--BOUNDARY--\r\n";

#[derive(Debug, Default)]
struct Observed {
    parts: Vec<(Part, Vec<u8>, bool)>,
    ended: bool,
    error: Option<Error>,
    qp: Option<QuotedPrintable>,
}
impl Observed {
    fn events(&mut self, decoder: &mut Decoder) -> Result<(), Error> {
        loop {
            let event = match decoder.next_event() {
                Ok(Some(event)) => event,
                Ok(None) => return Ok(()),
                Err(error) => {
                    if let Some(qp) = &mut self.qp {
                        let output = qp.abort(error);
                        self.parts.last_mut().unwrap().1.extend(output.bytes);
                        return Err(output.error.unwrap());
                    }
                    return Err(error);
                }
            };
            match event {
                Event::Part(part) => {
                    self.qp = if part.quoted_printable {
                        Some(QuotedPrintable::new(limits().chunk_bytes)?)
                    } else {
                        None
                    };
                    self.parts.push((part, Vec::new(), false));
                }
                Event::Data(bytes) => {
                    let (_, data, _) = self.parts.last_mut().unwrap();
                    if let Some(qp) = &mut self.qp {
                        let output = qp.feed(&bytes);
                        data.extend(output.bytes);
                        if let Some(error) = output.error {
                            return Err(error);
                        }
                    } else {
                        data.extend(bytes);
                    }
                }
                Event::PartEnd => {
                    let (_, data, complete) = self.parts.last_mut().unwrap();
                    if let Some(mut qp) = self.qp.take() {
                        let output = qp.finish();
                        data.extend(output.bytes);
                        if let Some(error) = output.error {
                            return Err(error);
                        }
                    }
                    *complete = true;
                }
                Event::End => self.ended = true,
            }
        }
    }
    fn normalized(&self) -> String {
        let mut output = String::new();
        for (part, data, complete) in &self.parts {
            let filename = part
                .filename
                .as_ref()
                .map_or_else(|| "-".into(), |s| format!("+{}", hex(s.as_bytes())));
            write!(
                output,
                "P:{}:{filename}:{}:{complete};",
                hex(part.name.as_deref().unwrap_or("").as_bytes()),
                hex(data)
            )
            .unwrap();
        }
        output.push_str(if self.ended && self.error.is_none() {
            "END"
        } else {
            "ERROR"
        });
        output
    }
}
fn hex(bytes: &[u8]) -> String {
    let mut value = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(value, "{b:02x}").unwrap();
    }
    value
}
fn observe(chunks: &[&[u8]], budget: Limits) -> Observed {
    let mut decoder = Decoder::new("BOUNDARY", budget).unwrap();
    let capacity = decoder.buffer.capacity();
    let mut observed = Observed::default();
    for chunk in chunks {
        if let Err(error) = decoder
            .feed(chunk)
            .and_then(|()| observed.events(&mut decoder))
        {
            observed.error = Some(error);
            return observed;
        }
        assert_eq!(
            decoder.buffer.capacity(),
            capacity,
            "decoder must never grow its allocation"
        );
    }
    decoder.end_input();
    observed.error = observed.events(&mut decoder).err();
    observed
}
fn parse(bytes: &[u8], chunk: usize) -> Observed {
    observe(&bytes.chunks(chunk).collect::<Vec<_>>(), limits())
}

#[test]
fn binary_content_near_boundaries_and_every_split_are_preserved() {
    let body = b"--BOUNDARY-extra\0\xff\r\n--BOUNDARY-extra\r\n--BOUNDARY-\r\n--BOUNDA";
    let input = wire(FILE, body, END);
    for size in 1..=limits().chunk_bytes {
        let result = parse(&input, size);
        assert_eq!(result.error, None, "chunk {size}");
        assert!(result.ended && result.parts[0].2);
        assert_eq!(result.parts[0].1, body, "chunk {size}");
        assert_eq!(result.parts[0].0.filename.as_deref(), Some("dir/file"));
    }
    // Exercise every two-fragment cut, including exactly on/inside a delimiter.
    for split in 0..=input.len() {
        let mut chunks: Vec<_> = input[..split].chunks(limits().chunk_bytes).collect();
        chunks.extend(input[split..].chunks(limits().chunk_bytes));
        assert_eq!(observe(&chunks, limits()).parts[0].1, body);
    }
}

#[test]
fn preamble_lf_boundaries_empty_parts_and_epilogue() {
    for input in [
        b"ignored preamble\n--BOUNDARY \t\n\n\n--BOUNDARY\n\n--BOUNDARY--\t\nignored".as_slice(),
        b"--BOUNDARY\r\n\r\n\r\n--BOUNDARY\r\n\r\n--BOUNDARY--",
    ] {
        for size in 1..=32 {
            let result = parse(input, size);
            assert_eq!(result.error, None, "{result:?}");
            assert!(result.ended);
            assert_eq!(result.parts.len(), 2);
            assert!(
                result
                    .parts
                    .iter()
                    .all(|(_, data, end)| data.is_empty() && *end)
            );
        }
    }
    for input in [
        b"--BOUNDARY--".as_slice(),
        b"--BOUNDARY--\r\n",
        b"preamble\n--BOUNDARY-- \t",
    ] {
        let result = parse(input, 1);
        assert!(result.ended && result.parts.is_empty());
    }
}

#[test]
fn truncated_input_never_finishes_request_and_keeps_available_prefix() {
    let input = wire(FILE, b"written", END);
    for end in 0..input.len() - 4 {
        let result = parse(&input[..end], 1);
        assert!(!result.ended, "cut {end}");
        assert!(result.error.is_some(), "cut {end}");
    }
    let result = parse(&wire(FILE, b"written", b""), 3);
    assert_eq!(result.parts[0].1, b"written");
    assert!(!result.parts[0].2);
    let malformed = parse(&wire(FILE, b"written", b"\r\n--BOUNDARY wrong\r\n"), 1);
    assert_eq!(malformed.parts[0].1, b"written");
    assert!(
        malformed.parts[0].2,
        "Go completes the prior part before rejecting the next delimiter line"
    );
    assert_eq!(malformed.error, Some(Error::Malformed));
}

#[test]
fn folded_headers_first_value_and_full_encoded_filename() {
    for (header, filename) in [
        (
            "Content-Disposition: form-data;\r\n\tname=file; filename=\"a/b\"",
            "a/b",
        ),
        (
            "Content-Disposition: form-data; name=file; filename=\"C:\\dir\\file\"",
            "C:\\dir\\file",
        ),
        (
            "Content-Disposition: form-data; name=file; filename=old; filename*=utf-8''dir%2F%E9%9B%AA",
            "dir/雪",
        ),
        (
            "Content-Disposition: form-data; name=file; filename*0*=utf-8''dir%2F; filename*1*=snow; filename*2=.txt",
            "dir/snow.txt",
        ),
        (
            "Content-Disposition: form-data; name=file; filename=old; filename*=bad''new",
            "old",
        ),
        (
            "Content-Disposition: form-data; name=file; filename*0*=utf-8''%E9; filename*1*=%9B%AA",
            "雪",
        ),
        (
            "Content-Disposition: form-data; name=file; filename=old; filename=old;",
            "old",
        ),
        (
            "Content-Disposition: form-data; name=file; filename=\"\"",
            "",
        ),
        (
            "Content-Disposition: form-data; name=file; filename=first\r\nContent-Disposition: attachment; filename=second",
            "first",
        ),
    ] {
        let result = parse(&wire(header.as_bytes(), b"data", END), 1);
        assert_eq!(result.error, None, "{header}");
        assert_eq!(result.parts[0].0.name.as_deref(), Some("file"));
        assert_eq!(result.parts[0].0.filename.as_deref(), Some(filename));
    }
    for header in [
        "Content-Disposition: form-data; name=file",
        "Content-Disposition: bad; name=file",
        "Content-Disposition: form-data; name=file; filename=one; filename=two",
    ] {
        let result = parse(&wire(header.as_bytes(), b"data", END), 1);
        assert_eq!(result.error, None);
        assert_eq!(result.parts[0].0.filename, None);
    }
    for header in [
        b"Bad\tName: value".as_slice(),
        b" folded first",
        b"X: \x7f",
        b"Content-Disposition: form-data; name=file; filename*=utf-8''%FF",
    ] {
        assert_eq!(
            parse(&wire(header, b"data", END), 1).error,
            Some(Error::Malformed)
        );
    }
}

#[test]
fn content_type_uses_quoted_boundary_and_rejects_ambiguous_parameters() {
    assert_eq!(
        boundary("Multipart/Form-Data; boundary=\"a b\"", 64),
        Ok("a b".into())
    );
    assert_eq!(
        boundary("multipart/form-data; boundary=x; boundary=x;", 64),
        Ok("x".into())
    );
    for content in [
        "text/plain; boundary=x",
        "multipart/form-data",
        "multipart/form-data; boundary=\"\"",
        "multipart/form-data; boundary=x; boundary=y",
    ] {
        assert_eq!(boundary(content, 128), Err(Error::Malformed));
    }
    assert_eq!(
        boundary("multipart/form-data; boundary=x", 1),
        Err(Error::ResourceExhausted)
    );
}

#[test]
fn quoted_printable_preserves_prefix_and_go_line_rules() {
    let headers = [FILE, b"\r\nContent-Transfer-Encoding: Quoted-Printable"].concat();
    for (body, output, success) in [
        (
            b"a=00=ff=3d\r\nb= \t\r\nc \t\r\n".as_slice(),
            b"a\0\xff=\r\nbc\r\n".as_slice(),
            true,
        ),
        (b"a=", b"a", true),
        (b"=", b"", false),
        (b"a=QZ", b"a=QZ", true),
        (b"a=Q", b"a=Q", true),
        (b"a\x7fmore", b"a", false),
        (b"a=\r", b"a", false),
    ] {
        for size in 1..=32 {
            let result = parse(&wire(&headers, body, END), size);
            assert_eq!(result.parts[0].1, output, "body {body:?}, chunk {size}");
            assert_eq!(result.ended, success);
            assert_eq!(result.error.is_none(), success);
        }
    }
    let exact = parse(&wire(&headers, &vec![b'x'; 4096], END), 1);
    assert!(exact.ended && exact.error.is_none());
    assert_eq!(exact.parts[0].1, vec![b'x'; 4096]);
    let result = parse(&wire(&headers, &vec![b'x'; 4097], END), 128);
    assert_eq!(result.parts[0].1, vec![b'x'; 4096]);
    assert_eq!(result.error, Some(Error::Malformed));
    let mut qp = QuotedPrintable::new(8).unwrap();
    assert_eq!(qp.feed(b"too large").error, Some(Error::ResourceExhausted));
    assert_eq!(qp.finish().error, Some(Error::Poisoned));
    let partial = parse(&wire(&headers, b"prefix=00=ff", b""), 1);
    assert_eq!(partial.parts[0].1, b"prefix\0\xff");
    assert!(!partial.parts[0].2 && !partial.ended);
    assert_eq!(partial.error, Some(Error::Truncated));
}

#[test]
fn quotas_include_ignored_headers_parts_framing_and_epilogue() {
    let input = wire(b"X: a\r\nX: b", b"data", END);
    let budgets = [
        Limits {
            header_bytes: 4,
            ..limits()
        },
        Limits {
            header_count: 1,
            ..limits()
        },
        Limits {
            line_bytes: 4,
            ..limits()
        },
        Limits {
            total_bytes: 8,
            ..limits()
        },
    ];
    for budget in budgets {
        assert_eq!(
            observe(&input.chunks(1).collect::<Vec<_>>(), budget).error,
            Some(Error::ResourceExhausted)
        );
    }
    let input = b"--BOUNDARY\r\n\r\n\r\n--BOUNDARY\r\n\r\n\r\n--BOUNDARY--";
    assert_eq!(
        observe(
            &input.chunks(1).collect::<Vec<_>>(),
            Limits {
                parts: 1,
                ..limits()
            }
        )
        .error,
        Some(Error::ResourceExhausted)
    );
    let input = b"--BOUNDARY--\r\nepilogue";
    let result = observe(
        &input.chunks(1).collect::<Vec<_>>(),
        Limits {
            total_bytes: 16,
            ..limits()
        },
    );
    assert!(result.ended && result.error == Some(Error::ResourceExhausted));
    let mut decoder = Decoder::new("BOUNDARY", limits()).unwrap();
    assert_eq!(decoder.feed(&[0; 129]), Err(Error::ResourceExhausted));
    assert_eq!(decoder.next_event(), Err(Error::Poisoned));
    let mut decoder = Decoder::new("BOUNDARY", limits()).unwrap();
    decoder.feed(b"--").unwrap();
    assert_eq!(decoder.feed(b"more"), Err(Error::InputPending));
    assert_eq!(decoder.next_event(), Err(Error::Poisoned));
}

#[test]
fn a_large_stream_keeps_decoder_allocation_fixed() {
    let mut decoder = Decoder::new(
        "BOUNDARY",
        Limits {
            total_bytes: 8 << 20,
            ..limits()
        },
    )
    .unwrap();
    let capacity = decoder.buffer.capacity();
    decoder.feed(b"--BOUNDARY\r\n\r\n").unwrap();
    assert!(matches!(decoder.next_event(), Ok(Some(Event::Part(_)))));
    assert_eq!(decoder.next_event(), Ok(None));
    let chunk = vec![b'x'; limits().chunk_bytes];
    let mut emitted = 0;
    for _ in 0..32768 {
        decoder.feed(&chunk).unwrap();
        while let Some(event) = decoder.next_event().unwrap() {
            if let Event::Data(bytes) = event {
                emitted += bytes.len();
            }
        }
        assert_eq!(decoder.buffer.capacity(), capacity);
    }
    assert_eq!(emitted, 4 << 20);
    decoder.feed(END).unwrap();
    assert_eq!(decoder.next_event(), Ok(Some(Event::PartEnd)));
    assert_eq!(decoder.next_event(), Ok(Some(Event::End)));
}

#[test]
#[ignore = "explicit fixed Go 1.26.8 oracle; set SANDPROOF_GUEST_MULTIPART_REFERENCE to its binary"]
fn fixed_go_multipart_reference() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};
    let path =
        std::env::var_os("SANDPROOF_GUEST_MULTIPART_REFERENCE").expect("reference binary required");
    let mut child = Command::new(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut cases = vec![
        wire(FILE, b"data", END),
        wire(FILE, b"\0\xff\r\n--BOUNDARY-extra", END),
        b"--BOUNDARY--".to_vec(),
        b"pre\n--BOUNDARY\n\nbody\n--BOUNDARY--\n".to_vec(),
    ];
    for disposition in [
        "form-data; name=file",
        "form-data; name=file; filename=\"C:\\dir\\file\"",
        "form-data; name=file; filename=old; filename*=utf-8''dir%2F%E9%9B%AA",
        "form-data; name=file; filename*0*=utf-8''a; filename*1*=b%2Fc",
        "form-data; name=file; filename*0*=utf-8''%E9; filename*1*=%9B%AA",
        "form-data; name=file; filename=old; filename*=bad''new; filename*0=ignored",
        "form-data; name=file; filename*0*=bad''new; filename*1*=b%2Fc; filename*3=gap",
        "form-data; name=file; filename*0=plain; filename*0*=utf-8''ignored; filename*1*=%zz",
        "form-data; name=file; filename=a; filename=b",
        "bad",
        "form-data;\r\n\tname=file; filename=\"\"",
        "form-data; name=file; filename=x\r\nContent-Disposition: attachment; filename=y",
    ] {
        cases.push(wire(
            format!("Content-Disposition: {disposition}").as_bytes(),
            b"data",
            END,
        ));
    }
    let headers = [FILE, b"\r\nContent-Transfer-Encoding: quoted-printable"].concat();
    for body in [
        b"=Q\r\na=\t\n=00=ff \t\r\n".to_vec(),
        b"=".to_vec(),
        b"a=".to_vec(),
        b"a\x7fmore".to_vec(),
        b"a=\r".to_vec(),
        vec![b'x'; 4095],
        vec![b'x'; 4096],
        vec![b'x'; 4097],
    ] {
        cases.push(wire(&headers, &body, END));
    }
    let base = wire(FILE, b"prefix\r\n--BOUNDARY-no\r\nsuffix", END);
    for end in 0..base.len() {
        cases.push(base[..end].to_vec());
    }
    let encoded = wire(&headers, b"prefix=00=ff\r\nmore=Q =\r\nlast=", END);
    for end in 0..encoded.len() {
        cases.push(encoded[..end].to_vec());
    }
    // Deterministic mutation corpus; keep the seed and input index in failures.
    let mut seed = 0x311_u64;
    for _ in 0..512 {
        let mut input = base.clone();
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let pos = usize::try_from(seed % input.len() as u64).unwrap();
        let alphabet = b"-\r\n\t X;:=\"";
        input[pos] = alphabet[usize::try_from((seed >> 32) % alphabet.len() as u64).unwrap()];
        cases.push(input);
    }
    let mut stricter_header_ends = 0;
    for (index, input) in cases.iter().enumerate() {
        writeln!(stdin, "{}\t{}", hex(b"BOUNDARY"), hex(input)).unwrap();
        stdin.flush().unwrap();
        let mut reference = String::new();
        assert!(stdout.read_line(&mut reference).unwrap() > 0);
        for chunk in [1, 3, 17, 128] {
            let observed = parse(input, chunk);
            let reference = reference.trim_end();
            // Deliberate stricter completion contract: EOF in MIME headers
            // never stands in for a closing delimiter. Compare all prior
            // part effects as usual; permit only this exact terminal delta.
            if observed.error == Some(Error::TruncatedHeaders) && reference.ends_with("END") {
                stricter_header_ends += 1;
                assert_eq!(
                    observed.normalized().strip_suffix("ERROR"),
                    reference.strip_suffix("END"),
                    "header EOF case {index}"
                );
                continue;
            }
            assert_eq!(
                observed.normalized(),
                reference,
                "seed 0x311 case {index} chunk {chunk} input {}",
                hex(input)
            );
        }
    }
    drop(stdin);
    assert!(child.wait().unwrap().success());
    eprintln!(
        "Fixed Go 1.26.8 MIME reader: {} inputs × 4 chunkings; {stricter_header_ends} explicit stricter header-EOF comparisons.",
        cases.len()
    );
}
