"""Exercise guest-only apt setup with isolated paths and recording commands."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from benchmarks.oracle_repairs import prepare_repaired_task

HELPER = Path(__file__).parents[1] / 'guest_bullseye_snapshot.sh'


class BullseyeSnapshotTests(unittest.TestCase):
    def test_snapshot_guards_signatures_and_index_failure(self):
        for failure in ('', 'distribution', 'codename', 'keyring', 'iso', 'disk',
                        'extra_list', 'extra_sources', 'update'):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                for directory in ('etc/apt/sources.list.d', 'usr/share/keyrings', 'app', 'bin'):
                    (root / directory).mkdir(parents=True)
                original = 'deb https://deb.debian.org/debian bullseye main\n'
                sources = root / 'etc/apt/sources.list'
                sources.write_text(original)
                (root / 'etc/os-release').write_text(
                    f'ID={"ubuntu" if failure == "distribution" else "debian"}\n'
                    f'VERSION_CODENAME={"bookworm" if failure == "codename" else "bullseye"}\n')
                if failure != 'keyring':
                    (root / 'usr/share/keyrings/debian-archive-keyring.gpg').write_bytes(b'fixture keyring')
                for name, missing in (('alpine.iso', 'iso'), ('alpine-disk.qcow2', 'disk')):
                    if failure != missing:
                        (root / 'app' / name).touch()
                if failure in ('extra_list', 'extra_sources'):
                    (root / 'etc/apt/sources.list.d' / ('extra.' + failure.split('_')[1])).touch()
                apt = root / 'bin/apt-get'
                apt.write_text('#!/bin/bash\nprintf "%s\\n" "$*" > "$REPAIR_TEST_LOG"\n'
                               + ('exit 42\n' if failure == 'update' else 'exit 0\n'))
                apt.chmod(0o700)
                # Relocate only absolute filesystem paths in the real helper;
                # apt is a recording stub, so no host repository is accessed.
                script = HELPER.read_text()
                for prefix in ('/etc/', '/usr/share/keyrings/', '/app/', '/logs/'):
                    script = script.replace(prefix, str(root) + prefix)
                helper = root / 'helper.sh'
                helper.write_text(script)
                log = root / 'commands'
                result = subprocess.run(['/bin/bash', str(helper)], capture_output=True, env={
                    **os.environ, 'PATH': str(root / 'bin') + ':/usr/bin:/bin', 'REPAIR_TEST_LOG': str(log)})
                self.assertEqual(result.returncode, 42 if failure == 'update' else 1 if failure else 0)
                if failure not in ('', 'update'):
                    self.assertFalse(log.exists())
                    self.assertEqual(sources.read_text(), original)
                else:
                    self.assertEqual(log.read_text(), '-o APT::Update::Error-Mode=any update\n')
                    self.assertEqual((root / 'logs/verifier/qemu-original-sources.list').read_text(), original)
                    lines = sources.read_text().splitlines()
                    self.assertEqual(len(lines), 2)
                    for line, archive, suite in zip(lines, ('debian', 'debian-security'), ('bullseye', 'bullseye-security')):
                        self.assertEqual(line, 'deb [check-valid-until=no signed-by='
                            + str(root / 'usr/share/keyrings/debian-archive-keyring.gpg') + '] '
                            + f'https://snapshot.debian.org/archive/{archive}/20260824T000000Z {suite} main')

    def test_both_qemu_repairs_preserve_task_and_stop_on_setup_failure(self):
        script = 'printf "original QEMU solution\\n" >> "$REPAIR_TEST_LOG"\n'
        for name in ('qemu-alpine-ssh', 'qemu-startup'):
            for code in (0, 42):
                with self.subTest(name=name, code=code), tempfile.TemporaryDirectory() as tmp:
                    root = Path(tmp)
                    source, bin_dir = root / 'source', root / 'bin'
                    (source / 'solution').mkdir(parents=True)
                    (source / 'tests').mkdir()
                    bin_dir.mkdir()
                    (source / 'solution/solve.sh').write_text(script)
                    (source / 'tests/test.sh').write_text('original scorer\n')
                    (source / 'task.toml').write_text('[agent]\ntimeout_sec = 900.0\n')
                    # Isolate the repository setup contract; console behavior
                    # has its own shell and native Expect regression coverage.
                    with patch.dict('benchmarks.oracle_repairs.REPAIRS', {
                            name: hashlib.sha256(script.encode()).hexdigest()}), \
                         patch('benchmarks.oracle_repairs.sync_qemu_ssh', side_effect=lambda value: value):
                        repaired = prepare_repaired_task(name, source, root / 'repaired')
                    stub = bin_dir / 'bash'
                    stub.write_text('#!/bin/bash\nprintf "%s\\n" "$*" >> "$REPAIR_TEST_LOG"\n'
                                    f'exit {code}\n')
                    stub.chmod(0o700)
                    log = root / 'commands'
                    result = subprocess.run(['/bin/bash', str(repaired / 'solution/solve.sh')],
                        env={**os.environ, 'PATH': str(bin_dir), 'REPAIR_TEST_LOG': str(log)})
                    self.assertEqual(result.returncode, code)
                    self.assertEqual(log.read_text().splitlines(), ['/solution/guest_bullseye_snapshot.sh']
                                     + ([] if code else ['original QEMU solution']))
                    self.assertEqual((source / 'solution/solve.sh').read_text(), script)
                    for relative in ('tests/test.sh', 'task.toml'):
                        self.assertEqual((source / relative).read_bytes(), (repaired / relative).read_bytes())
                    self.assertEqual((repaired / 'solution/guest_bullseye_snapshot.sh').read_bytes(), HELPER.read_bytes())
                    evidence = json.loads((root / 'oracle-repair.json').read_text())
                    self.assertEqual(evidence['repaired_files']['guest_bullseye_snapshot.sh'],
                                     hashlib.sha256(HELPER.read_bytes()).hexdigest())


if __name__ == '__main__':
    unittest.main()
