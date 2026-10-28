"""Private load capability candidate; no production adoption or proof claim.

Move only concrete library calls behind typed stateful interfaces. The original
load guards, both configuration reads, and both business validators remain real
Rust bodies. Existing validation candidates are reused without new assumptions.
"""
from pathlib import Path
import hashlib
import importlib.util
import subprocess

MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
CODEC = 'controller-service/src/controller/lifecycle_inbox/codec.rs'
CONFIG = 'controller-service/src/controller/checkpoint/config.rs'
CHECKPOINT = 'controller-service/src/controller/checkpoint.rs'
CAPACITY = 'controller-service/src/controller/capacity.rs'


def once(text, old, new):
    if text.count(old) != 1:
        raise ValueError('ambiguous candidate edit: '+old[:100])
    return text.replace(old, new, 1)


def candidate(project):
    spec = importlib.util.spec_from_file_location('load_full_validation', project /
        'verification/experiments/stage1-protocol/inbox-validation/prepare.py')
    previous = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(previous)
    changes, metadata = previous.prepare(project, 'candidate')
    original = {n: (project/n).read_text() for n in [MODEL, CODEC, CONFIG, CHECKPOINT, CAPACITY]}
    model = once(original[MODEL], '    fn validate_using<', '    pub(super) fn validate_using<')
    model = once(model, 'struct StandardTerminalEncoding;', 'pub(super) struct StandardTerminalEncoding;')
    (project/MODEL).write_text(model)

    # The generic configuration reader keeps PRAGMA, decode and validation order.
    config_signature = 'pub(super) fn read(connection: &Connection) -> Result<Option<MaintenanceConfig>, JournalError> {'
    text = original[CONFIG]
    start = text.index(config_signature)
    end = text.index('\npub(super) fn initialize(', start)
    body = text[start+len(config_signature):end]
    body = once(body, 'connection.query_row("PRAGMA user_version", [], |row| row.get(0))?',
        'effects.query_i64("PRAGMA user_version")?')
    body = once(body, '''connection.query_row(
        "SELECT config_json FROM controller_maintenance_config WHERE id=1",
        [],
        |row| row.get(0),
    )?''', '''effects.query_bytes(
        "SELECT config_json FROM controller_maintenance_config WHERE id=1",
    )?''')
    body = once(body, 'serde_json::from_slice(&bytes)?', 'effects.decode_maintenance(&bytes)?')
    declarations = '''pub(in crate::controller) trait ReadConfigEffects {
    fn query_i64(&mut self, statement: &str) -> Result<i64, JournalError>;
    fn query_bytes(&mut self, statement: &str) -> Result<Vec<u8>, JournalError>;
    fn decode_maintenance(&mut self, bytes: &[u8]) -> Result<MaintenanceConfig, JournalError>;
}

pub(in crate::controller) struct StandardConfigRead<'a> {
    pub(in crate::controller) connection: &'a Connection,
}

impl ReadConfigEffects for StandardConfigRead<'_> {
    fn query_i64(&mut self, statement: &str) -> Result<i64, JournalError> {
        Ok(self.connection.query_row(statement, [], |row| row.get(0))?)
    }
    fn query_bytes(&mut self, statement: &str) -> Result<Vec<u8>, JournalError> {
        Ok(self.connection.query_row(statement, [], |row| row.get(0))?)
    }
    fn decode_maintenance(&mut self, bytes: &[u8]) -> Result<MaintenanceConfig, JournalError> {
        Ok(serde_json::from_slice(bytes)?)
    }
}

pub(super) fn read(connection: &Connection) -> Result<Option<MaintenanceConfig>, JournalError> {
    read_using(&mut StandardConfigRead { connection })
}

pub(super) fn read_using<E: ReadConfigEffects>(
    effects: &mut E,
) -> Result<Option<MaintenanceConfig>, JournalError> {'''
    configuration = text[:start]+declarations+body+text[end:]
    configuration = once(configuration, '        for value in [', '        let values = [')
    configuration = once(configuration, '            self.prune_batch_rows,\n        ] {\n            if value == 0 || i64::try_from(value).is_err() || usize::try_from(value).is_err() {\n                return Err(JournalError::UnsupportedStore);\n            }\n        }', '            self.prune_batch_rows,\n        ];\n        let mut index = 0;\n        while index < values.len() {\n            let value = values[index];\n            if value == 0 || i64::try_from(value).is_err() || usize::try_from(value).is_err() {\n                return Err(JournalError::UnsupportedStore);\n            }\n            index += 1;\n        }')
    (project/CONFIG).write_text(configuration)
    checkpoint = once(original[CHECKPOINT], 'pub use config::MaintenanceConfig;',
        'pub use config::MaintenanceConfig;\npub(super) use config::{ReadConfigEffects, StandardConfigRead};')
    wrapper = '''pub(super) fn config(connection: &Connection) -> Result<Option<MaintenanceConfig>, JournalError> {
    config::read(connection)
}'''
    checkpoint = once(checkpoint, wrapper, wrapper+'''

pub(super) fn config_using<E: ReadConfigEffects>(
    effects: &mut E,
) -> Result<Option<MaintenanceConfig>, JournalError> {
    config::read_using(effects)
}''')
    (project/CHECKPOINT).write_text(checkpoint)
    capacity = once(original[CAPACITY], '''pub(super) fn config(connection: &Connection) -> Result<Option<CapacityConfig>, JournalError> {
    let policy = checkpoint::config(connection)?.ok_or(JournalError::UnsupportedStore)?;''',
        '''pub(super) fn config(connection: &Connection) -> Result<Option<CapacityConfig>, JournalError> {
    config_using(&mut checkpoint::StandardConfigRead { connection })
}

pub(super) fn config_using<E: checkpoint::ReadConfigEffects>(
    effects: &mut E,
) -> Result<Option<CapacityConfig>, JournalError> {
    let policy = checkpoint::config_using(effects)?.ok_or(JournalError::UnsupportedStore)?;''')
    capacity = once(capacity, """values
            .into_iter()
            .any(|n| n == 0 || i64::try_from(n).is_err() || usize::try_from(n).is_err())""",
        'invalid_capacity_values(values)')
    capacity += '\nfn invalid_capacity_values(values: [u64; 12]) -> bool {\n    let mut index = 0;\n    while index < values.len() {\n        let value = values[index];\n        if value == 0 || i64::try_from(value).is_err() || usize::try_from(value).is_err() {\n            return true;\n        }\n        index += 1;\n    }\n    false\n}\n'
    (project/CAPACITY).write_text(capacity)

    text = original[CODEC]
    signature = 'pub(in crate::controller) fn load(connection: &Connection) -> Result<Option<Inbox>, JournalError> {'
    start = text.index(signature)
    end = text.index('\npub(super) fn save(', start)
    body = text[start+len(signature):end]
    for old, new in [
        ('connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type=\'table\' AND name=\'controller_lifecycle_inbox\')",[],|r|r.get(0))?',
         'effects.query_bool("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type=\'table\' AND name=\'controller_lifecycle_inbox\')")?'),
        ('''connection.query_row("SELECT version FROM controller_meta WHERE id=1", [], |r| {
            r.get(0)
        })?''', 'effects.query_i64("SELECT version FROM controller_meta WHERE id=1")?'),
        ('''connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='controller_source_work')",
        [],
        |r| r.get(0),
    )?''', '''effects.query_bool(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='controller_source_work')",
    )?'''),
        ('connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type=\'table\' AND name=\'controller_internal_inbox\')",[],|r|r.get::<_,bool>(0))?',
         'effects.query_bool("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type=\'table\' AND name=\'controller_internal_inbox\')")?'),
        ('super::super::capacity::config(connection)?', 'super::super::capacity::config_using(effects)?'),
        ('''connection.query_row(
        "SELECT length(state_json),reserve_bytes FROM controller_lifecycle_inbox WHERE id=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?''', '''effects.query_i64_pair(
        "SELECT length(state_json),reserve_bytes FROM controller_lifecycle_inbox WHERE id=1",
    )?'''),
        ('''connection.query_row(
        "SELECT state_json,sha256 FROM controller_lifecycle_inbox WHERE id=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?''', '''effects.query_bytes_pair(
        "SELECT state_json,sha256 FROM controller_lifecycle_inbox WHERE id=1",
    )?'''),
        ('serde_json::from_slice(&bytes).map_err(|_| JournalError::CorruptStore)?',
         'effects.decode_inbox(&bytes).map_err(|_| JournalError::CorruptStore)?'),
        ('Sha256::digest(&bytes).as_slice() != sha', 'effects.digest(&bytes).as_slice() != &*sha'),
        ('serde_json::to_vec(&inbox)? != bytes', 'effects.encode_inbox(&inbox)? != bytes'),
        ('inbox.validate()?;', 'inbox.validate_using::<R, T, N, U, B>()?;'),
        ('super::super::checkpoint::config(connection)?', 'super::super::checkpoint::config_using(effects)?'),
    ]:
        body = once(body, old, new)
    declarations = '''pub(super) trait InboxLoadEffects: super::super::checkpoint::ReadConfigEffects {
    fn query_bool(&mut self, statement: &str) -> Result<bool, JournalError>;
    fn query_i64_pair(&mut self, statement: &str) -> Result<(i64, i64), JournalError>;
    fn query_bytes_pair(&mut self, statement: &str) -> Result<(Vec<u8>, Vec<u8>), JournalError>;
    fn decode_inbox(&mut self, bytes: &[u8]) -> Result<Inbox, JournalError>;
    fn encode_inbox(&mut self, inbox: &Inbox) -> Result<Vec<u8>, JournalError>;
    fn digest(&mut self, bytes: &[u8]) -> [u8; 32];
}

impl InboxLoadEffects for super::super::checkpoint::StandardConfigRead<'_> {
    fn query_bool(&mut self, statement: &str) -> Result<bool, JournalError> {
        Ok(self.connection.query_row(statement, [], |r| r.get(0))?)
    }
    fn query_i64_pair(&mut self, statement: &str) -> Result<(i64, i64), JournalError> {
        Ok(self.connection.query_row(statement, [], |r| Ok((r.get(0)?, r.get(1)?)))?)
    }
    fn query_bytes_pair(&mut self, statement: &str) -> Result<(Vec<u8>, Vec<u8>), JournalError> {
        Ok(self.connection.query_row(statement, [], |r| Ok((r.get(0)?, r.get(1)?)))?)
    }
    fn decode_inbox(&mut self, bytes: &[u8]) -> Result<Inbox, JournalError> {
        Ok(serde_json::from_slice(bytes)?)
    }
    fn encode_inbox(&mut self, inbox: &Inbox) -> Result<Vec<u8>, JournalError> {
        Ok(serde_json::to_vec(inbox)?)
    }
    fn digest(&mut self, bytes: &[u8]) -> [u8; 32] {
        Sha256::digest(bytes).into()
    }
}

pub(in crate::controller) fn load(connection: &Connection) -> Result<Option<Inbox>, JournalError> {
    load_using::<
        super::native_retirement::StandardRetirementEncoding,
        super::model::StandardTerminalEncoding,
        super::renewal_confirmation::StandardRenewalEncoding,
        super::unload_resolution::StandardUnloadEncoding,
        crate::capture::StandardGraphicBytes,
        _,
    >(&mut super::super::checkpoint::StandardConfigRead { connection })
}

pub(super) fn load_using<
    R: super::native_retirement::RetirementEncoding,
    T: super::model::TerminalEncoding,
    N: super::renewal_confirmation::RenewalEncoding,
    U: super::unload_resolution::UnloadEncoding,
    B: crate::capture::GraphicBytes,
    E: InboxLoadEffects,
>(effects: &mut E) -> Result<Option<Inbox>, JournalError> {'''
    (project/CODEC).write_text(text[:start]+declarations+body+text[end:])
    # Avoid unsupported library error carriers/default-method signatures, keeping
    # actual checks and their short-circuit positions in these private Rust bodies.
    for name in [CONFIG, CAPACITY]:
        p = project/name
        contents = p.read_text()
        contents = once(contents,
            'value == 0 || i64::try_from(value).is_err() || usize::try_from(value).is_err()',
            'value == 0 || value > i64::MAX as u64 || value > usize::MAX as u64')
        if name == CAPACITY:
            contents = once(contents, 'add(mul(3, policy.max_input_bytes)?, SCALAR_BYTES)?\n                    .max(2 * OPTIONS_BYTES + SCALAR_BYTES)', '{\n                    let computed = add(mul(3, policy.max_input_bytes)?, SCALAR_BYTES)?;\n                    let options = 2 * OPTIONS_BYTES + SCALAR_BYTES;\n                    if computed > options { computed } else { options }\n                }')
        p.write_text(contents)
    p = project/CODEC
    contents = p.read_text()
    for name in ['length', 'reserve']:
        contents = once(contents,
            f'let {name} = u64::try_from({name}).map_err(|_| JournalError::CorruptStore)?;',
            f'if {name} < 0 {{ return Err(JournalError::CorruptStore); }}\n    let {name} = {name}.cast_unsigned();')
    p.write_text(contents)
    subprocess.run(['rustfmt', '--edition', '2024', *original], cwd=project, check=True)
    changes.update({n:hashlib.sha256((project/n).read_bytes()).hexdigest() for n in original})
    metadata.update(load_candidate=True, load_business_body_from_original=True,
        configuration_business_body_from_original=True, standard_load_provider_adopted=False,
        original_equivalence_proved=False, opaque_business_functions=[],
        same_transaction_read_proved=False, production_modified=False)
    return changes, metadata
