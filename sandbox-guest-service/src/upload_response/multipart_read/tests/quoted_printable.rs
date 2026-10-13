use super::*;
use sandproof_guest_runtime::multipart::QuotedPrintableError as Qp;

fn field(data: &[u8], closed: bool, prior: bool) -> Vec<u8> {
    [
        if prior { [header("file", "first", "\r\n"), b"done\r\n".to_vec()].concat() } else { vec![] },
        b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"/second\"\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n".to_vec(),
        data.to_vec(),
        if closed { b"\r\n--B--\r\n".to_vec() } else { vec![] },
    ].concat()
}
fn bodies() -> Vec<Vec<u8>> {
    let mut cases: Vec<Vec<u8>> = vec![
        b"".as_slice(),
        b"=",
        b"a=",
        b"==",
        b"a==",
        b"a=Q",
        b"a=QZ",
        b"a=0g",
        b"a=\rX",
        b"a=\r",
        b"a=\n",
        b"a=\r\n",
        b"a=\r\r\n",
        b"a= \t\r\r\n",
        b"a\0=\r\r\n",
        b"prefix=00=ff",
        b"prefix\r\nnext",
        b"a= \t",
    ]
    .into_iter()
    .map(<[u8]>::to_vec)
    .collect();
    for byte in 0..=255u8 {
        for middle in [vec![byte], vec![b'=', byte, b'X'], vec![b'=', b'0', byte]] {
            cases.push([b"prefix=00".as_slice(), &middle, b"suffix"].concat());
        }
    }
    for size in [4094, 4095, 4096, 4097, 8192] {
        for tail in [b"".as_slice(), b"\n", b"\r\n", b"=", b"=\n", b"\0"] {
            cases.push([vec![b'x'; size], tail.to_vec()].concat());
        }
    }
    cases.push([b"prefix=".as_slice(), &vec![b'\r'; 3000]].concat());
    cases
}
#[tokio::test]
#[ignore = "requires explicit fixed Go upload reference and local loopback HTTP"]
async fn qp_diagnostics_prefix_completion_and_http_match_original_go() {
    let bodies = bodies();
    let mut reference = Reference::from_env("SANDPROOF_GUEST_UPLOAD_REFERENCE");
    for data in &bodies {
        for closed in [false, true] {
            for prior in [false, true] {
                let input = field(data, closed, prior);
                for chunk in [1, 7, 128] {
                    check(&mut reference, &input, chunk).await;
                }
            }
        }
    }
    reference.finish();
    eprintln!(
        "Fixed Go QP: {} bodies x 2 ends x 2 prior-file states x 3 chunks, exact file effects and terminal HTTP.",
        bodies.len()
    );
}
#[tokio::test]
#[ignore = "requires explicit fixed Go upload reference and local loopback HTTP"]
async fn qp_diagnostic_precedes_actual_source_failure() {
    let mut reference = Reference::from_env("SANDPROOF_GUEST_UPLOAD_REFERENCE");
    let mut count = 0;
    // No ambiguous delimiter tail: this tests QP/source ordering, not the
    // separate MIME partial-boundary behavior on arbitrary transport failure.
    for data in [
        b"prefix=".as_slice(),
        b"prefix\0tail",
        b"prefix=\rX",
        b"prefix==",
        b"prefix\0=",
        b"prefix=\r\rX",
    ] {
        for prior in [false, true] {
            for chunk in [1, 7, 128] {
                check_source(&mut reference, &field(data, false, prior), chunk, true).await;
                count += 1;
            }
        }
    }
    reference.finish();
    eprintln!("Fixed Go QP/source failure: {count} exact observations.");
}
#[test]
fn qp_response_budget_counts_quoted_suffix_and_hex_diagnostics() {
    for error in [
        Qp::UnexpectedEof,
        Qp::LineTooLong,
        Qp::InvalidHex(13),
        Qp::InvalidUnescaped(0),
        Qp::InvalidSoftBreak(vec![b'\r'; 3000]),
    ] {
        let bytes = qp_payload(&error, 65536).unwrap();
        assert_eq!(qp_payload(&error, bytes.len()).unwrap(), bytes);
        assert_eq!(
            qp_payload(&error, bytes.len() - 1),
            Err(Error::ResponseLimit)
        );
    }
}

#[tokio::test]
#[ignore = "requires explicit fixed Go upload reference and local loopback HTTP"]
async fn full_line_go_read_buffer_alignment_difference_is_explicit() {
    // Existing event decoding receives PartEnd separately from Data; it cannot
    // reproduce Go's simultaneous (n, EOF) observation at every source-buffer
    // offset. Preserve this known difference rather than claiming full parity.
    let mut reference = Reference::from_env("SANDPROOF_GUEST_UPLOAD_REFERENCE");
    let head = field(b"", false, false).len();
    for offset in [4095, 4096] {
        let data = vec![b'x'; 4096];
        let input = [
            vec![b'p'; offset - head - 2],
            b"\r\n".to_vec(),
            field(&data, true, false),
        ]
        .concat();
        for chunk in [1, 7, 128] {
            let (effects, error) = observe(&input, chunk, false);
            assert!(error.is_none());
            assert_eq!(effects, format!("W:{}:true;", hex(&data)));
        }
        let payload = qp_payload(&Qp::LineTooLong, 65536).unwrap();
        reference.check(
            &format!("N\t{}", hex(&input)),
            &format!(
                "W:{}:false;\t500\t{}\t{}\t{}\t{}",
                hex(&data),
                hex(b"application/json; charset=utf-8"),
                hex(payload.len().to_string().as_bytes()),
                hex(b"nosniff"),
                hex(&payload)
            ),
        );
    }
    reference.finish();
}
