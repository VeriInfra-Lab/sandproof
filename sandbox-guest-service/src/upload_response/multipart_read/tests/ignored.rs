use super::*;

pub(super) fn forms() -> Vec<Vec<u8>> {
    let mut head = header("ignored", "ignored", "\r\n");
    head.truncate(head.len() - 2);
    head.extend_from_slice(b"Content-Transfer-Encoding: quoted-printable\r\n\r\n");
    let following = [
        b"\r\n".as_slice(),
        &header("file", "next", "\r\n"),
        b"next\r\n--B--\r\n",
    ]
    .concat();
    let mut values = vec![];
    for bad in [
        b"bad\0\n".as_slice(),
        b"bad=\rX\n",
        b"bad=\r\r\n",
        b"=\n",
        b"ok=00\n",
    ] {
        for tail in [
            b"".as_slice(),
            b"extra\n",
            b"\r\n",
            b"--B--\r\n",
            b"extra\r\nmore\n",
        ] {
            values.push([bad, tail].concat());
        }
    }
    for length in [4095, 4096, 4097] {
        values.push(vec![b'x'; length]);
        values.push([b"bad\0\n".as_slice(), &vec![b'x'; length], b"\n"].concat());
        values.push([vec![b'x'; length], b"\0\nextra\n".to_vec()].concat());
    }
    let mut forms = Vec::new();
    for prior in [
        vec![],
        [header("file", "prior", "\r\n"), b"prior\r\n".to_vec()].concat(),
    ] {
        for value in &values {
            forms.push([prior.as_slice(), &head, value, &following].concat());
        }
    }
    forms
}

#[tokio::test]
#[ignore = "requires explicit fixed Go upload reference and local loopback HTTP"]
async fn ignored_qp_close_preserves_prefetch_and_next_part_errors() {
    let mut reference = Reference::from_env("SANDPROOF_GUEST_UPLOAD_REFERENCE");
    let forms = forms();
    for form in &forms {
        for source in [1, 7, 128, 4096] {
            for chunk in [1, 128] {
                check_reads(&mut reference, form, chunk, false, Some(source)).await;
            }
        }
    }
    reference.finish();
    eprintln!(
        "Fixed Go ignored QP Close: {} forms x 4 source reads x 2 event fragments; following file effects and exact HTTP.",
        forms.len()
    );
}
