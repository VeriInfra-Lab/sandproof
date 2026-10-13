use super::*;
mod reference;
use wire::date::Utc;
const BOUNDARY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
fn limits() -> Limits {
    Limits {
        request: wire::Limits {
            header_bytes: 65536,
            items: 1024,
        },
        response_header_bytes: 65536,
        wire_bytes: 1 << 20,
    }
}
fn initial() -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in [
        ("content-type", "application/octet-stream"),
        ("vary", "Accept-Encoding"),
        ("content-disposition", "inline"),
        ("cache-control", "private"),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    headers
}
fn plan(method: &Method, request: &HeaderMap, response: HeaderMap, size: u64) -> ResponsePlan {
    match prepare(
        method,
        request,
        response,
        Timestamp::new(784_111_777, 123).unwrap(),
        limits(),
        &Utc,
    )
    .unwrap()
    {
        Preparation::Finished(plan) => plan,
        Preparation::Continue(next) => next
            .finish(
                size,
                HeaderValue::from_static("application/octet-stream"),
                Some(BOUNDARY),
            )
            .unwrap(),
    }
}
fn render(body: &BodyPlan, data: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for segment in body.segments() {
        match segment {
            Segment::Bytes(value) => bytes.extend(value),
            Segment::File(range) => bytes.extend(
                &data[usize::try_from(range.start).unwrap()
                    ..usize::try_from(range.start + range.length).unwrap()],
            ),
        }
    }
    bytes
}

#[test]
fn conditional_completion_precedes_any_length_or_mime_work() {
    let mut request = HeaderMap::new();
    request.insert("if-none-match", HeaderValue::from_static("*"));
    let Preparation::Finished(plan) = prepare(
        &Method::GET,
        &request,
        initial(),
        Timestamp::new(784_111_777, 0).unwrap(),
        limits(),
        &Utc,
    )
    .unwrap() else {
        panic!("304 must stop before length seek")
    };
    assert_eq!(plan.status, StatusCode::NOT_MODIFIED);
    assert!(matches!(plan.body, BodyPlan::Empty));
    assert!(!plan.headers.contains_key("content-type"));
    assert!(!plan.headers.contains_key("content-length"));
    assert!(plan.headers.contains_key("last-modified"));
    request.insert("if-match", HeaderValue::from_static("bad"));
    let result = plan_for_request(&request);
    assert_eq!(result.status, StatusCode::PRECONDITION_FAILED);
    assert_eq!(result.headers["content-type"], "application/octet-stream");
}
fn plan_for_request(request: &HeaderMap) -> ResponsePlan {
    plan(&Method::GET, request, initial(), 5)
}

