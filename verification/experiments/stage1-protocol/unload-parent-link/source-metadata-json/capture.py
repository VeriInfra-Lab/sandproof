"""Capture the original source Configuration JSON dictionaries for review."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import source_metadata_capture as metadata
from probe_prepare_iterators import GIB, stop, target_bytes


ROOT = metadata.sql.ROOT
CODEC = metadata.sql.CODEC
CONFIG = Path('controller-service/src/capture/source.rs')
CODEC_ROOT = metadata.sql.CODEC_ROOT
ROOTS = [CODEC_ROOT + '::' + name for name in metadata.SERDE[:2]]
NAMESPACE = 'SandproofSourceMetadataJson'
VARIANTS = ('original', 'swapped-limit-names')
OPTIONS = dict(roots=ROOTS, retained_roots=[], include=[
    CODEC_ROOT + '::*', metadata.CONFIG + '::*', 'sandproof_protocol',
], opaque=['sandproof_controller_service', 'serde_json', 'rusqlite', 'sha2'])


def run(label, argv, seconds, source, output, env):
    """Bound this process and its own target without reserving unrelated disk."""
    started = time.monotonic()
    log = output / (label + '.log')
    peak = 0
    bounded = False
    with log.open('xb') as stream:
        child = subprocess.Popen(argv, cwd=source, env=env, stdout=stream,
                                 stderr=subprocess.STDOUT, start_new_session=True)
        try:
            while child.poll() is None:
                peak = max(peak, target_bytes(output / 'target'))
                if time.monotonic() - started > seconds or peak > 4 * GIB:
                    bounded = True
                    break
                time.sleep(0.5)
        finally:
            stop(child)
    return dict(label=label, argv=list(map(str, argv)), exit_code=child.returncode,
                seconds=time.monotonic() - started, bounded_stop=bounded,
                peak_target_bytes=peak, log_sha256=metadata.sql.digest(log))


def swap_names(text):
    for field, other in (('max_disk_bytes', 'max_material_bytes'),
                         ('max_material_bytes', 'max_disk_bytes')):
        original = f'    pub {field}: u64,'
        if text.count(original) != 1:
            raise ValueError('the original six-field source limits must be present')
        text = text.replace(original, f'    #[serde(rename = "{other}")]\n' + original, 1)
    return text


def capture(output, tools, lock, variant):
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal':
        raise ValueError('source capture requires the original formal role')
    os.umask(0o077)
    os.sched_setaffinity(0, set(sorted(os.sched_getaffinity(0))[:4]))
    fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())
    charon, aeneas = tools / 'charon/bin/charon', tools / 'bin/aeneas'
    for name, binary in (('charon', charon), ('aeneas', aeneas)):
        if metadata.sql.digest(binary) != fixed[name]['binary_sha256']:
            raise ValueError('the existing fixed tool is required: ' + name)
    output.mkdir(mode=0o700)
    report = dict(source=metadata.SOURCE, tree=metadata.TREE, variant=variant,
                  options=OPTIONS, driver=metadata.sql.digest(Path(__file__)), commands=[],
                  tools={name: fixed[name]['binary_sha256'] for name in ('charon', 'aeneas')},
                  source_capture_complete=False, library_refinement_proved=False)
    try:
        archive = output / 'source.tar'
        with archive.open('xb') as stream:
            subprocess.run(['git', 'archive', metadata.SOURCE], cwd=ROOT, stdout=stream, check=True)
        if metadata.sql.digest(archive) != metadata.ARCHIVE:
            raise ValueError('the exact parent production archive is required')
        source = output / 'source'
        source.mkdir(mode=0o700)
        (source / CODEC).parent.mkdir(parents=True)
        (source / CODEC).write_bytes(metadata.reconstruct_codec(source, archive, 'original'))
        if variant == 'swapped-limit-names':
            limits = source / CONFIG
            limits.write_text(swap_names(limits.read_text()))
        report['staged'] = {str(path): metadata.sql.digest(source / path) for path in (CODEC, CONFIG)}
        ir = output / 'actual.llbc'
        selectors = [part for flag, group in (('--start-from', 'roots'), ('--include', 'include'),
                    ('--opaque', 'opaque')) for pattern in OPTIONS[group] for part in (flag, pattern)]
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4',
                   CARGO_INCREMENTAL='0', CARGO_TARGET_DIR=str(output / 'target'))
        for name in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            env.pop(name, None)
        with lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            command = [charon, 'cargo', '--preset=aeneas', '--error-on-warnings',
                       '--sysroot', 'default', *selectors, '--dest-file', ir, '--',
                       '--manifest-path', 'controller-service/Cargo.toml', '--lib',
                       '--locked', '--offline', '--target', fixed['target']]
            result = run('charon', command, 240, source, output, env)
            report['commands'].append(result)
            if result['exit_code'] or result['bounded_stop']:
                raise ValueError('the actual JSON dictionary capture did not finish')
            report['ir'] = metadata.sql.digest(ir)
            raw = json.loads(ir.read_bytes())
            translated = raw['translated']
            actual_options = translated['options']
            if (raw['has_errors'] is not False
                    or {row['key'] for row in translated['target_information']} != {fixed['target']}
                    or any(actual_options[flag] != OPTIONS[group] for flag, group in
                           (('start_from', 'roots'), ('include', 'include'), ('opaque', 'opaque')))
                    or any(actual_options[flag] for flag in
                           ('skip_borrowck', 'no_typecheck', 'no_normalize', 'no_reorder_decls'))):
                raise ValueError('the original complete target-specific LLBC is required')
            report['source_capture_complete'] = True
            command = [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors',
                       '-split-files', '-emit-json', '-namespace', NAMESPACE, '-subdir', NAMESPACE,
                       '-dest', output / 'generated', ir]
            report['commands'].append(run('aeneas', command, 120, source, output, env))
        report['generated'] = {str(p.relative_to(output / 'generated')): metadata.sql.digest(p)
                               for p in sorted((output / 'generated').rglob('*')) if p.is_file()}
    finally:
        (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(report['source_capture_complete'], [(c['label'], c['exit_code']) for c in report['commands']])


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', required=True, choices=VARIANTS)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, required=True, type=Path)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True),
            args.lock.resolve(strict=True), args.variant)
