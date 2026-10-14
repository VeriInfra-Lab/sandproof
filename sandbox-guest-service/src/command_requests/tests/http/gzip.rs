use super::*;
use crate::command_requests::http::Encoding;
use crate::file_rpc_requests::tests::gzip::compressed;

fn wire(flag: u8, payload: &[u8]) -> Vec<u8> {
    let mut result = vec![flag];
    result.extend(u32::try_from(payload.len()).unwrap().to_be_bytes());
    result.extend(payload);
    result
}
fn chunks(bytes: &[u8]) -> Body {
    Body::from_stream(stream::iter(
        bytes
            .chunks(17)
            .map(|b| Ok::<_, std::io::Error>(Bytes::copy_from_slice(b)))
            .collect::<Vec<_>>(),
    ))
}

#[tokio::test]
async fn selected_gzip_preserves_request_format_deadline_and_original_capacity() {
    let session = session().await;
    let pool = CommandRequests::new(limits()).unwrap();
    for rpc in [
        Rpc::Start,
        Rpc::Connect,
        Rpc::List,
        Rpc::SendInput,
        Rpc::CloseStdin,
        Rpc::SendSignal,
        Rpc::Update,
    ] {
        let mut h = headers();
        h.insert(
            "content-type",
            if rpc.is_stream() {
                "application/connect+json"
            } else {
                "application/json"
            }
            .parse()
            .unwrap(),
        );
        h.insert(
            if rpc.is_stream() {
                "connect-content-encoding"
            } else {
                "content-encoding"
            },
            "gzip".parse().unwrap(),
        );
        let selected = prepare_headers(&pool, &session, h)
            .await
            .select_json(&Method::POST, rpc)
            .unwrap();
        assert_eq!(selected.request_encoding(), Encoding::Gzip);
        assert_eq!(selected.response_encoding(), Encoding::Gzip);
        let deadline = selected.deadline();
        let member = [compressed(b"{"), compressed(b"}")].concat();
        let payload = if rpc.is_stream() {
            [wire(1, &member), wire(3, &compressed(b"{}"))].concat()
        } else {
            member
        };
        let ready = selected.decode(chunks(&payload)).await.unwrap();
        let identity = if rpc.is_stream() {
            wire(0, b"{}")
        } else {
            b"{}".to_vec()
        };
        assert_eq!(
            ready.request(),
            &protocol::decode_request(rpc.method(), &identity, 128).unwrap()
        );
        assert_eq!(ready.deadline(), deadline);
        full(&pool);
        drop(ready);
        released(&pool).await;
    }
}
