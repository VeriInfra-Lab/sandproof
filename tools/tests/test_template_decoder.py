"""Checked optional decoder selection uses real, isolated child processes."""
import hashlib
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

from tools.template_inputs import InvalidTemplate, validate_tool_names
from tools.template_worker import COMMAND_ENV, Commands, buildkit_environment, check_tools


class DecoderTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.profile = {}
        for name in ('buildctl', 'buildkitd', 'buildkit-runc', 'umoci', 'envd', 'busybox',
                     'igzip', 'libisal.so.2'):
            path = self.root / ('versioned-' + name)
            if name == 'busybox':
                value = ('#!' + sys.executable + '\nimport sys\n'
                         'if sys.argv[1:] == ["--list"]:\n'
                         '    print("sh init mount grep mkdir chmod ln ip touch reboot umount rm stat".replace(" ", "\\n"))\n'
                         'elif sys.argv[1:] == ["stat", "-f", "-c", "%t", "/"]:\n'
                         '    print("ef53")\n'
                         'else:\n'
                         '    sys.exit(73)\n').encode()
            elif name == 'igzip':
                value = ('#!' + sys.executable + '\nimport json,os,sys\n'
                         'print(json.dumps({k:os.environ.get(k) for k in '
                         '("LD_LIBRARY_PATH","CONTAINERD_DISABLE_IGZIP","CONTAINERD_DISABLE_PIGZ")}))\n'
                         'sys.exit(17 if "--fail" in sys.argv else 0)\n').encode()
            else:
                value = name.encode()
            path.write_bytes(value)
            path.chmod(0o755)
            self.profile[name] = {'path': str(path), 'sha256': hashlib.sha256(value).hexdigest()}

    def test_optional_pair_is_atomic_and_unknown_tools_are_rejected(self):
        base = {k: v for k, v in self.profile.items() if k not in ('igzip', 'libisal.so.2')}
        for registry in (False, True):
            required = base | ({'regctl': self.profile['buildctl']} if registry else {})
            for extra in ({}, {k: self.profile[k] for k in ('igzip', 'libisal.so.2')}):
                validate_tool_names(required | extra, registry=registry)
            for extra in ({'igzip': {}}, {'libisal.so.2': {}}, {'../igzip': {}}, {'unpigz': {}}):
                with self.subTest(registry=registry, extra=extra), self.assertRaises(InvalidTemplate):
                    check_tools(required | extra, registry=registry)

    def test_both_decoder_fingerprints_and_regular_files_are_required(self):
        for name in ('igzip', 'libisal.so.2'):
            path = Path(self.profile[name]['path'])
            original = path.read_bytes()
            path.write_bytes(original + b'changed')
            with self.subTest(name=name), self.assertRaisesRegex(InvalidTemplate, 'fingerprint'):
                check_tools(self.profile)
            path.write_bytes(original)
            renamed = path.with_suffix('.saved')
            path.rename(renamed)
            path.symlink_to(renamed)
            with self.subTest(name=name, link=True), self.assertRaisesRegex(InvalidTemplate, 'fingerprint'):
                check_tools(self.profile)
            path.unlink()
            renamed.rename(path)

    def test_only_checked_versioned_artifacts_are_selected_and_failure_propagates(self):
        tools = check_tools(self.profile)
        env = buildkit_environment(tools, self.root)
        directory = self.root / 'gzip-tools'
        for name in ('igzip', 'libisal.so.2'):
            self.assertTrue(os.path.samefile(directory / name, tools[name]))
        self.assertEqual(directory.stat().st_mode & 0o777, 0o700)
        for fail in (False, True):
            log = io.BytesIO()
            runner = Commands(time.monotonic() + 5, log, 4096)
            args = [sys.executable, '-c', 'import subprocess; subprocess.run(' +
                    repr(['igzip'] + (['--fail'] if fail else [])) + ', check=True)']
            with patch.dict(os.environ, {'LD_LIBRARY_PATH': '/untrusted', 'CONTAINERD_DISABLE_IGZIP': '1'}):
                if fail:
                    with self.assertRaisesRegex(RuntimeError, 'tool failed'):
                        runner.run(args, env=env)
                else:
                    runner.run(args, env=env)
            first = json.loads(log.getvalue().splitlines()[0])
            self.assertEqual(first, {'LD_LIBRARY_PATH': str(directory),
                                    'CONTAINERD_DISABLE_IGZIP': '0', 'CONTAINERD_DISABLE_PIGZ': '1'})

    def test_default_disables_ambient_decoders_and_other_commands_keep_clean_environment(self):
        self.assertEqual(buildkit_environment({}, self.root),
                         dict(COMMAND_ENV, CONTAINERD_DISABLE_IGZIP='1', CONTAINERD_DISABLE_PIGZ='1'))
        self.assertFalse((self.root / 'gzip-tools').exists())
        log = io.BytesIO()
        with patch.dict(os.environ, {'LD_LIBRARY_PATH': '/untrusted', 'CONTAINERD_DISABLE_IGZIP': '0'}):
            Commands(time.monotonic() + 5, log, 4096).run([sys.executable, '-c',
                'import os; print(os.getenv("LD_LIBRARY_PATH")); print(os.getenv("CONTAINERD_DISABLE_IGZIP"))'])
        self.assertEqual(log.getvalue(), b'None\nNone\n')


if __name__ == '__main__':
    unittest.main()
