"""Reject diagnostic omissions and business changes even after candidate reseal."""
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from tools.check_redis_diagnostics_binding import STORE, DIAGNOSTICS, IO, check

from tools.tests.proof_input_fixture import reference_root

ROOT = reference_root()


class RedisDiagnosticBindingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name) / 'original'
        self.candidate = Path(self.temporary.name) / 'candidate'
        for root in (self.base, self.candidate):
            for name in (STORE, DIAGNOSTICS):
                p = root / name
                p.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / name, p)
        name = 'verification/experiments/redis-result/candidate.json'
        p = self.base / name
        p.parent.mkdir(parents=True)
        shutil.copyfile(ROOT / name, p)
        patch = ROOT / 'verification/experiments/redis-result/candidate.patch'
        subprocess.run(['git', 'apply', str(patch)], cwd=self.candidate, check=True, capture_output=True)

    def replace(self, name, old, new):
        p = self.candidate / name
        text = p.read_text()
        self.assertIn(old, text)
        p.write_text(text.replace(old, new, 1))

    def test_actual_prepared_candidate(self):
        self.assertGreaterEqual(len(check(self.base, self.candidate)['checks']), 25)

    def test_output_cannot_be_silently_removed(self):
        self.replace(IO, 'diagnostics::emit(phase, failure, elapsed, budget);', 'let _ = (phase, failure, elapsed, budget);')
        with self.assertRaisesRegex(ValueError, 'one original output call'):
            check(self.base, self.candidate)

    def test_output_keeps_original_time_arguments(self):
        self.replace(IO, 'diagnostics::emit(phase, failure, elapsed, budget);', 'diagnostics::emit(phase, failure, budget, elapsed);')
        with self.assertRaisesRegex(ValueError, 'one original output call'):
            check(self.base, self.candidate)

    def test_endpoint_cannot_acquire_business_state(self):
        self.replace(IO, 'pub(super) struct Diagnostics;', 'pub(super) struct Diagnostics { connection: Option<Connection> }')
        with self.assertRaisesRegex(ValueError, 'zero-resource endpoint'):
            check(self.base, self.candidate)

    def test_original_logging_implementation_is_retained(self):
        self.replace(DIAGNOSTICS, 'const LIMIT: usize = 64;', 'const LIMIT: usize = 65;')
        with self.assertRaisesRegex(ValueError, 'original diagnostic implementation'):
            check(self.base, self.candidate)

    def test_connection_must_be_discarded_before_logging(self):
        self.replace(IO, '*connection = None;\n        diagnostics::emit(phase, failure, started.elapsed(), remaining);', 'diagnostics::emit(phase, failure, started.elapsed(), remaining);\n        *connection = None;')
        with self.assertRaisesRegex(ValueError, 'complete moved query body'):
            check(self.base, self.candidate)

    def test_unknown_must_keep_original_fence(self):
        self.replace(STORE, 'self.write_fenced = true;', 'self.write_fenced = false;')
        with self.assertRaisesRegex(ValueError, 'original unknown fence'):
            check(self.base, self.candidate)

    def test_cas_keeps_original_phase(self):
        self.replace(IO, '        Phase::Cas,', '        Phase::Read,')
        with self.assertRaisesRegex(ValueError, 'CAS diagnostic phase'):
            check(self.base, self.candidate)

    def test_configuration_rejection_cannot_omit_logging(self):
        self.replace(STORE, 'return Err(configuration_error(\n                &mut self.diagnostics,\n                Phase::ServerHealth,\n            ));', 'return Err(StoreError::Configuration);')
        with self.assertRaisesRegex(ValueError, 'original server rejection'):
            check(self.base, self.candidate)

    def test_configuration_rejection_order_is_not_interchangeable(self):
        p = self.candidate / STORE
        s = p.read_text()
        first = s.index('        if !setting_matches(&settings, "appendonly"')
        second = s.index('        if !setting_matches(&settings, "appendfsync"', first)
        third = s.index('        if !setting_matches(&settings, "maxmemory-policy"', second)
        p.write_text(s[:first] + s[second:third] + s[first:second] + s[third:])
        with self.assertRaisesRegex(ValueError, 'configuration rejection order'):
            check(self.base, self.candidate)

    def test_persistence_rejection_cannot_omit_logging(self):
        self.replace(STORE, 'return Err(persistence_error(&mut self.diagnostics));', 'return Err(StoreError::Unavailable);')
        with self.assertRaisesRegex(ValueError, 'first persistence rejection'):
            check(self.base, self.candidate)

    def test_output_cannot_add_a_resource_destructor(self):
        p = self.candidate / IO
        p.write_text(p.read_text() + '\nimpl Drop for Diagnostics { fn drop(&mut self) {} }\n')
        with self.assertRaisesRegex(ValueError, 'no endpoint resource destructor'):
            check(self.base, self.candidate)


if __name__ == '__main__':
    unittest.main()
