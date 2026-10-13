use super::{DecodeLimits, Decoder, EncodeLimits, Encoder, Error};
use std::io::{self, BufRead, BufReader, Read, Write};

fn limits(chunk: usize) -> DecodeLimits {
    DecodeLimits {
        chunk_bytes: chunk,
        encoded_bytes: 8 << 20,
        decoded_bytes: 8 << 20,
        header_bytes: 70_000,
        members: 32,
    }
}
fn encode(input: &[u8], chunk: usize) -> Vec<u8> {
    let mut encoder = Encoder::new(
        input,
        EncodeLimits {
            chunk_bytes: chunk,
            decoded_bytes: input.len() as u64,
            encoded_bytes: 8 << 20,
        },
    )
    .unwrap();
    let mut output = Vec::new();
    while let Some(bytes) = encoder.next_chunk().unwrap() {
        assert!(!bytes.is_empty() && bytes.len() <= chunk);
        output.extend(bytes);
    }
    assert!(encoder.next_chunk().unwrap().is_none());
    output
}
fn decode(input: &[u8], budget: DecodeLimits) -> (Vec<u8>, Option<Error>, bool) {
    let mut decoder =
        match Decoder::new(BufReader::with_capacity(budget.chunk_bytes, input), budget) {
            Ok(decoder) => decoder,
            Err(error) => return (Vec::new(), Some(error), false),
        };
    let mut output = Vec::new();
    loop {
        match decoder.next_chunk() {
            Ok(Some(bytes)) => {
                assert!(!bytes.is_empty() && bytes.len() <= budget.chunk_bytes);
                output.extend(bytes);
            }
            Ok(None) => {
                assert!(decoder.next_chunk().unwrap().is_none());
                return (output, None, true);
            }
            Err(error) => {
                assert!(matches!(decoder.next_chunk(), Err(Error::Failed)));
                return (output, Some(error), true);
            }
        }
    }
}
fn with_header(encoded: &[u8], flags: u8, extra: &[u8], name: &[u8], comment: &[u8]) -> Vec<u8> {
    let mut bytes = encoded[..10].to_vec();
    bytes[3] = flags;
    if flags & 4 != 0 {
        bytes.extend(u16::try_from(extra.len()).unwrap().to_le_bytes());
        bytes.extend(extra);
    }
    if flags & 8 != 0 {
        bytes.extend(name);
        bytes.push(0);
    }
    if flags & 16 != 0 {
        bytes.extend(comment);
        bytes.push(0);
    }
    if flags & 2 != 0 {
        let crc = crc32fast::hash(&bytes).to_le_bytes();
        bytes.extend(&crc[..2]);
    }
    bytes.extend(&encoded[10..]);
    bytes
}

#[test]
fn streaming_roundtrips_empty_binary_compressible_and_concatenated_members() {
    let bodies = [
        Vec::new(),
        b"binary\0\xff\n".to_vec(),
        (0..=255).cycle().take(90_000).collect(),
    ];
    for body in bodies {
        for chunk in [1, 3, 17, 1024] {
            let encoded = encode(&body, chunk);
            let (output, error, initialized) = decode(&encoded, limits(chunk));
            assert!(initialized && error.is_none(), "chunk {chunk}: {error:?}");
            assert_eq!(output, body);
        }
    }
    let encoded = [encode(b"first", 17), encode(b"", 3), encode(b"\0last", 1)].concat();
    assert_eq!(decode(&encoded, limits(1)).0, b"first\0last");
}

