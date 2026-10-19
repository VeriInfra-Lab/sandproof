//! Loopback-only SDK regression helper. Runs the ORIGINAL gateway and a real
//! stdio relay; only durable authorization/namespace placement are controlled.
//! Not a deployable server or a VM/network-namespace acceptance test.
#[cfg(target_os = "linux")]
mod linux {
    use axum::{Router, body::Body, http::Request};
    use sandbox_controller_core::VmId;
    use sandproof_api::admission::{RequestAdmission, RequestAdmissionLimits, RequestClass};
    use sandproof_controller_service::application::{
        ApplicationError, DataAccessPort, DataAuthorization, DataKind, DataLease, DataTarget,
        RelayReaper, TrackedRelayChild,
    };
    use sandproof_host_service::data_gateway::{self, Gateway, GatewayListener, TransferLimits};
    use std::{
        io::Write,
        net::SocketAddr,
        process::{Command, Stdio},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::watch,
    };

    struct Permit {
        revocation: watch::Receiver<bool>,
        lease: DataLease,
    }
    impl DataAuthorization for Permit {
        fn target(&self) -> DataTarget {
            DataTarget {
                vm: VmId(7),
                generation: 3,
            }
        }
        fn revocation(&mut self) -> &mut watch::Receiver<bool> {
            &mut self.revocation
        }
        fn drain(&self) -> &DataLease {
            &self.lease
        }
    }
    struct Port {
        upstream: SocketAddr,
        revocation: watch::Sender<bool>,
        lease: DataLease,
        authorized: AtomicUsize,
        launched: AtomicUsize,
    }
    impl DataAccessPort for Port {
        type Permit = Permit;
        fn data_permit(&self, token: &str, _: DataKind) -> Result<Permit, ApplicationError> {
            if token != "token" {
                return Err(ApplicationError::NotFound);
            }
            self.authorized.fetch_add(1, Ordering::SeqCst);
            Ok(Permit {
                revocation: self.revocation.subscribe(),
                lease: self.lease.clone(),
            })
        }
        fn spawn_data_relay(
            &self,
            _: &Permit,
            _: bool,
        ) -> Result<TrackedRelayChild, ApplicationError> {
            self.launched.fetch_add(1, Ordering::SeqCst);
            let reaper = RelayReaper::new()?;
            let child = Command::new(std::env::current_exe().map_err(|_| ApplicationError::Busy)?)
                .args(["--relay", &self.upstream.to_string()])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(|_| ApplicationError::Busy)?;
            Ok(reaper.attach(child))
        }
    }

    async fn relay(address: SocketAddr) {
        let socket = TcpStream::connect(address).await.unwrap();
        let (mut read, mut write) = socket.into_split();
        let input = async {
            let mut stdin = tokio::io::stdin();
            tokio::io::copy(&mut stdin, &mut write).await.unwrap();
            write.shutdown().await.unwrap();
        };
        let output = async {
            let mut stdout = tokio::io::stdout();
            tokio::io::copy(&mut read, &mut stdout).await.unwrap();
            stdout.shutdown().await.unwrap();
        };
        tokio::join!(input, output);
    }

    pub async fn run() {
        let arguments: Vec<_> = std::env::args().skip(1).collect();
        let (is_relay, address) = match arguments.as_slice() {
            [flag, address] if flag == "--relay" => (true, address),
            [address] => (false, address),
            _ => panic!("expected one upstream loopback address or --relay address"),
        };
        let address: SocketAddr = address.parse().unwrap();
        assert!(address.ip().is_loopback());
        if is_relay {
            relay(address).await;
            return;
        }
        let limits = TransferLimits {
            file_upload_bytes: 16384,
            file_download_bytes: 16384,
            command_input_bytes: 16384,
            rpc_request_bytes: 16384,
            rpc_response_bytes: 16384,
            max_transfers: 2,
            max_transfers_per_vm: 2,
            control_reserve: 1,
            request_idle_seconds: 5,
            file_idle_seconds: 5,
            write_idle_seconds: 5,
        };
        let admission = RequestAdmission::new(RequestAdmissionLimits {
            max_pending_requests: 3,
            control_reserve: 1,
        })
        .unwrap();
        let gateway = Arc::new(Gateway::with_admission(limits, Arc::clone(&admission)).unwrap());
        let port = Arc::new(Port {
            upstream: address,
            revocation: watch::channel(false).0,
            lease: DataLease::default(),
            authorized: AtomicUsize::new(0),
            launched: AtomicUsize::new(0),
        });
        let handler_port = Arc::clone(&port);
        let router = Router::new().fallback(move |request: Request<Body>| {
            data_gateway::handle(
                Arc::clone(&handler_port),
                Some(Arc::clone(&gateway)),
                request,
            )
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        println!("{}", listener.local_addr().unwrap());
        std::io::stdout().flush().unwrap();
        axum::serve(
            GatewayListener::new(listener, Some(Duration::from_secs(5))),
            router,
        )
        .with_graceful_shutdown(async {
            let mut stop = [0];
            assert_eq!(tokio::io::stdin().read(&mut stop).await.unwrap(), 0);
        })
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(6), async {
            while !port.lease.drained() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("original gateway relay/response pins did not drain");
        assert_eq!(port.authorized.load(Ordering::SeqCst), 1);
        assert_eq!(port.launched.load(Ordering::SeqCst), 1);
        let ordinary: Vec<_> = (0..2)
            .map(|_| admission.try_acquire(RequestClass::Ordinary).unwrap())
            .collect();
        let control = admission.try_acquire(RequestClass::Control).unwrap();
        assert!(admission.try_acquire(RequestClass::Control).is_none());
        drop((ordinary, control));
    }
}

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() {
    linux::run().await;
}

#[cfg(not(target_os = "linux"))]
fn main() {
    panic!("SDK gateway regression helper requires Linux");
}
