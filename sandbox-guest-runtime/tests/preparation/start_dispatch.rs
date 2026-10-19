//! Full untagged component path; no public HTTP router or fixed-Go endpoint.
use super::*;
use sandproof_guest_runtime::Credentials;
use sandproof_guest_service::{ExecutionSettings, command_requests::StartPreparation};
use std::path::Path;

#[path = "http_dispatch.rs"]
mod http_dispatch;

struct CurrentAccount(String);
impl AccountSource for CurrentAccount {
    fn lookup(&self, name: &str) -> Result<Account, AccountError> {
        if name != "fixture" {
            return Err(AccountError::NotFound);
        }
        Ok(Account {
            name: name.into(),
            home: self.0.clone(),
            credentials: Credentials {
                uid: rustix::process::geteuid().as_raw(),
                gid: rustix::process::getegid().as_raw(),
                groups: rustix::process::getgroups()
                    .unwrap()
                    .iter()
                    .map(|group| group.as_raw())
                    .collect(),
            },
        })
    }
}
async fn fixture() -> Fixture {
    let fixture = Fixture::new().await;
    fixture.session.initialize(br#"{"accessToken":"token","defaultUser":"fixture","defaultWorkdir":".","envVars":{"VALUE":"default"}}"#).await.unwrap();
    fixture
}
fn accounts(fixture: &Fixture) -> AccountPool<CurrentAccount> {
    AccountPool::new(
        CurrentAccount(fixture.directory.path().to_str().unwrap().into()),
        1,
        Duration::from_secs(5),
    )
    .unwrap()
}
fn settings() -> ExecutionSettings {
    ExecutionSettings {
        daemon_path: "/bin:/usr/bin".into(),
        daemon_workdir: "/".into(),
        limits: ContextLimits {
            bytes: 4096,
            env_entries: 32,
            arguments: 32,
            username_bytes: 64,
            authorization_bytes: 256,
        },
    }
}
fn preparation<'a>(
    fixture: &'a Fixture,
    registry: &'a PidRegistry,
    registrar: &'a PidRegistrar,
    gates: &'a Path,
) -> StartPreparation<'a> {
    StartPreparation {
        owners: &fixture.owners,
        registry,
        registrar,
        binding: binding(),
        session: 1,
        command: 1,
        worker: Path::new(WORKER),
        private_directory: gates,
        process_limits: fixture.spec().process_limits,
        admission_limits: limits(),
    }
}
async fn start(
    fixture: &Fixture,
    registry: &PidRegistry,
    registrar: &PidRegistrar,
    pool: &CommandRequests,
    payload: Value,
) -> BodyDataStream {
    let ready = ready(
        pool,
        fixture,
        "Start",
        payload,
        Instant::now() + Duration::from_secs(30),
    )
    .await;
    let reply = ready
        .start_pid(
            &accounts(fixture),
            &settings(),
            preparation(
                fixture,
                registry,
                registrar,
                &fixture.directory.path().join("gates"),
            ),
            response(),
        )
        .await
        .unwrap();
    assert!(reply.is_stream());
    reply.into_body().into_data_stream()
}
async fn started(body: &mut BodyDataStream, registry: &PidRegistry) -> Stop {
    let event = json_frame(&frame(body).await, 0);
    let pid = u32::try_from(event["event"]["start"]["pid"].as_u64().unwrap()).unwrap();
    // A quick command can already be reaped when its Start frame is consumed.
    // Cleanup uses the registry's original pidfd, never a fresh numeric lookup.
    Stop(registry.connectable(pid).unwrap())
}
async fn data(body: &mut BodyDataStream, channel: &str, expected: &[u8]) {
    let mut output = Vec::new();
    while output.len() < expected.len() {
        let event = json_frame(&frame(body).await, 0);
        output.extend(
            STANDARD
                .decode(event["event"]["data"][channel].as_str().unwrap())
                .unwrap(),
        );
    }
    assert_eq!(output, expected);
}
async fn end(body: &mut BodyDataStream) {
    assert_eq!(
        json_frame(&frame(body).await, 0),
        json!({"event":{"end":{"exited":true,"status":"exit status 0"}}})
    );
    assert_eq!(json_frame(&frame(body).await, 2), json!({}));
    assert!(
        timeout(Duration::from_secs(5), body.next())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn authenticated_start_composes_defaults_and_reconnects_to_the_same_binary_command() {
    let fixture = fixture().await;
    let registry = registry();
    let registrar = PidRegistrar::new(store(&fixture));
    let pool = requests(2);
    let payload = json!({"process":{
        "cmd":"/bin/sh",
        "args":["-c","printf once >> effect; printf '%s|%s|%s|%s' \"$VALUE\" \"$USER\" \"$HOME\" \"$PWD\"; exec /bin/cat"],
        "envs":{"VALUE":"request","USER":"root","HOME":"/fake"}
    }});
    let mut body = start(&fixture, &registry, &registrar, &pool, payload.clone()).await;
    let command = started(&mut body, &registry).await;
    let pid = command.0.identity().pid;
    data(
        &mut body,
        "stdout",
        format!("request|root|/fake|{}", fixture.directory.path().display()).as_bytes(),
    )
    .await;
    assert_eq!(
        std::fs::read(fixture.directory.path().join("effect")).unwrap(),
        b"once"
    );
    let listed: Value = serde_json::from_slice(&registry.list_json(4096).unwrap()).unwrap();
    assert_eq!(
        listed,
        json!({"processes":[{"pid":pid,"config":payload["process"]}]})
    );
    // Losing the initial stream neither closes stdin nor repeats the shell.
    drop(body);
    let mut body = dispatch(
        &pool,
        &fixture,
        &registry,
        "Connect",
        json!({"process":{"pid":pid}}),
    )
    .await
    .into_body()
    .into_data_stream();
    assert_eq!(
        json_frame(&frame(&mut body).await, 0),
        json!({"event":{"start":{"pid":pid}}})
    );
    let bytes: Vec<u8> = (0..=255).collect();
    empty(
        dispatch(
            &pool,
            &fixture,
            &registry,
            "SendInput",
            json!({"process":{"pid":pid},"input":{"stdin":STANDARD.encode(&bytes)}}),
        )
        .await,
    )
    .await;
    data(&mut body, "stdout", &bytes).await;
    empty(
        dispatch(
            &pool,
            &fixture,
            &registry,
            "CloseStdin",
            json!({"process":{"pid":pid}}),
        )
        .await,
    )
    .await;
    end(&mut body).await;
    drop(body);
    assert_eq!(
        std::fs::read(fixture.directory.path().join("effect")).unwrap(),
        b"once"
    );
    clean(&fixture, &registry, command).await;
    let first = pool.admit().unwrap();
    let second = pool.admit().unwrap();
    drop((first, second));
}

#[tokio::test]
async fn start_preserves_disabled_pipe_input_and_initial_pty_dimensions() {
    for pty in [false, true] {
        let fixture = fixture().await;
        let registry = registry();
        let registrar = PidRegistrar::new(store(&fixture));
        let pool = requests(2);
        let script = if pty {
            "stty -echo; stty size; read value; printf 'got:%s' \"$value\""
        } else {
            "if read value; then exit 1; fi; printf closed"
        };
        let mut payload = json!({"process":{"cmd":"/bin/sh","args":["-c",script]},"stdin":false});
        if pty {
            payload["pty"] = json!({"size":{"cols":65616,"rows":65560}});
        }
        let mut body = start(&fixture, &registry, &registrar, &pool, payload).await;
        let command = started(&mut body, &registry).await;
        if pty {
            data(&mut body, "pty", b"24 80\r\n").await;
            empty(dispatch(&pool,&fixture,&registry,"SendInput",json!({"process":{"pid":command.0.identity().pid},"input":{"pty":STANDARD.encode(b"go\n")}})).await).await;
            data(&mut body, "pty", b"got:go").await;
        } else {
            data(&mut body, "stdout", b"closed").await;
        }
        end(&mut body).await;
        drop(body);
        clean(&fixture, &registry, command).await;
    }
}

#[tokio::test]
async fn start_budget_expiry_and_wrong_vm_registration_never_release_user_code() {
    for refusal in 0..4 {
        let fixture = fixture().await;
        let registry = registry();
        let registrar = PidRegistrar::new(store(&fixture));
        let pool = requests(1);
        let payload = json!({"process":config(&fixture)});
        let request = ready(
            &pool,
            &fixture,
            "Start",
            payload,
            Instant::now() + Duration::from_secs(30),
        )
        .await;
        let gates = fixture.directory.path().join("gates");
        let mut preparation = preparation(&fixture, &registry, &registrar, &gates);
        let mut response = response();
        let reservation =
            (refusal == 1).then(|| registry.reserve(&config(&fixture), None).unwrap());
        match refusal {
            0 => response.message_bytes = 1,
            2 => {
                tokio::time::pause();
                tokio::time::advance(Duration::from_secs(31)).await;
                tokio::time::resume();
            }
            3 => preparation.binding = Binding::new([7; 32]).unwrap(),
            _ => {}
        }
        let failure = request
            .start_pid(&accounts(&fixture), &settings(), preparation, response)
            .await
            .err()
            .unwrap();
        assert_eq!(
            failure.error(),
            match refusal {
                0 => Error::Response(
                    sandproof_guest_service::command::CommandBodyError::ResourceExhausted
                ),
                1 => Error::Registry(RegistryError::ResourceExhausted),
                2 => Error::DeadlineExceeded,
                3 => Error::Registration(RegistrationError::Storage(ClaimError::BindingMismatch)),
                _ => unreachable!(),
            }
        );
        drop(failure);
        drop(reservation);
        registry_capacity(&registry).await;
        capacity_restored(&fixture.owners).await;
        fixture.assert_no_effect();
        assert_eq!(fixture.gate_count(), 0);
        drop(pool.admit().unwrap());
    }
}
