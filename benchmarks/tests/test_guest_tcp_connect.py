import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from benchmarks.guest_tcp_connect import configure


class GuestTcpConnectTests(unittest.TestCase):
    def test_records_actual_before_and_after_values_in_a_new_log_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            setting, evidence = root / 'setting', root / 'logs/policy.json'
            setting.write_text('6\n')
            result = configure(setting, evidence)
            self.assertEqual(setting.read_text(), '2\n')
            self.assertEqual(result['tcp_syn_retries_before'], 6)
            self.assertEqual(result['tcp_syn_retries'], 2)
            self.assertEqual(json.loads(evidence.read_text()), result)
            with self.assertRaisesRegex(RuntimeError, 'already exists'):
                configure(setting, evidence)
            self.assertEqual(json.loads(evidence.read_text()), result)

    def test_failed_write_or_mismatched_readback_cannot_create_success_evidence(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            setting, evidence = root / 'setting', root / 'policy.json'
            setting.write_text('6\n')
            with patch.object(Path, 'write_text', side_effect=PermissionError('read-only sysctl')):
                with self.assertRaises(PermissionError):
                    configure(setting, evidence)
            self.assertFalse(evidence.exists())
            with patch.object(Path, 'read_text', side_effect=['6\n', '3\n']):
                with self.assertRaisesRegex(RuntimeError, 'did not take effect'):
                    configure(setting, evidence)
            self.assertFalse(evidence.exists())


if __name__ == '__main__':
    unittest.main()
