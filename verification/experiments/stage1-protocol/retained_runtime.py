"""Run just the retained Stage1 component at the frozen extraction location."""
import argparse
import os
import subprocess
import sys
from pathlib import Path

import component

ROOT, HERE = component.ROOT, component.HERE


def retained_command(output, command, runtime):
    """Expose the frozen Charon source location for the retained component only."""
    target = Path(os.environ['CARGO_TARGET_DIR'])
    temporary = Path(os.environ.get('TMPDIR', str(output.parent)))
    writable = dict.fromkeys((output.parent, target, temporary))
    if any(not path.is_absolute() or not path.is_dir() for path in writable):
        raise ValueError('retained proof requires allocated absolute output, target and temporary directories')
    return [
        'bwrap', '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
        '--tmpfs', '/home', '--ro-bind', str(Path.home()), str(Path.home()),
        *runtime.tool_bind(ROOT),
        *[part for path in writable for part in ('--bind', str(path), str(path))],
        '--bind', str(ROOT), str(runtime.VIRTUAL), '--chdir', str(runtime.VIRTUAL),
        '--setenv', 'TMPDIR', str(temporary),
        '--', *command,
    ]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    output = args.output.absolute()
    output.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    runtime = component.load('stage1_retained_runtime', HERE/'formal_runtime.py')
    driver = runtime.VIRTUAL / HERE.relative_to(ROOT) / 'component.py'
    command = [sys.executable, str(driver), '--output', str(output)]
    if ROOT != runtime.VIRTUAL:
        command = retained_command(output, command, runtime)
    return subprocess.run(command, cwd=ROOT).returncode


if __name__ == '__main__':
    raise SystemExit(main())
