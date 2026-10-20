// Appended only to a private copy of the original listener module.
#[cfg(test)]
mod joinset_contract_trial {
    use super::*;
    use std::convert::Infallible;
    use tokio::{
        io::AsyncWriteExt,
        net::TcpStream,
        sync::{Semaphore, oneshot},
    };

    #[tokio::test]
    async fn shutdown_does_not_return_before_original_connection_is_joined() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, stopped) = oneshot::channel();
        let (ack, observed) = oneshot::channel();
        let entered = Arc::new(Semaphore::new(0));
        let release = Arc::new(Semaphore::new(0));
        let noticed = Arc::clone(&entered);
        let gate = Arc::clone(&release);
        let server = tokio::spawn(
            Listener::new(Limits {
                connections: 1,
                header_count: 8,
                buffer_bytes: 8192,
                header_timeout: Duration::from_secs(10),
                connection_timeout: Duration::from_secs(30),
            })
            .unwrap()
            .serve(
                listener,
                move |_| {
                    let (noticed, gate) = (Arc::clone(&noticed), Arc::clone(&gate));
                    async move {
                        noticed.add_permits(1);
                        gate.acquire().await.unwrap().forget();
                        Ok::<_, Infallible>(Response::new(Body::from("original-result")))
                    }
                },
                async move {
                    stopped.await.unwrap();
                    ack.send(()).unwrap();
                },
            ),
        );
        let mut client = TcpStream::connect(address).await.unwrap();
        client
            .write_all(b"GET / HTTP/1.1\r\nHost: local\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        entered.acquire().await.unwrap().forget();
        stop.send(()).unwrap();
        // On the current-thread executor the same serve poll continues after
        // sending ack until it returns Pending or Ready. No timed guess.
        observed.await.unwrap();
        assert!(
            !server.is_finished(),
            "returned before original task was joined"
        );
        release.add_permits(1);
        let report = server.await.unwrap().unwrap();
        assert_eq!(
            (
                report.admitted,
                report.completed,
                report.failed,
                report.expired
            ),
            (1, 1, 0, 0)
        );
    }
}
