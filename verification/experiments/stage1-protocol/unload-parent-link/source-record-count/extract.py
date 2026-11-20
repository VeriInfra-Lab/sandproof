"""Re-capture the original record loader and both count helpers."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import source_record_bounds_capture as bounds
from probe_prepare_iterators import GIB, run

SOURCE, TREE, ARCHIVE = bounds.SOURCE, bounds.TREE, bounds.ARCHIVE
NAMESPACE, OPTIONS = bounds.NAMESPACE, bounds.CFG
CODEC = bounds.sql.CODEC
VARIANTS = ('original', 'keep-one-position', 'zero-skips-policy')


def mutate(text, variant):
    if variant == 'keep-one-position':
        anchor = 'Ok(policy.max_operations_per_sandbox - 2)'
        if text.count(anchor) != 1:
            raise ValueError('the sole original two-position subtraction is required')
        return text.replace(anchor, anchor.replace('- 2', '- 1'), 1)
    if variant == 'zero-skips-policy':
        anchor = 'fn reserved_records(connection: &Connection, sessions: u16) -> Result<u64, JournalError> {\n'
        if text.count(anchor) != 1:
            raise ValueError('the original reserved-record entry must be unique')
        return text.replace(anchor, anchor + '    if sessions == 0 {\n        return Ok(0);\n    }\n', 1)
    if variant != 'original':
        raise ValueError('unknown variant')
    return text


def reconstruct(source, archive, variant):
    original = bounds.reconstruct_codec(source, archive, 'original').decode()
    return mutate(original, variant).encode()


def capture(output, tools, lock, variant):
    if (os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal'
            or shutil.disk_usage(output.parent).free < 80 * GIB):
        raise ValueError('the formal role and original capture budget are required')
    os.umask(0o077)
    os.sched_setaffinity(0, set(sorted(os.sched_getaffinity(0))[:4]))
    fixed = json.loads((bounds.sql.ROOT / 'verification/foundations-environment.json').read_bytes())
    charon, aeneas = tools / 'charon/bin/charon', tools / 'bin/aeneas'
    if any(bounds.sql.digest(binary) != fixed[name]['binary_sha256']
           for name, binary in (('charon', charon), ('aeneas', aeneas))):
        raise ValueError('the original fixed tools are required')
    if subprocess.check_output(['git', 'rev-parse', SOURCE + '^{tree}'], cwd=bounds.sql.ROOT, text=True).strip() != TREE:
        raise ValueError('the fixed production tree is required')
    output.mkdir(mode=0o700)
    report = dict(source=SOURCE, tree=TREE, variant=variant, options=OPTIONS,
                  driver=bounds.sql.digest(Path(__file__)), commands=[], complete=False)
    try:
        archive = output / 'source.tar'
        with archive.open('xb') as stream:
            subprocess.run(['git', 'archive', SOURCE], cwd=bounds.sql.ROOT, stdout=stream, check=True)
        if bounds.sql.digest(archive) != ARCHIVE:
            raise ValueError('the exact original production archive is required')
        source = output / 'source'
        source.mkdir(mode=0o700)
        (source / CODEC).write_bytes(reconstruct(source, archive, variant))
        report['codec'] = bounds.sql.digest(source / CODEC)
        selectors = [part for flag, group in (('--start-from', 'roots'), ('--include', 'include'),
                    ('--opaque', 'opaque')) for pattern in OPTIONS[group] for part in (flag, pattern)]
        ir, generated = output / 'actual.llbc', output / 'generated'
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4',
                   CARGO_INCREMENTAL='0', CARGO_TARGET_DIR=str(output / 'target'))
        for name in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            env.pop(name, None)
        commands = [
            ('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
                        *selectors, '--dest-file', ir, '--', '--manifest-path', 'controller-service/Cargo.toml',
                        '--lib', '--locked', '--offline', '--target', fixed['target']], 240),
            ('aeneas', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors', '-split-files',
                        '-emit-json', '-namespace', NAMESPACE, '-subdir', NAMESPACE,
                        '-dest', generated, ir], 120),
        ]
        with lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            for name, argv, seconds in commands:
                reply = run(name, argv, seconds, source, output, env)
                report['commands'].append(reply)
                if reply['exit_code'] or reply['bounded_stop']:
                    raise ValueError('the complete fixed ' + name + ' output is required')
        report['ir'] = bounds.sql.digest(ir)
        report['generated'] = {str(p.relative_to(generated)): bounds.sql.digest(p)
                               for p in sorted(generated.rglob('*')) if p.is_file()}
        report['admission'] = bounds.admit(ir, generated, fixed['target'])
        report['complete'] = True
    finally:
        (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(variant, 'complete original loader and record-count helpers')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True), args.variant)
