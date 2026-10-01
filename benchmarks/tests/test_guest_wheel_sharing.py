"""Guard wheel provenance, unchanged payloads, and real hard-link semantics."""
import copy
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from benchmarks import guest_wheel_sharing as sharing


class WheelSharingTests(unittest.TestCase):
    def report(self):
        return {'install': [{'metadata': {'name': 'Torch', 'version': '2.7.1'},
            'download_info': {'url': 'https://files.pythonhosted.org/packages/torch.whl',
                'archive_info': {'hashes': {'sha256': 'a' * 64}}}}]}

    def test_requirement_pins_exact_public_wheel(self):
        self.assertEqual(sharing.wheel_requirement(self.report(), 'torch', '2.7.1'),
                         'torch==2.7.1 --hash=sha256:' + 'a' * 64 + '\n')

    def test_reject_changed_resolution_untrusted_url_and_missing_hash(self):
        bad = []
        for field, value in (('name', 'other'), ('version', '2.8.0')):
            report = self.report()
            report['install'][0]['metadata'][field] = value
            bad.append(report)
        for url in ('http://files.pythonhosted.org/a.whl', 'https://example.invalid/a.whl',
                    'https://user@files.pythonhosted.org/a.whl',
                    'https://files.pythonhosted.org:444/a.whl',
                    'https://files.pythonhosted.org/a.tar.gz'):
            report = self.report()
            report['install'][0]['download_info']['url'] = url
            bad.append(report)
        report = self.report()
        report['install'].append(copy.deepcopy(report['install'][0]))
        bad.append(report)
        report = self.report()
        report['install'][0]['download_info']['archive_info']['hashes']['sha256'] = 'bad'
        bad.append(report)
        for report in bad:
            with self.subTest(report=report), self.assertRaises(ValueError):
                sharing.wheel_requirement(report, 'torch', '2.7.1')

    def test_relink_requires_identical_payload_version_and_cache_hardlinks(self):
        for outcome in ('hardlink', 'copy', 'changed', 'version', 'installer_failure'):
            with self.subTest(outcome=outcome), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                payload = root / 'lib.so'
                payload.write_bytes(b'original binary payload')
                dist = SimpleNamespace(version='2.7.1', files=['lib.so'],
                                       locate_file=lambda relative: root / relative)

                def install(command, **kwargs):
                    self.assertIn('--require-hashes', command)
                    self.assertEqual(command[command.index('--link-mode') + 1], 'hardlink')
                    self.assertEqual(command[command.index('--reinstall-package') + 1], 'torch')
                    self.assertIn('--no-deps', command)
                    if outcome == 'installer_failure':
                        raise subprocess.CalledProcessError(42, command)
                    if outcome == 'changed':
                        payload.write_bytes(b'changed binary payload')
                    if outcome == 'version':
                        dist.version = '2.8.0'
                    if outcome != 'copy':
                        os.link(payload, root / 'cache.so')

                with (patch.object(sharing.metadata, 'distribution', return_value=dist),
                      patch.object(sharing, 'LARGE_FILE_BYTES', 1),
                      patch.object(sharing.subprocess, 'run', side_effect=install)):
                    if outcome == 'hardlink':
                        result = sharing.relink('torch', '2.7.1', root / 'pins', root / 'uv')
                        self.assertTrue(result['payload_unchanged'])
                        self.assertEqual(result['large_files'][0]['links'], 2)
                        self.assertEqual(result['large_files'][0]['sha256'],
                                         hashlib.sha256(payload.read_bytes()).hexdigest())
                    else:
                        error = subprocess.CalledProcessError if outcome == 'installer_failure' else ValueError
                        with self.assertRaises(error):
                            sharing.relink('torch', '2.7.1', root / 'pins', root / 'uv')

    def test_snapshot_rejects_missing_absolute_or_symlink_payload(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'data').write_bytes(b'data')
            (root / 'link').symlink_to(root / 'data')
            for relative in ('missing', '/absolute', 'link'):
                dist = SimpleNamespace(files=[relative], locate_file=lambda p: root / p)
                with self.subTest(relative=relative), self.assertRaises(ValueError):
                    sharing.snapshot(dist)


if __name__ == '__main__':
    unittest.main()
