use super::*;
use crate::{
    upload_body::GzipBody,
    upload_response::{
        GzipReadError,
        gzip_read::tests::{encode, limits as gzip_limits, wire},
    },
};
use axum::body::{Body, Bytes};
use futures_util::stream;
use sandproof_guest_runtime::multipart::SourceStage;

fn damaged(input: &[u8]) -> Vec<Vec<u8>> {
    let encoded = encode(input);
    let mut checksum = encoded.clone();
    let trailer = checksum.len() - 8;
    checksum[trailer] ^= 1;
    vec![
        checksum,
        encoded[..encoded.len() - 3].to_vec(),
        [encoded, b"invalid next member".to_vec()].concat(),
    ]
}
fn forms() -> Vec<Vec<u8>> {
    let mut forms = vec![b"preamble".to_vec(), b"--B--".to_vec()];
    for line in [
        b"X: valid".as_slice(),
        b"x<: bad",
        b" missing",
        b"missing",
        b"X: ok\r\n\tfolded",
    ] {
        let form = [b"--B\r\n".as_slice(), line].concat();
        for end in 5..=form.len() {
            forms.push(form[..end].to_vec());
        }
    }
    for name in ["file", "ignored"] {
        for qp in [false, true] {
            // Unknown malformed QP fields remain a separate compatibility gap.
            if name == "ignored" && qp {
                continue;
            }
            let mut head = header(name, "file", "\r\n");
            if qp {
                head.truncate(head.len() - 2);
                head.extend_from_slice(b"Content-Transfer-Encoding: quoted-printable\r\n\r\n");
            }
            for data in [
                b"prefix".as_slice(),
                b"prefix=00",
                b"prefix=",
                b"prefix=0",
                b"prefix\x00",
            ] {
                let form = [head.as_slice(), data, b"\r\n--B--\r\n"].concat();
                for end in head.len()..=form.len() {
                    forms.push(form[..end].to_vec());
                }
            }
        }
    }
    let prior = [header("file", "prior", "\r\n"), b"done\r\n".to_vec()].concat();
    let additions: Vec<_> = forms
        .iter()
        .filter(|s| s.starts_with(b"--B\r\n"))
        .map(|s| [prior.as_slice(), s].concat())
        .collect();
    forms.extend(additions);
    forms
}

// Real GzipBody -> bounded MIME/QP readers; in-memory file observations only.
// This stops at MIME End like the original Go method. Linux batch tests also
// check our separate strict gzip/HTTP drain after that point.
async fn observe_gzip(
    input: &[u8],
    chunk: usize,
    source_chunk: usize,
) -> (String, Option<Rejection>, Vec<usize>) {
    let mut body = gzip_source(input, source_chunk).await;
    let mut decoder = Decoder::new_read_buffered("B", limits(chunk)).unwrap();
    let mut pending = Bytes::new();
    let mut reads = Vec::new();
    let mut cause = None;
    let mut writes: Vec<(Vec<u8>, bool)> = vec![];
    let mut file = false;
    let mut qp: Option<QuotedPrintable> = None;
    let mut discard: Option<Discard> = None;
    let failure = loop {
        match next_event(&mut decoder, &mut discard) {
            Ok(Some(Event::Part(part))) => {
                file = part.name.as_deref() == Some("file");
                qp = (file && part.quoted_printable).then(|| QuotedPrintable::new(chunk).unwrap());
                discard = (!file).then(|| Discard::new(part.quoted_printable, chunk).unwrap());
                if file {
                    writes.push((vec![], false));
                }
            }
            Ok(Some(Event::Data(bytes))) => {
                if file {
                    if let Some(reader) = &mut qp {
                        let end = decoder.data_read_end();
                        let output = reader.feed_reader(&bytes, end);
                        writes.last_mut().unwrap().0.extend(output.bytes);
                        if let Some(error) = output.error {
                            break Some(match error {
                                QuotedPrintableReadError::Decode(DecodeError::Truncated) => {
                                    assert_eq!(end, PartReadEnd::Failed(DecodeError::Truncated));
                                    Rejection::MultipartGzip {
                                        stage: SourceStage::Body,
                                        error: cause.unwrap(),
                                    }
                                }
                                error => qp_rejection(error),
                            });
                        }
                        if end == PartReadEnd::Eof {
                            qp = None;
                        }
                    } else {
                        writes.last_mut().unwrap().0.extend(bytes);
                    }
                }
            }
            Ok(Some(Event::PartEnd)) => {
                discard = None;
                if let Some(mut reader) = qp.take() {
                    let output = reader.finish_detailed();
                    writes.last_mut().unwrap().0.extend(output.bytes);
                    if let Some(error) = output.error {
                        break Some(qp_rejection(error));
                    }
                }
                if file {
                    writes.last_mut().unwrap().1 = true;
                }
                file = false;
            }
            Ok(Some(Event::End)) => break None,
            Ok(None) => {
                assert!(
                    cause.is_none(),
                    "parser asked for input after source failure"
                );
                cause = feed_gzip(&mut body, &mut pending, &mut decoder, chunk, &mut reads).await;
            }
            Err(ReadError::Source(mut stage)) => {
                if let Some(reader) = &mut qp {
                    let output = reader.abort_detailed(DecodeError::Truncated);
                    writes.last_mut().unwrap().0.extend(output.bytes);
                    if let Some(error @ QuotedPrintableReadError::Encoding(_)) = output.error {
                        break Some(qp_rejection(error));
                    }
                }
                if stage == SourceStage::Body && !file {
                    stage = SourceStage::NextPart;
                }
                break Some(Rejection::MultipartGzip {
                    stage,
                    error: cause.unwrap(),
                });
            }
            Err(error) => break Some(mime_rejection(error)),
        }
    };
    let mut effects = String::new();
    for (bytes, complete) in writes {
        write!(effects, "W:{}:{complete};", hex(&bytes)).unwrap();
    }
    (effects, failure, reads)
}

