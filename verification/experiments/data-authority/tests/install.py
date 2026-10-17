#!/usr/bin/env python3
"""Install identical test-only hooks and real application tests into scratch copies.

Run only in the approved Linux proof environment: SOURCE_ROOT RECEIPT_JSON.
No original integration fixture or production function body is replaced.
"""
from pathlib import Path
import hashlib
import json
import sys

source = Path(sys.argv[1]).resolve()
receipt = Path(sys.argv[2]) if len(sys.argv) > 2 else source.parent / (source.name + '-test-install.json')
assert not (source / '.git').exists(), 'Use an isolated source archive, not a checkout'
here = Path(__file__).resolve().parent
business_tests = here / 'test-drafts/business-tests.rs'
if not business_tests.exists():
    business_tests = here / 'business-tests.rs'
assert business_tests.is_file()
fixture_path = source / 'controller-service/tests/application_lifecycle.rs'
fixture = fixture_path.read_text()
assert fixture.count('mod support;') == 1
assert fixture.count('#[path = "application_lifecycle/contention.rs"]') == 1
fixture = fixture.replace('mod support;', '#[path = "data_authority_pilot_support.rs"]\nmod support;')
fixture = fixture.replace(
    '#[path = "application_lifecycle/contention.rs"]',
    '#[path = "../tests/application_lifecycle/contention.rs"]',
)

def replace_once(old: str, new: str) -> None:
    global fixture
    assert fixture.count(old) == 1, (old, fixture.count(old))
    fixture = fixture.replace(old, new)

replace_once(
    '    actions: Vec<WorkAction<core::EffectId>>,\n}',
    '    actions: Vec<WorkAction<core::EffectId>>,\n'
    '    dp_gate_fails: bool,\n'
    '    dp_gate_missing: bool,\n'
    '    dp_gate_calls: Vec<(core::SandboxId, core::VmId, core::Generation)>,\n'
    '    dp_relay_inputs: Vec<(core::SandboxId, core::VmId, core::Generation, bool)>,\n}',
)
replace_once(
    '    fn acquire_data_gate(\n        &self,\n        _: core::SandboxId,\n'
    '        _: core::VmId,\n        _: core::Generation,\n'
    '    ) -> Result<Option<std::fs::File>, ApplicationError> {\n',
    '    fn acquire_data_gate(\n        &self,\n        sandbox: core::SandboxId,\n'
    '        vm: core::VmId,\n        generation: core::Generation,\n'
    '    ) -> Result<Option<std::fs::File>, ApplicationError> {\n'
    '        {\n            let mut state = self.state.lock().unwrap();\n'
    '            state.dp_gate_calls.push((sandbox, vm, generation));\n'
    '            if state.dp_gate_fails { return Err(ApplicationError::Busy); }\n'
    '            if state.dp_gate_missing { return Ok(None); }\n        }\n',
)
replace_once(
    '    fn spawn_data_relay(\n        &mut self,\n        _: core::SandboxId,\n'
    '        _: core::VmId,\n        _: core::Generation,\n        _: bool,\n'
    '    ) -> Result<std::process::Child, ApplicationError> {\n',
    '    fn spawn_data_relay(\n        &mut self,\n        sandbox: core::SandboxId,\n'
    '        vm: core::VmId,\n        generation: core::Generation,\n        control: bool,\n'
    '    ) -> Result<std::process::Child, ApplicationError> {\n'
    '        self.state.lock().unwrap().dp_relay_inputs.push((sandbox, vm, generation, control));\n',
)
test_path = source / 'controller-service/src/data_authority_pilot_tests.rs'
assert not test_path.exists()
test_body = fixture + '\n' + business_tests.read_text()
support_path = source / 'controller-service/src/data_authority_pilot_support.rs'
assert not support_path.exists()
support_original = (source / 'controller-service/tests/support/mod.rs').read_text()
assert support_original.count('pub mod complete_fixture;') == 1
support_body = support_original.replace(
    'pub mod complete_fixture;',
    'pub(crate) use crate::controller::tests_support::fixture as complete_fixture;',
)

changes = {}
def append(relative: str, sentinel: str, addition: str) -> None:
    path = source / relative
    before = path.read_text()
    current = changes.get(relative, (before, before))[1]
    assert sentinel not in current
    after = current + '\n' + addition
    changes[relative] = (before, after)

append(
    'controller-service/src/lib.rs', 'mod data_authority_pilot_tests;',
    '#[cfg(all(test, feature = "durable"))]\nmod data_authority_pilot_tests;\n',
)
assert '#[cfg(test)]\nextern crate self as sandproof_controller_service;' in (
    source / 'controller-service/src/lib.rs').read_text()
