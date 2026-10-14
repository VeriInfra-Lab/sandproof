use super::*;
use sandproof_guest_runtime::multipart::{
    BoundaryError, Error as DecodeError, boundary, boundary_detailed,
};

#[test]
fn boundary_budget_and_unsupported_text_remain_distinct_from_public_setup_errors() {
    assert_eq!(
        boundary_detailed("multipart/form-data", 1),
        Err(BoundaryError::Decode(DecodeError::ResourceExhausted))
    );
    assert_eq!(
        boundary_detailed("multipart/form-data; boundary*=UTF-8''%ff", 128),
        Err(BoundaryError::Decode(DecodeError::Malformed))
    );
    for (content, error) in [
        (
            "multipart/related; boundary=B",
            BoundaryRejection::NotMultipart,
        ),
        (
            "multipart/form-data; broken",
            BoundaryRejection::NotMultipart,
        ),
        ("multipart/form-data", BoundaryRejection::MissingBoundary),
        (
            "multipart/form-data; boundary=\"\"",
            BoundaryRejection::EmptyBoundary,
        ),
    ] {
        assert_eq!(
            boundary_detailed(content, 128),
            Err(BoundaryError::Rejected(error))
        );
        assert_eq!(boundary(content, 128), Err(DecodeError::Malformed));
    }
}

#[tokio::test]
#[ignore = "requires explicit fixed Go upload reference and local loopback HTTP"]
async fn multipart_setup_and_zero_body_read_errors_match_fixed_go() {
    let mut reference = Reference::from_env("SANDPROOF_GUEST_UPLOAD_REFERENCE");
    let mut values: Vec<String> = [
        "",
        "text/plain",
        "multipart/related",
        "multipart/mixed",
        "Multipart/Form-Data",
        "multipart/form-data; broken",
        "multipart/form-data; boundary",
        "multipart/form-data; boundary=B; broken",
        "multipart/form-data; boundary=\"\"",
        "multipart/form-data; boundary=B; boundary=C",
        "multipart/form-data; boundary=B; boundary=B",
        "multipart/mixed; boundary=B;",
        "multipart/form-data; boundary*=UTF-8''",
        "multipart/form-data; boundary*=x''B",
        "multipart/form-data; boundary*0=; boundary*1=B",
        "multipart/form-data; boundary*0=\"\"",
        "multipart/form-data; boundary*0=AB; boundary*1=CD",
        "multipart/form-data; boundary*=UTF-8''%42",
        "multipart/form-data; boundary=B; boundary*=UTF-8''C",
        "multipart/form-data; boundary=B; boundary*=UTF-8''",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for value in (0..=127u8)
        .map(char::from)
        .chain(['雪', '\u{2028}', '\u{a0}'])
    {
        for template in [
            "multipart/form-data; boundary=VALUE",
            "multipart/mixed; boundary=\"VALUE\"",
            "multipart/form-data; ignored=VALUE",
            "multipart/related; boundary=\"VALUE\"",
        ] {
            values.push(template.replace("VALUE", &value.to_string()));
        }
    }
    let cases = values.len();
    for value in values {
        for duplicate in [false, true] {
            let mut input = format!("T\t{}", hex(value.as_bytes()));
            if duplicate {
                write!(input, "\t{}", hex(b"multipart/form-data; boundary=later")).unwrap();
            }
            match boundary_detailed(&value, 8192) {
                Ok(_) => reference.check(&input, "READY"),
                Err(BoundaryError::Rejected(error)) => {
                    compare(&mut reference, &input, Rejection::MultipartSetup(error)).await;
                }
                Err(error) => panic!("unexpected component failure {error:?}: {value:?}"),
            }
        }
    }
    reference.finish();
    eprintln!(
        "Fixed Go multipart setup: {cases} content types x 2 header cases; READY means body input reached, errors consumed zero body bytes."
    );
}