#[test]
fn single_multiple_and_head_plan_exact_owned_spans_and_mime_lengths() {
    let mut request = HeaderMap::new();
    request.insert("range", HeaderValue::from_static("bytes=1-2"));
    let single = plan_for_request(&request);
    assert_eq!(single.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(single.required_seek, Some(1));
    assert_eq!(render(&single.body, b"01234"), b"12");
    assert_eq!(single.headers["content-range"], "bytes 1-2/5");
    let head = plan(&Method::HEAD, &request, initial(), 5);
    assert!(matches!(head.body, BodyPlan::Empty));
    assert_eq!(head.required_seek, Some(1));
    assert_eq!(head.headers["content-length"], "2");
    request.insert("range", HeaderValue::from_static("bytes=0-0,3-4"));
    let multiple = plan_for_request(&request);
    let bytes = render(&multiple.body, b"01234");
    assert_eq!(multiple.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        multiple.headers["content-length"]
            .to_str()
            .unwrap()
            .parse::<usize>()
            .unwrap(),
        bytes.len()
    );
    assert!(
        bytes.starts_with(format!("--{BOUNDARY}\r\nContent-Range: bytes 0-0/5\r\n").as_bytes())
    );
    assert!(bytes.ends_with(format!("34\r\n--{BOUNDARY}--\r\n").as_bytes()));
}

#[test]
fn range_errors_clear_entity_headers_and_keep_disposition_vary_and_overlap_detail() {
    for (range, no_overlap) in [("bytes=99-", true), ("bytes=bad", false)] {
        let mut request = HeaderMap::new();
        request.insert("range", HeaderValue::from_static(range));
        let result = plan_for_request(&request);
        assert_eq!(result.status, StatusCode::RANGE_NOT_SATISFIABLE);
        assert!(!result.headers.contains_key("last-modified"));
        assert!(!result.headers.contains_key("cache-control"));
        assert_eq!(result.headers["content-type"], "text/plain; charset=utf-8");
        assert_eq!(result.headers["x-content-type-options"], "nosniff");
        assert_eq!(result.headers.contains_key("content-range"), no_overlap);
        assert_eq!(result.headers["vary"], "Accept-Encoding");
        assert_eq!(result.headers["content-disposition"], "inline");
    }
}

fn bounded_plan(
    range: &'static str,
    budget: Limits,
    boundary: Option<&str>,
) -> Result<ResponsePlan, Error> {
    let mut request = HeaderMap::new();
    request.insert("range", HeaderValue::from_static(range));
    let Preparation::Continue(next) = prepare(
        &Method::GET,
        &request,
        initial(),
        Timestamp::new(1, 0).unwrap(),
        budget,
        &Utc,
    )?
    else {
        panic!("no preconditions")
    };
    next.finish(
        5,
        HeaderValue::from_static("application/octet-stream"),
        boundary,
    )
}

#[test]
fn wire_budget_includes_mime_and_errors_and_header_budget_covers_generated_fields() {
    let range = "bytes=0-0,3-4";
    let length = bounded_plan(range, limits(), Some(BOUNDARY))
        .unwrap()
        .body
        .wire_length()
        .unwrap();
    assert!(length > 3);
    assert!(
        bounded_plan(
            range,
            Limits {
                wire_bytes: length,
                ..limits()
            },
            Some(BOUNDARY)
        )
        .is_ok()
    );
    assert!(matches!(
        bounded_plan(
            range,
            Limits {
                wire_bytes: length - 1,
                ..limits()
            },
            Some(BOUNDARY)
        ),
        Err(Error::ResponseLimit)
    ));
    assert!(matches!(
        bounded_plan(
            "bad",
            Limits {
                wire_bytes: 13,
                ..limits()
            },
            None
        ),
        Err(Error::ResponseLimit)
    ));
    assert!(matches!(
        bounded_plan(range, limits(), Some("request-controlled")),
        Err(Error::InvalidConfiguration)
    ));
    let initial_bytes = initial()
        .iter()
        .map(|(name, value)| name.as_str().len() + value.as_bytes().len())
        .sum();
    assert!(matches!(
        bounded_plan(
            "",
            Limits {
                response_header_bytes: initial_bytes,
                ..limits()
            },
            None
        ),
        Err(Error::ResponseLimit)
    ));
}

#[test]
fn first_empty_headers_and_existing_response_metadata_keep_original_semantics() {
    let mut request = HeaderMap::new();
    request.append("if-match", HeaderValue::from_static(""));
    request.append("if-match", HeaderValue::from_static("bad"));
    request.append("range", HeaderValue::from_static(""));
    request.append("range", HeaderValue::from_static("bytes=0-0"));
    let mut response = initial();
    response.insert("content-type", HeaderValue::from_static(""));
    response.insert("content-encoding", HeaderValue::from_static("gzip"));
    response.insert("content-length", HeaderValue::from_static("99"));
    let full = plan(&Method::GET, &request, response.clone(), 5);
    assert_eq!(full.status, StatusCode::OK);
    assert_eq!(full.headers["content-type"], "");
    assert_eq!(full.headers["content-length"], "99");
    request.insert("range", HeaderValue::from_static("bytes=0-0"));
    let partial = plan(&Method::GET, &request, response.clone(), 5);
    assert_eq!(partial.headers["content-length"], "1");
    request.insert("if-none-match", HeaderValue::from_static("*"));
    response.insert("etag", HeaderValue::from_static("\"tag\""));
    let unchanged = plan(&Method::GET, &request, response, 5);
    assert_eq!(unchanged.status, StatusCode::NOT_MODIFIED);
    for name in [
        "content-type",
        "content-length",
        "content-encoding",
        "last-modified",
    ] {
        assert!(!unchanged.headers.contains_key(name));
    }
    assert_eq!(unchanged.headers["etag"], "\"tag\"");
}
