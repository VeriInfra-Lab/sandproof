// Stateful boundary tests for the actual generic load candidate. These injected
// values are not a claim about the real SQLite/Serde provider or its snapshot.
use crate::controller::checkpoint::{MaintenanceConfig, ReadConfigEffects};
use crate::controller::lifecycle_inbox::codec::InboxLoadEffects;

const EXISTS: &str = "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='controller_lifecycle_inbox')";
const VERSION: &str = "SELECT version FROM controller_meta WHERE id=1";
const SOURCE: &str =
    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='controller_source_work')";
const INTERNAL: &str = "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='controller_internal_inbox')";
const PRAGMA: &str = "PRAGMA user_version";
const CONFIG: &str = "SELECT config_json FROM controller_maintenance_config WHERE id=1";
const LENGTH: &str =
    "SELECT length(state_json),reserve_bytes FROM controller_lifecycle_inbox WHERE id=1";
const BODY: &str = "SELECT state_json,sha256 FROM controller_lifecycle_inbox WHERE id=1";

#[derive(Debug, PartialEq, Eq, Clone)]
enum LoadCall {
    Bool(&'static str),
    I64(&'static str),
    Bytes(&'static str),
    PairI64,
    PairBytes,
    DecodeConfig,
    DecodeInbox,
    Digest,
    EncodeInbox,
}

struct LoadTape {
    calls: Vec<LoadCall>,
    fail_at: Option<usize>,
    inbox: Inbox,
    policies: [MaintenanceConfig; 2],
    policy_index: usize,
    length: i64,
    corrupt_digest: bool,
}

impl LoadTape {
    fn valid() -> Self {
        Self {
            calls: vec![],
            fail_at: None,
            inbox: empty(),
            policies: [
                lifecycle_application::maintenance(),
                lifecycle_application::maintenance(),
            ],
            policy_index: 0,
            length: 2,
            corrupt_digest: false,
        }
    }
    fn step(&mut self, call: LoadCall) -> Result<(), JournalError> {
        self.calls.push(call);
        if self.fail_at == Some(self.calls.len()) {
            Err(JournalError::MaintenanceCapacity)
        } else {
            Ok(())
        }
    }
    fn run(&mut self) -> Result<Option<Inbox>, JournalError> {
        codec::load_using::<
            crate::controller::lifecycle_inbox::native_retirement::StandardRetirementEncoding,
            crate::controller::lifecycle_inbox::model::StandardTerminalEncoding,
            crate::controller::lifecycle_inbox::renewal_confirmation::StandardRenewalEncoding,
            crate::controller::lifecycle_inbox::unload_resolution::StandardUnloadEncoding,
            crate::capture::StandardGraphicBytes,
            _,
        >(self)
    }
}

impl ReadConfigEffects for LoadTape {
    fn query_i64(&mut self, statement: &str) -> Result<i64, JournalError> {
        let (name, value) = match statement {
            VERSION => (VERSION, crate::LIFECYCLE_STORE_FORMAT_VERSION),
            PRAGMA => (PRAGMA, crate::journal::MAINTAINED_FORMAT_VERSION),
            _ => panic!("unexpected SQL: {statement}"),
        };
        self.step(LoadCall::I64(name))?;
        Ok(value)
    }
    fn query_bytes(&mut self, statement: &str) -> Result<Vec<u8>, JournalError> {
        assert_eq!(statement, CONFIG);
        self.step(LoadCall::Bytes(CONFIG))?;
        Ok(vec![7])
    }
    fn decode_maintenance(&mut self, bytes: &[u8]) -> Result<MaintenanceConfig, JournalError> {
        assert_eq!(bytes, &[7]);
        self.step(LoadCall::DecodeConfig)?;
        let policy = self.policies[self.policy_index].clone();
        self.policy_index += 1;
        Ok(policy)
    }
}

impl InboxLoadEffects for LoadTape {
    fn query_bool(&mut self, statement: &str) -> Result<bool, JournalError> {
        let (name, value) = match statement {
            EXISTS => (EXISTS, true),
            SOURCE => (SOURCE, true),
            INTERNAL => (INTERNAL, false),
            _ => panic!("unexpected SQL: {statement}"),
        };
        self.step(LoadCall::Bool(name))?;
        Ok(value)
    }
    fn query_i64_pair(&mut self, statement: &str) -> Result<(i64, i64), JournalError> {
        assert_eq!(statement, LENGTH);
        self.step(LoadCall::PairI64)?;
        Ok((
            self.length,
            i64::try_from(self.inbox.config.required_bytes()).unwrap(),
        ))
    }
    fn query_bytes_pair(&mut self, statement: &str) -> Result<(Vec<u8>, Vec<u8>), JournalError> {
        assert_eq!(statement, BODY);
        self.step(LoadCall::PairBytes)?;
        Ok((
            vec![8, 9],
            vec![if self.corrupt_digest { 4 } else { 3 }; 32],
        ))
    }
    fn decode_inbox(&mut self, bytes: &[u8]) -> Result<Inbox, JournalError> {
        assert_eq!(bytes, &[8, 9]);
        self.step(LoadCall::DecodeInbox)?;
        Ok(self.inbox.clone())
    }
    fn encode_inbox(&mut self, inbox: &Inbox) -> Result<Vec<u8>, JournalError> {
        assert_eq!(inbox, &self.inbox);
        self.step(LoadCall::EncodeInbox)?;
        Ok(vec![8, 9])
    }
    fn digest(&mut self, bytes: &[u8]) -> [u8; 32] {
        assert_eq!(bytes, &[8, 9]);
        self.calls.push(LoadCall::Digest);
        [3; 32]
    }
}

fn full_load_calls() -> Vec<LoadCall> {
    vec![
        LoadCall::Bool(EXISTS),
        LoadCall::I64(VERSION),
        LoadCall::Bool(SOURCE),
        LoadCall::Bool(INTERNAL),
        LoadCall::I64(PRAGMA),
        LoadCall::Bytes(CONFIG),
        LoadCall::DecodeConfig,
        LoadCall::PairI64,
        LoadCall::PairBytes,
        LoadCall::DecodeInbox,
        LoadCall::Digest,
        LoadCall::EncodeInbox,
        LoadCall::I64(PRAGMA),
        LoadCall::Bytes(CONFIG),
        LoadCall::DecodeConfig,
    ]
}

#[test]
fn load_preserves_full_arguments_and_both_configuration_read_traces() {
    for length in [2, 1] {
        let mut tape = LoadTape::valid();
        tape.length = length;
        assert_eq!(tape.run().unwrap(), Some(tape.inbox.clone()));
        assert_eq!(tape.calls, full_load_calls());
        assert_eq!(tape.policy_index, 2);
    }
    // The injected length=1 with two returned bytes demonstrates why provider
    // snapshot coherence is separate from this exact call-and-value contract.
}

#[test]
fn load_propagates_each_first_effect_error_with_the_exact_prefix_state() {
    for fail_at in (1..=15).filter(|index| *index != 11) {
        let mut tape = LoadTape::valid();
        tape.fail_at = Some(fail_at);
        let result = tape.run();
        if fail_at == 10 {
            assert!(matches!(result, Err(JournalError::CorruptStore)));
        } else {
            assert!(matches!(result, Err(JournalError::MaintenanceCapacity)));
        }
        assert_eq!(tape.calls, full_load_calls()[..fail_at]);
    }
}

#[test]
fn digest_rejection_precedes_reencoding_and_second_configuration() {
    let mut tape = LoadTape::valid();
    tape.corrupt_digest = true;
    assert!(matches!(tape.run(), Err(JournalError::CorruptStore)));
    assert_eq!(tape.calls, full_load_calls()[..11]);
}

#[test]
fn complete_inbox_validation_still_precedes_second_configuration() {
    let mut tape = LoadTape::valid();
    tape.inbox.lanes.clear();
    assert!(matches!(tape.run(), Err(JournalError::CorruptStore)));
    assert_eq!(tape.calls, full_load_calls()[..12]);
}

#[test]
fn second_decoded_configuration_runs_the_actual_business_validator() {
    let mut tape = LoadTape::valid();
    tape.policies[1].retention_ms = 0;
    assert!(matches!(tape.run(), Err(JournalError::UnsupportedStore)));
    assert_eq!(tape.calls, full_load_calls());
}

#[test]
fn final_operation_bound_uses_the_second_valid_configuration() {
    let mut tape = LoadTape::valid();
    tape.policies[1].max_operations_per_sandbox = 3;
    tape.policies[1].validate().unwrap();
    assert!(tape.inbox.config.max_operations_per_session > 3);
    assert!(matches!(tape.run(), Err(JournalError::CorruptStore)));
    assert_eq!(tape.calls, full_load_calls());
}

#[test]
fn negative_length_rejects_before_reading_any_inbox_bytes() {
    let mut tape = LoadTape::valid();
    tape.length = -1;
    assert!(matches!(tape.run(), Err(JournalError::CorruptStore)));
    assert_eq!(tape.calls, full_load_calls()[..8]);
}
