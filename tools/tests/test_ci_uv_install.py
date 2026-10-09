import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest
import zipfile


ROOT = Path(__file__).resolve().parents[2]


class CiUvInstallTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.stage = Path(self.temp.name)
        self.pins = json.loads((ROOT / 'tools/ci_image_pins.json').read_text())
        self.env = os.environ | {'PATH': str(self.stage) + os.pathsep + os.environ['PATH'],
                                 'CI_UV_TEST_STAGE': str(self.stage)}
        self.shim('id', 'print(0)\n')
        self.shim('curl', '''
import shutil, sys
stage = Path(os.environ['CI_UV_TEST_STAGE'])
(stage / 'downloaded').touch()
shutil.copyfile(stage / 'wheel.whl', sys.argv[sys.argv.index('-o') + 1])
''')
        # No test may mutate /usr/local/bin, even when run by root in a CI guest.
        self.shim('install', "(Path(os.environ['CI_UV_TEST_STAGE']) / 'installed').touch()\nraise SystemExit(93)\n")

    def shim(self, name, code):
        script = self.stage / name
        script.write_text('#!' + sys.executable + '\nfrom pathlib import Path\nimport os\n' +
                          textwrap.dedent(code))
        script.chmod(0o700)

    def wheel(self, version='uv 9.9.9', member='uv-0.11.31.data/scripts/uv'):
        archive = self.stage / 'wheel.whl'
        with zipfile.ZipFile(archive, 'w') as wheel:
            wheel.writestr(member, '#!/bin/sh\nprintf "%s\\n" "' + version + '"\n')
        self.pins['uv_sha256'] = hashlib.sha256(archive.read_bytes()).hexdigest()

    def invoke(self):
        pins = self.stage / 'pins.json'
        pins.write_text(json.dumps(self.pins))
        return subprocess.run(['/bin/bash', str(ROOT / 'tools/ci_install_uv.sh'), str(pins)],
                              env=self.env, capture_output=True, text=True, timeout=10)

    def test_wrong_release_or_untrusted_artifact_is_rejected_before_download(self):
        for field, value in (('uv_version', '0.11.32'), ('uv_sha256', 'not-a-sha'),
                             ('uv_url', self.pins['uv_url'].replace('https:', 'http:')),
                             ('uv_url', self.pins['uv_url'].replace('files.pythonhosted.org', 'example.com')),
                             ('uv_url', self.pins['uv_url'] + '?other=source')):
            with self.subTest(field=field, value=value):
                original = self.pins[field]
                self.pins[field] = value
                result = self.invoke()
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse((self.stage / 'downloaded').exists())
                self.assertFalse((self.stage / 'installed').exists())
                self.pins[field] = original

    def test_checksum_failure_prevents_archive_execution_or_installation(self):
        self.wheel()
        self.pins['uv_sha256'] = '0' * 64
        result = self.invoke()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue((self.stage / 'downloaded').exists())
        self.assertIn('FAILED', result.stdout)
        self.assertFalse((self.stage / 'installed').exists())

    def test_verified_artifact_with_wrong_version_is_rejected_before_installation(self):
        self.wheel()
        result = self.invoke()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('unexpected version', result.stderr)
        self.assertFalse((self.stage / 'installed').exists())

    def test_verified_release_accepts_official_build_details_before_installation(self):
        self.wheel('uv 0.11.31 (b7fdec626 2026-07-21 x86_64-unknown-linux-gnu)')
        result = self.invoke()
        # The installation shim deliberately blocks host mutation after the gate.
        self.assertEqual(result.returncode, 93, result.stdout + result.stderr)
        self.assertTrue((self.stage / 'installed').exists())

    def test_verified_archive_cannot_extract_other_paths(self):
        self.wheel('uv 0.11.31', '../not-the-uv-binary')
        result = self.invoke()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.stage / 'installed').exists())
        self.assertFalse((self.stage.parent / 'not-the-uv-binary').exists())


class CiUvWorkflowTests(unittest.TestCase):
    def test_both_jobs_use_preinstalled_version_gate_and_original_locked_entry(self):
        workflow = (ROOT / '.github/workflows/function-ci.yml').read_text()
        blocks = re.findall(r'        run: \|\n((?:          [^\n]*\n)+)', workflow)
        self.assertEqual(len(blocks), 2)
        self.assertNotIn('uv-bootstrap', workflow)
        self.assertNotIn('pip install', workflow)
        cases = (('uv 0.11.31', 0),
                 ('uv 0.11.31 (b7fdec626 2026-07-21 x86_64-unknown-linux-gnu)', 0),
                 ('uv 0.11.30', 0), ('uv 0.11.310', 0), ('', 127))
        for block, script, lock in zip(blocks, ('check-compat.sh', 'check-bench.sh'),
                                       ('tests/compat/uv.lock', 'benchmarks/uv.lock')):
            for version, uv_exit in cases:
                with self.subTest(script=script, version=version), tempfile.TemporaryDirectory() as temp:
                    stage = Path(temp)
                    uv = stage / 'uv'
                    uv.write_text('#!/bin/sh\nprintf "%s\\n" "' + version +
                                  '"\nexit ' + str(uv_exit) + '\n')
                    uv.chmod(0o700)
                    bash = stage / 'bash'
                    bash.write_text('#!' + sys.executable + '''
import json, os, sys
from pathlib import Path
Path(os.environ['CI_UV_TEST_STAGE'], 'entry.json').write_text(json.dumps({
    'arguments': sys.argv[1:], 'lock': os.environ.get('CI_GIT_LOCK')}))
''')
                    bash.chmod(0o700)
                    env = os.environ | {'PATH': temp + os.pathsep + os.environ['PATH'],
                                        'RUNNER_TEMP': temp, 'GITHUB_WORKSPACE': str(ROOT),
                                        'CI_UV_TEST_STAGE': temp}
                    result = subprocess.run(['/bin/bash', '-e', '-o', 'pipefail', '-c',
                                             textwrap.dedent(block)], env=env,
                                            capture_output=True, text=True, timeout=10)
                    entry = stage / 'entry.json'
                    if version in ('uv 0.11.31',
                                   'uv 0.11.31 (b7fdec626 2026-07-21 x86_64-unknown-linux-gnu)'):
                        self.assertEqual(result.returncode, 0, result.stderr)
                        self.assertEqual(json.loads(entry.read_text()), {
                            'arguments': ['scripts/' + script], 'lock': str(ROOT / lock)})
                        self.assertEqual((stage / 'ci-git/git').resolve(), ROOT / 'tools/ci_git.py')
                    else:
                        self.assertNotEqual(result.returncode, 0)
                        self.assertFalse(entry.exists())
                        self.assertFalse((stage / 'ci-git').exists())


if __name__ == '__main__':
    unittest.main()
