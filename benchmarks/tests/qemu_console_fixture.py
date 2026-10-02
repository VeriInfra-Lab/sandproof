"""Native Expect regression with a synthetic console, never QEMU or SSH.

Run with --expect /path/to/expect --source /path/to/pinned/qemu-alpine-ssh.
The original and repaired console scripts are extracted without executing
either task's shell, package installation, VM startup, or scoring code.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from benchmarks.oracle_repairs import repair_solution


CONSOLE = r'''
import json, os, subprocess, sys
from pathlib import Path
mode = os.environ["QEMU_FIXTURE_MODE"]
events = Path(os.environ["QEMU_FIXTURE_EVENTS"])
def event(value):
    with events.open("a") as log:
        log.write(json.dumps(value) + "\n")
def output(value):
    print(value, end="", flush=True)
output("\nlocalhost login: ")
logged_in, here, pending = False, False, False
for raw in sys.stdin:
    line = raw.rstrip("\r\n")
    if pending:
        # Input is still echoed by the PTY. It is NOT executed or completed.
        event("input_while_setup_pending")
        continue
    if here:
        if line == "EOF":
            here = False
            output("\nlocalhost:~# ")
        continue
    if not logged_in:
        if line == "root":
            logged_in = True
            output("\nWelcome to Alpine!\nlocalhost:~# ")
        else:
            output("\nlocalhost login: ")
    elif line.startswith("cat > setup_ssh.sh"):
        here = True
    elif line.startswith("chmod "):
        output("\nlocalhost:~# ")
    elif line.startswith("./setup_ssh.sh"):
        event("setup_started")
        if mode == "echo-only":
            pending = True
        elif mode == "eof":
            break
        else:
            # Execute the transmitted status wrapper around ONLY our local
            # exit-code stub. The real here-document was never written.
            result = subprocess.run(['/bin/sh', '-c', line], capture_output=True, text=True)
            assert result.returncode == 0, result.stderr
            event("setup_finished")
            output(result.stdout + "\nlocalhost:~# ")
'''


def console_script(solution):
    return solution.split("cat <<'EOF' > connect\n", 1)[1].split("\nEOF\n", 1)[0] + "\n"


def exercise(expect, source):
    results = []
    with tempfile.TemporaryDirectory(prefix='sp-qemu-console-') as tmp:
        root = Path(tmp)
        repaired = root / 'repaired'
        repair_solution('qemu-alpine-ssh', source / 'solution', repaired)
        scripts = {
            'original': console_script((source / 'solution/solve.sh').read_text()),
            'repaired': console_script((repaired / 'solve.sh').read_text()),
        }
        bin_dir = root / 'bin'
        bin_dir.mkdir()
        for name, body in {'telnet': CONSOLE, 'stty': ''}.items():
            path = bin_dir / name
            path.write_text('#!' + sys.executable + '\n' + body)
            path.chmod(0o700)
        for variant, mode, expected in (
                ('original', 'echo-only', 0),
                ('repaired', 'echo-only', 124),
                ('repaired', 'eof', 125),
                ('repaired', 'failure', 42),
                ('repaired', 'success', 0)):
            label = variant + '-' + mode
            working = root / label
            working.mkdir()
            stub = working / 'setup_ssh.sh'
            stub.write_text('#!/bin/sh\nexit ' + ('42' if mode == 'failure' else '0') + '\n')
            stub.chmod(0o700)
            path = root / (label + '.exp')
            path.write_text(scripts[variant])
            events = root / (label + '.events')
            result = subprocess.run([str(expect), '-f', str(path)], cwd=working, capture_output=True,
                text=True, timeout=45, env={**os.environ, 'PATH': str(bin_dir),
                'QEMU_FIXTURE_MODE': mode, 'QEMU_FIXTURE_EVENTS': str(events)})
            observed = ([json.loads(line) for line in events.read_text().splitlines()]
                        if events.exists() else [])
            if result.returncode != expected:
                raise AssertionError((label, result.returncode, expected, result.stdout, result.stderr))
            # The original may exit even before the echoed setup command is
            # consumed. The repaired cases must reach that command first.
            if variant == 'repaired':
                assert 'setup_started' in observed, (label, observed, result.stdout, result.stderr)
            assert ('setup_finished' in observed) == (mode in ('failure', 'success'))
            results.append({'case': label, 'exit_code': result.returncode,
                'setup_started': 'setup_started' in observed,
                'setup_finished': 'setup_finished' in observed,
                'console_sha256': hashlib.sha256(scripts[variant].encode()).hexdigest()})
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--expect', type=Path, required=True)
    parser.add_argument('--source', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps({'scope': 'native Expect with synthetic terminal echo; no KVM acceptance',
                      'cases': exercise(args.expect.resolve(), args.source.resolve())}))
