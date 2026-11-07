"""Admission regressions for this batch's finite source binding, not proofs."""
from pathlib import Path
from tempfile import TemporaryDirectory
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import capture


CALL = ('crate::lifecycle::decode_accepted_with(&body.to_bytes(), &context, |input| {\n'
        '        serde_json::from_slice::<AcceptedLifecycleIntent>(input)\n    })')


class FiniteParserSources(unittest.TestCase):
    def setUp(self):
        self.temp = TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.fixed = {'sandbox-protocol/src/lib.rs': b'fixed protocol',
                      'sandbox-internal/src/lifecycle.rs': b'fixed helper',
                      'Cargo.lock': b'fixed dependencies'}
        for name, value in self.fixed.items():
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(value)
        self.server = self.root / 'sandbox-internal/src/server/lifecycle.rs'
        self.server.parent.mkdir(parents=True)
        self.server.write_text(CALL)
        self.addCleanup(patch.stopall)
        patch.object(capture, 'ROOT', self.root).start()
        patch.object(capture.subprocess, 'run', return_value=SimpleNamespace(returncode=0)).start()
        patch.object(capture.subprocess, 'check_output', side_effect=self.git).start()

    def git(self, argv, **kwargs):
        if argv[1] == 'rev-parse':
            return 'current-head\n'
        if argv[1] == 'ls-tree':
            return ('\0'.join(self.fixed) + '\0').encode()
        if argv[1] == 'show':
            return self.fixed[argv[2].split(':', 1)[1]]
        self.fail('unexpected source query: ' + repr(argv))

    def test_unrelated_current_component_does_not_replace_parser_inputs(self):
        (self.root / 'other-v3-component.rs').write_text('adopted elsewhere')
        self.assertEqual(capture.guard_baseline(), 'current-head')

    def test_changed_protocol_cannot_reuse_source_evidence(self):
        (self.root / 'sandbox-protocol/src/lib.rs').write_bytes(b'different protocol')
        with self.assertRaisesRegex(ValueError, 'reviewed parser source changed'):
            capture.guard_baseline()

    def test_replaced_context_is_rejected(self):
        self.server.write_text(CALL.replace('&context', '&other_context'))
        with self.assertRaisesRegex(ValueError, 'original production parser call'):
            capture.guard_baseline()

    def test_full_reply_adoption_does_not_change_original_parser_call(self):
        self.server.write_text(CALL + '\n// unrelated full record reply branch\n')
        self.assertEqual(capture.guard_baseline(), 'current-head')


if __name__ == '__main__':
    unittest.main()
