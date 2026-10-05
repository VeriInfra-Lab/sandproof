"""Shell failure propagation for the separately recorded QEMU SSH repair."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from benchmarks.oracle_repairs import prepare_repaired_task


CONSOLE_TASK = r'''qemu-system-x86_64
cat <<'BOOT' > wait
set timeout 300
BOOT
expect -f wait
cat <<'EOF' > connect
set timeout 30
set host "localhost"
set port "6665"
set username "root"
spawn telnet $host $port
expect "# "
send "cat > setup_ssh.sh << 'EOF'\n"
send "#!/bin/sh\n"
send "echo \"SSH setup complete\"\n"
send "EOF\n"
expect "# "
send "chmod +x setup_ssh.sh\n"
expect "# "
send "./setup_ssh.sh\n"
# Wait for setup to complete
expect "SSH setup complete"
expect "# "
send "echo 'Setup completed successfully!'\n"
expect "Setup completed successfully!"
# You can add more commands here if needed, or exit
EOF
expect -f connect
'''


class QemuConsoleTests(unittest.TestCase):
    def test_vm_and_console_failures_stop_before_later_steps(self):
        for failure in ('', 'qemu', 'wait', 'connect'):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                source, commands = root / 'source', root / 'bin'
                (source / 'solution').mkdir(parents=True)
                (source / 'tests').mkdir()
                commands.mkdir()
                (source / 'solution/solve.sh').write_text(CONSOLE_TASK)
                preserved = {'task.toml': '[agent]\ntimeout_sec = 900.0\n',
                             'tests/test.sh': 'original SSH scorer\n'}
                for relative, content in preserved.items():
                    (source / relative).write_text(content)
                with patch.dict('benchmarks.oracle_repairs.REPAIRS', {
                        'qemu-alpine-ssh': hashlib.sha256(CONSOLE_TASK.encode()).hexdigest()}):
                    repaired = prepare_repaired_task('qemu-alpine-ssh', source, root / 'repaired')
                for name, stage in {'bash': 'snapshot', 'qemu-system-x86_64': 'qemu',
                                    'expect': '$2'}.items():
                    path = commands / name
                    path.write_text('#!/bin/bash\nstage="' + stage + '"\n'
                        'printf "%s\\n" "$stage" >> "$CONSOLE_TEST_LOG"\n'
                        'if [ "$stage" = "$CONSOLE_TEST_FAILURE" ]; then exit 42; fi\n')
                    path.chmod(0o700)
                (commands / 'cat').symlink_to('/bin/cat')
                log = root / 'commands.log'
                result = subprocess.run(['/bin/bash', str(repaired / 'solution/solve.sh')],
                    cwd=root, capture_output=True, text=True, env={**os.environ,
                    'PATH': str(commands), 'CONSOLE_TEST_LOG': str(log),
                    'CONSOLE_TEST_FAILURE': failure})
                self.assertEqual(result.returncode, 42 if failure else 0, result.stderr)
                stages = ['snapshot', 'qemu', 'wait', 'connect']
                if failure:
                    stages = stages[:stages.index(failure) + 1]
                self.assertEqual(log.read_text().splitlines(), stages)
                self.assertEqual((source / 'solution/solve.sh').read_text(), CONSOLE_TASK)
                for relative, content in preserved.items():
                    self.assertEqual((repaired / relative).read_text(), content)
                evidence = json.loads((root / 'oracle-repair.json').read_text())
                self.assertNotEqual(evidence['original_files']['solve.sh'],
                                    evidence['repaired_files']['solve.sh'])


if __name__ == '__main__':
    unittest.main()
