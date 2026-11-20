//! Bounded final replies from the already authenticated original API socket.
use super::RuntimeError;
use std::io::Read;

pub(super) fn read_reply(input: &mut impl Read) -> Result<(u16, Vec<u8>), RuntimeError> {
    let mut response = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        if let Some(end) = response.windows(4).position(|part| part == b"\r\n\r\n") {
            if end > 65_536 {
                return Err(RuntimeError::Protocol("API headers exceeded limit"));
            }
            break end;
        }
        if response.len() > 65_536 {
            return Err(RuntimeError::Protocol("API headers exceeded limit"));
        }
        let count = input.read(&mut chunk)?;
        if count == 0 {
            return Err(RuntimeError::Protocol("API closed before headers"));
        }
        response.extend_from_slice(&chunk[..count]);
    };
    let header = std::str::from_utf8(&response[..header_end])
        .map_err(|_| RuntimeError::Protocol("non-UTF-8 HTTP headers"))?;
    let mut lines = header.split("\r\n");
    let mut status_line = lines.next().unwrap_or_default().splitn(3, ' ');
    if status_line.next() != Some("HTTP/1.1") {
        return Err(RuntimeError::Protocol("unexpected API HTTP version"));
    }
    let code = status_line.next().unwrap_or_default();
    let status: u16 = code
        .parse()
        .map_err(|_| RuntimeError::Protocol("missing HTTP status"))?;
    if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_digit()) || !(200..600).contains(&status)
    {
        return Err(RuntimeError::Protocol(
            "API reply is not a final HTTP status",
        ));
    }
    let mut declared_length = None;
    for line in lines {
        let (key, value) = line
            .split_once(':')
            .ok_or(RuntimeError::Protocol("invalid API header"))?;
        if key.is_empty()
            || key.bytes().any(|b| b.is_ascii_whitespace())
            || value.contains(['\r', '\n'])
        {
            return Err(RuntimeError::Protocol("invalid API header"));
        }
        if key.eq_ignore_ascii_case("transfer-encoding") {
            return Err(RuntimeError::Protocol("unsupported API response framing"));
        }
        if key.eq_ignore_ascii_case("content-length") {
            let value = value.trim();
            if declared_length.is_some()
                || value.is_empty()
                || !value.bytes().all(|b| b.is_ascii_digit())
            {
                return Err(RuntimeError::Protocol("ambiguous API Content-Length"));
            }
            declared_length = Some(
                value
                    .parse::<usize>()
                    .map_err(|_| RuntimeError::Protocol("invalid API Content-Length"))?,
            );
        }
    }
    let content_length = if status == 204 || status == 304 {
        if declared_length.is_some_and(|length| length != 0) {
            return Err(RuntimeError::Protocol("body forbidden for this API status"));
        }
        0
    } else {
        declared_length.ok_or(RuntimeError::Protocol("missing Content-Length"))?
    };
    if content_length > 1_048_576 {
        return Err(RuntimeError::Protocol("API response exceeded limit"));
    }
    let body_start = header_end + 4;
    while response.len() - body_start < content_length {
        let count = input.read(&mut chunk)?;
        if count == 0 {
            return Err(RuntimeError::Protocol("API closed before response body"));
        }
        response.extend_from_slice(&chunk[..count]);
    }
    if response.len() - body_start != content_length {
        return Err(RuntimeError::Protocol("surplus API response bytes"));
    }
    Ok((status, response[body_start..].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_success_and_complete_failure_remain_distinct() {
        assert_eq!(
            read_reply(&mut &b"HTTP/1.1 204 No Content\r\n\r\n"[..]).unwrap(),
            (204, vec![])
        );
        assert_eq!(
            read_reply(&mut &b"HTTP/1.1 400 Bad Request\r\nContent-Length: 3\r\n\r\nbad"[..])
                .unwrap(),
            (400, b"bad".to_vec())
        );
    }

    #[test]
    fn incomplete_interim_or_ambiguous_replies_never_establish_api_end() {
        for response in [
            &b"HTTP/1.1 204 No Content\r\n"[..],
            &b"HTTP/1.1 400 Bad Request\r\nContent-Length: 4\r\n\r\nbad"[..],
            &b"HTTP/1.1 100 Continue\r\n\r\n"[..],
            &b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n"[..],
            &b"HTTP/1.1 200 OK\r\nContent-Length: invalid\r\n\r\n"[..],
            &b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nTransfer-Encoding: chunked\r\n\r\n"[..],
            &b"HTTP/1.1 204 No Content\r\nContent-Length: 1\r\n\r\nx"[..],
            &b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\nx"[..],
        ] {
            let mut input = response;
            assert!(read_reply(&mut input).is_err());
        }
    }
}