#[test]
fn headers_are_preflighted_bounded_and_follow_fixed_go_flags_and_strings() {
    let plain = encode(b"data", 17);
    let valid = with_header(&plain, 0xff, &vec![7; 65535], &vec![0xff; 511], b"comment");
    let (data, error, _) = decode(&valid, limits(3));
    assert!(error.is_none(), "{error:?}");
    assert_eq!(data, b"data");
    for input in [
        Vec::new(),
        b"not gzip".to_vec(),
        with_header(&plain, 8, b"", &vec![b'x'; 512], b""),
    ] {
        assert!(!decode(&input, limits(3)).2);
    }
    let header_size = valid.len() - plain.len() + 10;
    for budget in [10, header_size - 1] {
        let result = decode(
            &valid,
            DecodeLimits {
                header_bytes: budget,
                ..limits(17)
            },
        );
        assert!(!result.2 && matches!(result.1, Some(Error::HeaderLimit)));
    }
    assert!(
        decode(
            &valid,
            DecodeLimits {
                header_bytes: header_size,
                ..limits(17)
            }
        )
        .1
        .is_none()
    );
    let mut bad = with_header(&plain, 2, b"", b"", b"");
    bad[10] ^= 1;
    assert!(matches!(
        decode(&bad, limits(17)),
        (_, Some(Error::Header), false)
    ));
}

#[test]
fn every_truncation_and_bad_trailer_is_failure_with_no_false_completion() {
    let encoded = encode(b"prefix with some repeated repeated bytes", 17);
    for end in 0..encoded.len() {
        let (_, error, _) = decode(&encoded[..end], limits(3));
        assert!(error.is_some(), "cut {end}");
    }
    for trailer_byte in 0..8 {
        let mut bad = encoded.clone();
        let offset = bad.len() - 8 + trailer_byte;
        bad[offset] ^= 1;
        let result = decode(&bad, limits(3));
        assert_eq!(result.0, b"prefix with some repeated repeated bytes");
        assert!(matches!(result.1, Some(Error::Checksum)));
    }
    let mut trailing = encoded.clone();
    trailing.push(0);
    let result = decode(&trailing, limits(17));
    assert_eq!(result.0, b"prefix with some repeated repeated bytes");
    assert!(matches!(result.1, Some(Error::Truncated)));
    let second_bad = [encoded, b"invalid next gzip".to_vec()].concat();
    assert!(matches!(
        decode(&second_bad, limits(17)).1,
        Some(Error::Header)
    ));
}

#[test]
fn totals_and_members_are_exact_and_do_not_synthesize_eof() {
    let plain = b"abcdef";
    let compressed = encode(plain, 3);
    let exact = DecodeLimits {
        encoded_bytes: compressed.len() as u64,
        decoded_bytes: plain.len() as u64,
        header_bytes: 10,
        members: 1,
        ..limits(3)
    };
    assert!(decode(&compressed, exact).1.is_none());
    assert!(matches!(
        decode(
            &compressed,
            DecodeLimits {
                encoded_bytes: exact.encoded_bytes - 1,
                ..exact
            }
        )
        .1,
        Some(Error::EncodedLimit)
    ));
    let result = decode(
        &compressed,
        DecodeLimits {
            decoded_bytes: 5,
            ..exact
        },
    );
    assert_eq!(result.0, b"abcde");
    assert!(matches!(result.1, Some(Error::DecodedLimit)));
    let extra = [compressed.clone(), encode(b"", 3)].concat();
    assert!(matches!(
        decode(
            &extra,
            DecodeLimits {
                encoded_bytes: 100,
                ..exact
            }
        )
        .1,
        Some(Error::MemberLimit)
    ));
    assert!(matches!(decode(&extra, exact).1, Some(Error::EncodedLimit)));
    let empty = encode(b"", 3);
    assert!(
        decode(
            &empty,
            DecodeLimits {
                decoded_bytes: 0,
                ..limits(1)
            }
        )
        .1
        .is_none()
    );
    for (input_limit, output_limit, expected) in [(5, 1000, "decoded"), (6, 1, "encoded")] {
        let mut encoder = Encoder::new(
            plain.as_slice(),
            EncodeLimits {
                chunk_bytes: 3,
                decoded_bytes: input_limit,
                encoded_bytes: output_limit,
            },
        )
        .unwrap();
        loop {
            match encoder.next_chunk() {
                Ok(Some(_)) => (),
                Err(error) => {
                    assert!(
                        matches!(
                            (&error, expected),
                            (Error::DecodedLimit, "decoded") | (Error::EncodedLimit, "encoded")
                        ),
                        "{error:?}"
                    );
                    break;
                }
                Ok(None) => panic!("limit must not manufacture EOF"),
            }
        }
        assert!(matches!(encoder.next_chunk(), Err(Error::Failed)));
    }
    let length = compressed.len() as u64;
    let mut encoder = Encoder::new(
        plain.as_slice(),
        EncodeLimits {
            chunk_bytes: 3,
            decoded_bytes: 6,
            encoded_bytes: length,
        },
    )
    .unwrap();
    while encoder.next_chunk().unwrap().is_some() {}
}

