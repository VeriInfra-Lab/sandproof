"""Capture the source guard and its actual metadata caller with fixed tools."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / 'source-metadata-json'))
from capture import run
import source_metadata_capture as metadata

CONFIG = Path('controller-service/src/capture/source.rs')
NAMESPACE = 'SandproofSourceConfigGuards'
VARIANTS = ('original', 'omit-configuration-bound')


def omit_configuration_bound(text):
    check = '\n            && (1..=1_048_576).contains(&self.max_configuration_bytes)'
    if text.count(check) != 1:
        raise ValueError('the sole original configuration size guard is required')
    return text.replace(check, '', 1)


def capture(output, tools, lock, variant):
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal':
        raise ValueError('the existing formal role is required')
    os.umask(0o077)
    os.sched_setaffinity(0, set(sorted(os.sched_getaffinity(0))[:4]))
    fixed = json.loads((metadata.sql.ROOT / 'verification/foundations-environment.json').read_bytes())
    charon, aeneas = tools / 'charon/bin/charon', tools / 'bin/aeneas'
    for name, binary in (('charon', charon), ('aeneas', aeneas)):
        if metadata.sql.digest(binary) != fixed[name]['binary_sha256']:
            raise ValueError('the original fixed tool is required: ' + name)
    output.mkdir(mode=0o700)
    result = dict(source=metadata.SOURCE, tree=metadata.TREE, variant=variant, options=metadata.CFG,
                  driver=metadata.sql.digest(Path(__file__)), commands=[], complete=False)
    try:
        archive = output / 'source.tar'
        with archive.open('xb') as stream:
            subprocess.run(['git', 'archive', metadata.SOURCE], cwd=metadata.sql.ROOT, stdout=stream, check=True)
        if metadata.sql.digest(archive) != metadata.ARCHIVE:
            raise ValueError('the exact parent production archive is required')
        source = output / 'source'
        source.mkdir(mode=0o700)
        codec = metadata.reconstruct_codec(source, archive, 'original')
        (source / metadata.sql.CODEC).write_bytes(codec)
        if variant == 'omit-configuration-bound':
            (source / CONFIG).write_text(omit_configuration_bound((source / CONFIG).read_text()))
        result['staged'] = {str(path): metadata.sql.digest(source / path)
                            for path in (metadata.sql.CODEC, CONFIG)}
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4',
                   CARGO_INCREMENTAL='0', CARGO_TARGET_DIR=str(output / 'target'))
        for name in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            env.pop(name, None)
        selectors = [part for flag, group in (('--start-from', 'roots'), ('--include', 'include'),
                    ('--opaque', 'opaque')) for pattern in metadata.CFG[group] for part in (flag, pattern)]
        ir, generated = output / 'actual.llbc', output / 'generated'
        commands = [
            ('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings',
                        '--sysroot', 'default', *selectors, '--dest-file', ir, '--',
                        '--manifest-path', 'controller-service/Cargo.toml', '--lib',
                        '--locked', '--offline', '--target', fixed['target']], 240),
            ('aeneas', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors',
                        '-split-files', '-emit-json', '-namespace', NAMESPACE, '-subdir', NAMESPACE,
                        '-dest', generated, ir], 120),
        ]
        with lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            for name, argv, seconds in commands:
                status = run(name, argv, seconds, source, output, env)
                result['commands'].append(status)
                if status['exit_code'] or status['bounded_stop']:
                    raise ValueError('the complete fixed ' + name + ' output is required')
        result['ir'] = metadata.sql.digest(ir)
        result['generated'] = {str(p.relative_to(generated)): metadata.sql.digest(p)
                               for p in sorted(generated.rglob('*')) if p.is_file()}
        result['admission'] = metadata.admit(ir, generated, fixed['target'])
        result['complete'] = True
    finally:
        (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(variant, 'complete actual metadata caller and configuration guard')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', required=True, choices=VARIANTS)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, required=True, type=Path)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True), args.variant)