append(
    'controller-service/src/controller/mod.rs', 'fn data_authority_pilot_fail_next_commit',
    '#[cfg(test)]\npub(crate) fn data_authority_pilot_fail_next_commit(durable: bool) {\n'
    '    transaction::fail_next_commit(durable);\n}\n',
)
append(
    'controller-service/src/controller/mod.rs', 'fn data_authority_pilot_close_effective',
    '#[cfg(test)]\nimpl ControllerJournal {\n'
    '    pub(crate) fn data_authority_pilot_close_effective(&mut self, sandbox: core::SandboxId) -> core::World {\n'
    '        let before = self.world().unwrap();\n'
    '        let mut expected = before.sandbox_record(sandbox).unwrap().clone();\n'
    '        expected.effective = core::Authority::Closed;\n'
    '        let injected = core::data_authority_pilot::close_effective(&before, sandbox);\n'
    '        assert_eq!(injected.sandbox_record(sandbox), Some(&expected));\n'
    '        self.committed_world = transaction::CommittedWorld::new(&self.journal.connection, injected.clone()).unwrap();\n'
    '        injected\n    }\n}\n',
)
controller_relative = 'controller-service/src/controller/mod.rs'
before, after = changes[controller_relative]
assert after.count('#[cfg(test)]\nmod tests_support;') == 1
changes[controller_relative] = (
    before,
    after.replace('#[cfg(test)]\nmod tests_support;', '#[cfg(test)]\npub(crate) mod tests_support;'),
)
fixture_relative = 'controller-service/src/controller/tests_support.rs'
before = (source / fixture_relative).read_text()
assert before.count('pub(super) mod fixture;') == 1
changes[fixture_relative] = (before, before.replace('pub(super) mod fixture;', 'pub(crate) mod fixture;'))
append(
    'controller-service/src/application/coordinator.rs',
    'fn data_authority_pilot_registration_snapshot',
    '#[cfg(test)]\nimpl<H: ControllerHost> ControllerApplication<H> {\n'
    '    pub(crate) fn data_authority_pilot_registration_snapshot(&self) -> (Vec<(usize, bool)>, usize) {\n'
    '        let inner = self.inner.lock().unwrap();\n'
    '        let signals = inner.data_revocations.values().flatten().map(|sender| {\n'
    '            (sender.receiver_count(), *sender.borrow())\n'
    '        }).collect();\n'
    '        (signals, inner.data_drains.len())\n    }\n}\n',
)
append(
    'controller-service/src/application/coordinator.rs', 'fn data_authority_pilot_close_effective',
    '#[cfg(test)]\nimpl<H: ControllerHost> ControllerApplication<H> {\n'
    '    pub(crate) fn data_authority_pilot_close_effective(&self, sandbox: core::SandboxId) -> core::World {\n'
    '        self.inner.lock().unwrap().journal.data_authority_pilot_close_effective(sandbox)\n'
    '    }\n}\n',
)
append(
    'controller-service/src/application/coordinator.rs', 'fn data_authority_pilot_sandbox',
    '#[cfg(test)]\nimpl DataPermit {\n'
    '    pub(crate) fn data_authority_pilot_sandbox(&self) -> core::SandboxId { self.sandbox }\n'
    '}\n',
)
append(
    'controller-core/src/lib.rs', 'pub mod data_authority_pilot;',
    '#[cfg(feature = "data-authority-pilot")]\npub mod data_authority_pilot;\n',
)
core_manifest = 'controller-core/Cargo.toml'
before = (source / core_manifest).read_text()
assert before.count('[features]\n') == 1
changes[core_manifest] = (before, before.replace('[features]\n', '[features]\ndata-authority-pilot = []\n'))
fault_path = source / 'controller-core/src/data_authority_pilot.rs'
assert not fault_path.exists()
fault_body = '''//! Fault injection for isolated application tests; absent from default builds.
use crate::{Authority, SandboxId, World};

/// Return a copy with only the selected record's effective authority closed.
#[must_use]
pub fn close_effective(world: &World, sandbox: SandboxId) -> World {
    let mut next = world.clone();
    if let Some(record) = world.sandbox_record(sandbox) {
        let mut updated = record.clone();
        updated.effective = Authority::Closed;
        next.sandboxes.put(sandbox, updated);
    }
    next
}
'''
sha = lambda value: hashlib.sha256(value).hexdigest()
for relative, (_, after) in changes.items():
    (source / relative).write_text(after)
test_path.write_text(test_body)
support_path.write_text(support_body)
fault_path.write_text(fault_body)
receipt.parent.mkdir(parents=True, exist_ok=True)
receipt.write_text(json.dumps({
    'purpose': 'identical cfg(test) bridge, observer and real ControllerApplication fixture',
    'source_commit': '0a23b57af60752f61067fc00b0b0b656e98347cb',
    'fixture_sha256': sha(fixture_path.read_bytes()),
    'fixture_original_unchanged': True,
    'changed_files': {relative: {
        'before_sha256': sha(before.encode()), 'after_install_sha256': sha(after.encode())
    } for relative, (before, after) in changes.items()},
    'new_test_file': str(test_path.relative_to(source)),
    'new_test_sha256': sha(test_path.read_bytes()),
    'shared_fixture': 'Reuse the original cfg(test) complete_fixture module; no duplicate load',
    'support_original_sha256': sha(support_original.encode()),
    'new_support_file': str(support_path.relative_to(source)),
    'new_support_sha256': sha(support_path.read_bytes()),
    'fault_injection_feature': 'sandbox-controller-core/data-authority-pilot, test command only; disabled for extraction',
    'new_fault_file': str(fault_path.relative_to(source)),
    'new_fault_sha256': sha(fault_path.read_bytes()),
    'formatting': 'subsequent rustfmt hashes must be frozen separately',
}, indent=2) + '\n')
print(receipt.read_text())
