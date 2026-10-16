use super::*;

#[test]
fn keepalive_selects_only_streams_after_existing_header_errors_under_original_deadline() {
    let now = Instant::now();
    let original = now + Duration::from_secs(30);
    for rpc in [Rpc::Start, Rpc::Connect] {
        let mut h = headers(true);
        h.insert(
            "content-type",
            "application/connect+json; charset=utf-8".parse().unwrap(),
        );
        h.insert("keepalive-ping-interval", "0".parse().unwrap());
        h.insert("connect-content-encoding", "br".parse().unwrap());
        h.insert("connect-protocol-version", "bad".parse().unwrap());
        h.insert("connect-timeout-ms", "bad".parse().unwrap());
        assert!(matches!(
            select(&h, &Method::GET, rpc, 4096, now, original),
            Err((Error::Method { .. }, _, _))
        ));
        assert!(matches!(
            select(&h, &Method::POST, rpc, 4096, now, original),
            Err((Error::Compression(_), _, _))
        ));
        h.insert("connect-content-encoding", "gzip".parse().unwrap());
        assert!(matches!(
            select(&h, &Method::POST, rpc, 4096, now, original),
            Err((Error::Version(_), _, _))
        ));
        h.remove("connect-protocol-version");
        assert!(matches!(
            select(&h, &Method::POST, rpc, 4096, now, original),
            Err((Error::Timeout(_), _, _))
        ));
        h.insert("connect-timeout-ms", "100".parse().unwrap());
        let (error, deadline, format) =
            select(&h, &Method::POST, rpc, 4096, now, original).unwrap_err();
        assert_eq!(
            error,
            Error::Keepalive(crate::command_keepalive::InvalidInterval)
        );
        assert_eq!(deadline, now + Duration::from_millis(100));
        let format = format.unwrap();
        assert_eq!(
            format.content_type(),
            "application/connect+json; charset=utf-8"
        );
        assert_eq!(format.encoding(), Encoding::Gzip);
        h.insert("keepalive-ping-interval", "50".parse().unwrap());
        h.append("keepalive-ping-interval", "0".parse().unwrap());
        let plan = select(&h, &Method::POST, rpc, 4096, now, original).unwrap();
        assert_eq!(plan.keepalive, Some(Duration::from_secs(50)));
        assert_eq!(plan.deadline, deadline);
        h.remove("keepalive-ping-interval");
        assert_eq!(
            select(&h, &Method::POST, rpc, 4096, now, original)
                .unwrap()
                .keepalive,
            Some(Duration::from_secs(90))
        );
    }
    for rpc in [
        Rpc::List,
        Rpc::SendInput,
        Rpc::CloseStdin,
        Rpc::SendSignal,
        Rpc::Update,
    ] {
        let mut h = headers(false);
        h.insert("keepalive-ping-interval", "0".parse().unwrap());
        let plan = select(&h, &Method::POST, rpc, 4096, now, original).unwrap();
        assert_eq!(plan.keepalive, None);
        assert_eq!(plan.deadline, original);
    }
}