async fn gzip_source(input: &[u8], chunk: usize) -> GzipBody {
    let frames: Vec<_> = input
        .chunks(chunk)
        .map(|v| Ok::<_, std::io::Error>(Bytes::copy_from_slice(v)))
        .collect();
    GzipBody::open(
        Body::from_stream(stream::iter(frames)),
        Some(input.len() as u64),
        gzip_limits(chunk),
        Instant::now() + Duration::from_secs(30),
        Arc::new(()),
    )
    .await
    .unwrap()
}

async fn feed_gzip(
    body: &mut GzipBody,
    pending: &mut Bytes,
    decoder: &mut Decoder,
    chunk: usize,
    reads: &mut Vec<usize>,
) -> Option<GzipReadError> {
    if pending.is_empty() {
        match body.next().await {
            Ok(Some(bytes)) => {
                reads.push(bytes.as_ref().len());
                *pending = Bytes::from_owner(bytes);
            }
            Ok(None) => panic!("damaged gzip returned EOF"),
            Err(error) => {
                reads.push(0);
                decoder.fail_source().unwrap();
                return Some(GzipReadError::try_from(error).unwrap());
            }
        }
    }
    let count = pending.len().min(decoder.source_read_capacity().unwrap());
    let bytes = pending.split_to(count);
    let fragments = bytes.chunks(chunk);
    let length = fragments.len();
    for (i, fragment) in fragments.enumerate() {
        decoder.feed_read(fragment, i + 1 == length).unwrap();
    }
    None
}

#[tokio::test]
#[ignore = "requires explicit fixed Go upload reference and local loopback HTTP"]
async fn gzip_source_stage_prefix_completion_and_precedence_match_original_go() {
    let mut reference = Reference::from_env("SANDPROOF_GUEST_UPLOAD_REFERENCE");
    let forms = forms();
    for form in &forms {
        for encoded in damaged(form) {
            for chunk in [1, 7, 128] {
                let (effects, error, _) = observe_gzip(&encoded, chunk, chunk).await;
                let suffix = if let Some(error) = error {
                    wire(
                        rejected(
                            error,
                            Limits {
                                wire_bytes: 65536,
                                chunk_bytes: 37,
                            },
                            Instant::now() + Duration::from_secs(30),
                            Arc::new(()),
                        )
                        .await
                        .unwrap(),
                    )
                    .await
                } else {
                    "END".into()
                };
                reference.check(
                    &format!("Z\t{}", hex(&encoded)),
                    &format!("{effects}\t{suffix}"),
                );
            }
        }
    }
    reference.finish();
    eprintln!(
        "Fixed Go gzip/MIME: {} forms x 3 source failures x 3 chunks; exact prefixes, completion and HTTP.",
        forms.len()
    );
}

#[tokio::test]
#[ignore = "requires explicit fixed Go upload reference and local loopback HTTP"]
async fn ignored_qp_with_gzip_keeps_decoded_source_reads_and_error_stages() {
    let mut reference = Reference::from_env("SANDPROOF_GUEST_UPLOAD_REFERENCE");
    let forms = super::ignored::forms();
    for form in &forms {
        for encoded in damaged(form) {
            for source in [1, 7, 128, 4096] {
                for chunk in [1, 128] {
                    let (effects, error, reads) = observe_gzip(&encoded, chunk, source).await;
                    let suffix = if let Some(error) = error {
                        wire(
                            rejected(
                                error,
                                Limits {
                                    wire_bytes: 65536,
                                    chunk_bytes: 37,
                                },
                                Instant::now() + Duration::from_secs(30),
                                Arc::new(()),
                            )
                            .await
                            .unwrap(),
                        )
                        .await
                    } else {
                        "END".into()
                    };
                    let schedule = reads
                        .iter()
                        .map(usize::to_string)
                        .collect::<Vec<_>>()
                        .join(",");
                    reference.check(
                        &format!("Y\t{}\t{schedule}", hex(&encoded)),
                        &format!("{effects}\t{suffix}"),
                    );
                }
            }
        }
    }
    reference.finish();
    eprintln!(
        "Fixed Go ignored QP/gzip: {} forms x 3 failures x 4 decoded reads x 2 event fragments.",
        forms.len()
    );
}