struct Broken<'a> {
    remaining: &'a [u8],
}
impl Read for Broken<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let bytes = self.fill_buf()?;
        let count = bytes.len().min(output.len());
        output[..count].copy_from_slice(&bytes[..count]);
        self.consume(count);
        Ok(count)
    }
}
impl BufRead for Broken<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.remaining.is_empty() {
            Err(io::Error::other("transport failure"))
        } else {
            Ok(self.remaining)
        }
    }
    fn consume(&mut self, count: usize) {
        self.remaining = &self.remaining[count..];
    }
}
#[test]
fn source_failure_after_valid_member_is_not_success_and_encoder_never_finishes() {
    let bytes = encode(b"valid prefix", 3);
    let mut decoder = Decoder::new(Broken { remaining: &bytes }, limits(3)).unwrap();
    let mut output = Vec::new();
    loop {
        match decoder.next_chunk() {
            Ok(Some(bytes)) => output.extend(bytes),
            Err(Error::Io(_)) => break,
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(output, b"valid prefix");
    assert!(matches!(decoder.next_chunk(), Err(Error::Failed)));
    let mut encoder = Encoder::new(
        Broken { remaining: b"data" },
        EncodeLimits {
            chunk_bytes: 3,
            decoded_bytes: 100,
            encoded_bytes: 1000,
        },
    )
    .unwrap();
    let mut compressed = Vec::new();
    loop {
        match encoder.next_chunk() {
            Ok(Some(bytes)) => compressed.extend(bytes),
            Err(Error::Io(_)) => break,
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(decode(&compressed, limits(3)).1.is_some());
    assert!(matches!(encoder.next_chunk(), Err(Error::Failed)));
}

#[test]
fn large_stream_is_processed_with_small_buffers() {
    // A bounded adapter passes compressed chunks directly to the decoder.
    struct Encoded<R: BufRead> {
        encoder: Encoder<R>,
        chunk: io::Cursor<Vec<u8>>,
    }
    impl<R: BufRead> BufRead for Encoded<R> {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            if self.chunk.position() == self.chunk.get_ref().len() as u64 {
                self.chunk = io::Cursor::new(
                    self.encoder
                        .next_chunk()
                        .map_err(|e| io::Error::other(format!("{e:?}")))?
                        .unwrap_or_default(),
                );
            }
            self.chunk.fill_buf()
        }
        fn consume(&mut self, count: usize) {
            self.chunk.consume(count);
        }
    }
    impl<R: BufRead> Read for Encoded<R> {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            let count = self.fill_buf()?.read(bytes)?;
            self.consume(count);
            Ok(count)
        }
    }
    let input = io::repeat(b'z').take(4 << 20);
    let mut encoder = Encoder::new(
        BufReader::with_capacity(97, input),
        EncodeLimits {
            chunk_bytes: 97,
            decoded_bytes: 4 << 20,
            encoded_bytes: 8 << 20,
        },
    )
    .unwrap();
    let first = encoder.next_chunk().unwrap().unwrap();
    let source = Encoded {
        encoder,
        chunk: io::Cursor::new(first),
    };
    let mut decoder = Decoder::new(source, limits(113)).unwrap();
    let mut total = 0;
    while let Some(bytes) = decoder.next_chunk().unwrap() {
        assert!(bytes.len() <= 113 && bytes.iter().all(|b| *b == b'z'));
        total += bytes.len();
    }
    assert_eq!(total, 4 << 20);
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(text, "{byte:02x}").unwrap();
    }
    text
}

fn unhex(text: &str) -> Vec<u8> {
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

const BAD_DISTANCE: &str =
    "1f8b08000000000000ff2b284a4dcbac6007af50945a909a58929a82601497a601a50093e717f021000000";

#[test]
fn invalid_backreference_stops_at_the_go_prefix_without_zero_filling() {
    for chunk in [1, 3, 17, 1024] {
        let (bytes, error, ready) = decode(&unhex(BAD_DISTANCE), limits(chunk));
        assert!(ready && matches!(error, Some(Error::Deflate)));
        assert_eq!(bytes, b"prefix\x07");
    }
}

#[test]
#[ignore = "explicit fixed Go 1.26.8 oracle; set SANDPROOF_GUEST_GZIP_REFERENCE"]
fn fixed_go_gzip_reference() {
    use std::process::{Command, Stdio};
    let mut child = Command::new(
        std::env::var_os("SANDPROOF_GUEST_GZIP_REFERENCE").expect("reference required"),
    )
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::inherit())
    .spawn()
    .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let base = encode(b"prefix\0\xff repeated repeated suffix", 17);
    let mut cases = vec![
        base.clone(),
        encode(b"", 3),
        [base.clone(), base.clone()].concat(),
    ];
    for flags in 0..=255 {
        cases.push(with_header(&base, flags, b"extra", b"name\xff", b"comment"));
    }
    for size in [0, 510, 511, 512, 513] {
        cases.push(with_header(&base, 8, b"", &vec![b'n'; size], b""));
    }
    for end in 0..base.len() {
        cases.push(base[..end].to_vec());
    }
    for offset in 0..base.len() {
        let mut bytes = base.clone();
        bytes[offset] ^= 0xff;
        cases.push(bytes);
    }
    cases.push([base.clone(), vec![0; 1]].concat());
    cases.push([base.clone(), vec![0; 10]].concat());
    cases.push(unhex(BAD_DISTANCE));
    let mut seed = 0x311_u64;
    for _ in 0..512 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let mut bytes = base.clone();
        let offset = usize::try_from(seed % bytes.len() as u64).unwrap();
        bytes[offset] ^= seed.to_le_bytes()[4];
        cases.push(bytes);
    }
    for (index, bytes) in cases.iter().enumerate() {
        writeln!(stdin, "D\t{}", hex(bytes)).unwrap();
        stdin.flush().unwrap();
        let mut reference = String::new();
        assert!(stdout.read_line(&mut reference).unwrap() > 0);
        for chunk in [1, 3, 17, 1024] {
            let (output, error, initialized) = decode(bytes, limits(chunk));
            let observed = format!(
                "{}\t{}\t{}",
                if initialized { "READY" } else { "INIT_ERROR" },
                hex(&output),
                if error.is_none() { "END" } else { "ERROR" }
            );
            assert_eq!(
                observed,
                reference.trim_end(),
                "case {index} chunk {chunk} input {}",
                hex(bytes)
            );
        }
    }
    // Go-generated members must decode in Rust; Rust's different DEFLATE bytes
    // must also decode through the real Go reader (checked above).
    let large_body = vec![b'x'; 32769];
    for body in [b"".as_slice(), b"binary\0\xff", &large_body] {
        writeln!(stdin, "E\t{}", hex(body)).unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        assert!(stdout.read_line(&mut line).unwrap() > 0);
        let bytes: Vec<u8> = line
            .trim()
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect();
        for chunk in [1, 3, 17, 1024] {
            let (output, error, _) = decode(&bytes, limits(chunk));
            assert!(error.is_none(), "{error:?}");
            assert_eq!(output, body);
        }
    }
    drop(stdin);
    assert!(child.wait().unwrap().success());
    eprintln!(
        "Fixed Go 1.26.8 gzip: {} decode cases × 4 chunk sizes, plus 3 Go encoder cases × 4.",
        cases.len()
    );
}
