use super::*;

fn cases() -> Vec<Vec<u8>> {
    let mut headers = Vec::new();
    for nl in [b"\r\n".as_slice(), b"\n"] {
        for byte in (0..=255u8).filter(|b| *b != b'\n' && *b != b'\r') {
            for line in [
                [b"x-".as_slice(), &[byte], b"-key: value", nl].concat(),
                [b"x-key: value".as_slice(), &[byte], b"tail", nl].concat(),
                [
                    b"x-key: first".as_slice(),
                    nl,
                    b" \tvalue",
                    &[byte],
                    b"tail \t",
                    nl,
                ]
                .concat(),
                [
                    b"x bad: first".as_slice(),
                    nl,
                    b" \tvalue",
                    &[byte],
                    b"tail \t",
                    nl,
                ]
                .concat(),
                [
                    b"NoColon".as_slice(),
                    &[byte],
                    b" \t",
                    nl,
                    b" continuation: ignored",
                    nl,
                ]
                .concat(),
                [b" \tinitial".as_slice(), &[byte], b": value", nl].concat(),
            ] {
                headers.push(line);
            }
        }
        for count in [0, 1, 78, 79, 80, 81, 128] {
            headers.push([b" ".as_slice(), vec![b'x'; count].as_slice(), nl].concat());
        }
        for line in [
            b": value".as_slice(),
            b"x-key: bad\rvalue",
            b"x-key: \0",
            b"x-\tkey: value",
            b"x-key: good",
            b"NoColon \t",
            b"content-disposition: not valid",
        ] {
            headers.push([line, nl, b"   folded \t", nl, b"\t\t", nl].concat());
        }
        headers.push(
            [
                b"x-key: \0".as_slice(),
                b"\\\"<&>".repeat(400).as_slice(),
                nl,
            ]
            .concat(),
        );
    }
    let mut cases = Vec::new();
    let prior = [header("file", "first", "\r\n"), b"done\r\n".to_vec()].concat();
    for headers in headers {
        for prefix in [b"".as_slice(), prior.as_slice()] {
            cases.push([prefix, b"--B\r\n", &headers,
                b"Content-Disposition: form-data; name=\"file\"; filename=\"/target\"\r\n\r\npayload\r\n--B--\r\n"].concat());
        }
    }
    // Complete physical errors without a header terminator, including EOF in
    // the last physical/folded line. Valid header EOF remains a separate limit.
    for headers in [
        b" Bad: value".as_slice(),
        b"NoColon \t",
        b"x-key: bad\0",
        b"x-key: bad\0\r\n \t",
        b"x-key: value\r\n fold\x7f",
        b"x-key: value\r\nBad\t",
        b"x-key: value\r\n x\r",
    ] {
        cases.push([b"--B\r\n".as_slice(), headers].concat());
    }
    cases
}

#[test]
fn header_wire_budget_counts_original_go_quote_and_json() {
    for error in [
        HeaderError::InitialLine(b" \"\xff\r".to_vec()),
        HeaderError::InitialLineTooLong,
        HeaderError::MissingColon(b"no: not this test\xff\t".to_vec()),
        HeaderError::InvalidLine("X: 雪\u{2028}\u{10ffff}".as_bytes().to_vec()),
    ] {
        let full = headers_payload(&error, 65536).unwrap();
        assert_eq!(headers_payload(&error, full.len()).unwrap(), full);
        assert_eq!(
            headers_payload(&error, full.len() - 1),
            Err(Error::ResponseLimit)
        );
    }
}

#[tokio::test]
#[ignore = "requires explicit fixed Go upload reference and local loopback HTTP"]
async fn mime_headers_and_prior_file_completion_match_original_go() {
    let cases = cases();
    let mut reference = Reference::from_env("SANDPROOF_GUEST_UPLOAD_REFERENCE");
    for input in &cases {
        for chunk in [1, 7, 128] {
            check(&mut reference, input, chunk).await;
        }
    }
    reference.finish();
    eprintln!(
        "Fixed Go MIME headers: {} inputs x 3 chunks, prior file completion and exact terminal HTTP.",
        cases.len()
    );
}

#[tokio::test]
#[ignore = "requires explicit fixed Go upload reference and local loopback HTTP"]
async fn source_read_failure_finishes_mime_header_validation_before_error_wrapping() {
    let prior = [header("file", "first", "\r\n"), b"done\r\n".to_vec()].concat();
    let mut reference = Reference::from_env("SANDPROOF_GUEST_UPLOAD_REFERENCE");
    let mut count = 0;
    for line in [
        b"NoColon".to_vec(),
        b" Bad: x".to_vec(),
        [b" ".as_slice(), &[b'x'; 80]].concat(),
        b"x<: bad".to_vec(),
        b"x<: bad\r\n".to_vec(),
        b"x-key: bad\0".to_vec(),
        b"x-key: good\r\n bad\0".to_vec(),
        b"x-key: bad\0\r\n \t".to_vec(),
    ] {
        for prefix in [b"".as_slice(), prior.as_slice()] {
            let input = [prefix, b"--B\r\n", line.as_slice()].concat();
            for chunk in [1, 7, 128] {
                check_source(&mut reference, &input, chunk, true).await;
                count += 1;
            }
        }
    }
    reference.finish();
    eprintln!(
        "Fixed Go source failure MIME headers: {count} observations, actual reader failure and exact file/HTTP output."
    );
}
