use super::*;

#[tokio::test]
#[ignore = "explicit fixed Go native-input error rendering; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn fixed_go_native_input_replies_match_original_pid_channel_and_symbolic_errno() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut count = 0;
    for (errno, name) in [
        (4, "EINTR"),
        (5, "EIO"),
        (9, "EBADF"),
        (11, "EAGAIN"),
        (12, "ENOMEM"),
        (22, "EINVAL"),
        (28, "ENOSPC"),
        (32, "EPIPE"),
    ] {
        for pid in [1, 65537, u32::MAX] {
            for pty in [false, true] {
                for utf8 in [false, true] {
                    for gzip in [false, true] {
                        let response = send(
                            source(
                                Some(Rejection::NativeInput { pid, pty, errno }),
                                Some(format(Rpc::SendInput, gzip, utf8)),
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
                        let input = serde_json::json!({"Method":"SendInput", "Kind":"native-input", "Number":pid,"Pty":pty,"Native":name,"Gzip":gzip,"UTF8":utf8});
                        oracle.check(
                            &format!("R\t{}", hex(&serde_json::to_vec(&input).unwrap())),
                            &observed.join("\t"),
                        );
                        count += 1;
                    }
                }
            }
        }
    }
    oracle.finish();
    eprintln!(
        "{count} fixed Go native input replies; original formatting with controlled errno, no original process handler."
    );
}
