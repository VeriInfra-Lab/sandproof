use super::*;

#[cfg(target_os = "linux")]
#[test]
fn native_resize_fact_requires_positive_errno_and_original_coarse_result() {
    use crate::command::CommandError;
    for errno in [-1, 0, 1, 5, 22, 25] {
        let fact = Rejection::NativeResize(errno);
        let coarse = crate::command::input_error(&std::io::Error::from_raw_os_error(errno));
        assert_eq!(fact.matches(&RequestError::Command(coarse)), errno > 0);
        for error in [
            RequestError::DeadlineExceeded,
            RequestError::Command(CommandError::ClosedInput),
            RequestError::Command(CommandError::StaleIdentity),
            RequestError::Command(if coarse == CommandError::InvalidArgument {
                CommandError::Unavailable
            } else {
                CommandError::InvalidArgument
            }),
        ] {
            assert!(!fact.matches(&error));
        }
    }
}

#[tokio::test]
#[ignore = "explicit fixed Go resize error rendering; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn fixed_go_resize_replies_keep_ioctl_errno_before_complete_http_rendering() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut count = 0;
    for (errno, name) in [
        (1, "EPERM"),
        (25, "ENOTTY"),
        (4, "EINTR"),
        (5, "EIO"),
        (9, "EBADF"),
        (22, "EINVAL"),
        (38, "ENOSYS"),
        (i32::MAX, "UNKNOWN"),
    ] {
        for utf8 in [false, true] {
            for gzip in [false, true] {
                let response = send(
                    source(
                        Some(Rejection::NativeResize(errno)),
                        Some(format(Rpc::Update, gzip, utf8)),
                        Arc::new(()),
                    ),
                    limits(),
                )
                .await
                .unwrap();
                let mut observed = vec![response.status().as_u16().to_string()];
                for name in [
                    "content-type",
                    "content-encoding",
                    "connect-content-encoding",
                    "accept-encoding",
                    "connect-accept-encoding",
                    "content-length",
                    "allow",
                ] {
                    observed.push(hex(response
                        .headers()
                        .get(name)
                        .map_or(b"", |v| v.as_bytes())));
                }
                let bytes = drain(response.into_body()).await;
                observed.push(String::new());
                observed.push(hex(&bytes));
                let input = serde_json::json!({"Method":"Update", "Kind":"native-resize", "Native":name, "Gzip":gzip,"UTF8":utf8});
                oracle.check(
                    &format!("R\t{}", hex(&serde_json::to_vec(&input).unwrap())),
                    &observed.join("\t"),
                );
                count += 1;
            }
        }
    }
    oracle.finish();
    eprintln!(
        "{count} fixed Go resize error replies; controlled syscall observations, no original process resize."
    );
}
