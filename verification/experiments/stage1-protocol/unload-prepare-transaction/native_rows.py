"""Run the four original first-Unload SQLite assertions and three Rust mutants."""

import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
sys.path.insert(0, str(ROOT))
from tools.acceptance_receipts import sha, write_json
from tools.benchmark_ci import remove_private_tree
from tools.ci_retention import allocated_bytes, reject_mounts

SOURCE = Path('controller-service/src/controller/lifecycle_inbox/source_work')
CODEC = SOURCE/'codec.rs'
PARENT = SOURCE.with_suffix('.rs')
TESTS = SOURCE/'tests.rs'
FRAGMENT = SOURCE/'tests/codec_rows.rs'
TRANSACTION = Path('controller-service/src/controller/transaction.rs')
PREFIX = 'controller::lifecycle_inbox::source_work::tests::'
SLOT = PREFIX+'codec_rows::retains_existing_slot_then_uses_minimum_vacancy_for_an_absent_key'
ZERO = PREFIX+'codec_rows::rejects_zero_updates_without_reporting_a_saved_payload'
ROLLBACK = PREFIX+'codec_rows::second_update_failure_rolls_back_the_source_observation_prefix'
BEGIN = PREFIX+'proof_first_unload_prepare_begin_busy'
VARIANTS = {
    'original': ((SLOT, ZERO, ROLLBACK, BEGIN), None),
    'zero-ok': ((ZERO,), 'zero updated source rows must be rejected: Ok(())'),
    'commit-prefix': ((ROLLBACK,), 'the failed save must roll back the updated first row'),
    'deferred-begin': ((BEGIN,), 'original BEGIN IMMEDIATE must report SQLITE_BUSY before allocator read: Err(CorruptStore)'),
}


def replace(project, source, old, new, *, scope=None):
    path = project/source
    before = path.read_text()
    prefix, body = ('', before) if scope is None else before.split(scope, 1)
    if body.count(old) != 1:
        raise ValueError('missing or ambiguous original Rust operation: '+str(source))
    path.write_text(prefix+(scope or '')+body.replace(old, new, 1))
    return source


def stage(project, variant):
    fixtures = project/HERE.relative_to(ROOT)
    files = [TESTS, FRAGMENT]
    replace(project, TESTS, 'mod continue_application;\n', 'mod codec_rows;\nmod continue_application;\n')
    with (project/TESTS).open('ab') as tests:
        tests.write(b'\n'+(fixtures/'native-tests.rs.in').read_bytes()+(fixtures/'begin-contention.rs.in').read_bytes())
    with (project/FRAGMENT).open('xb') as output:
        output.write((fixtures/'codec-rows.rs.in').read_bytes())
    if variant == 'zero-ok':
        original = 'params![bytes,Sha256::digest(&bytes).as_slice(),key.as_slice(),id])? != 1'
        files.append(replace(project, CODEC, original, original.replace(' != 1', ' > 1')))
    elif variant == 'commit-prefix':
        original = '        codec::save(&tx, &sources, inbox.config.max_sessions)?;'
        changed = ('        if let Err(error) = codec::save(&tx, &sources, inbox.config.max_sessions) {\n'
                   '            tx.commit()?;\n            return Err(error);\n        }')
        files.append(replace(project, PARENT, original, changed, scope='    pub(crate) fn save_source_observation<'))
    elif variant == 'deferred-begin':
        original = 'let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;'
        files.append(replace(project, TRANSACTION, original, original.replace('::Immediate', '::Deferred'),
                             scope="impl<'a> WorldTransaction<'a> {\n    pub(super) fn begin("))
    return files


def command(argv, project, env, log):
    argv = list(map(str, argv))
    with log.open('xb') as stream:
        result = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                                stdout=stream, stderr=subprocess.STDOUT)
    return dict(argv=argv, cwd=str(project), exit_code=result.returncode, log_sha256=sha(log))


def native_variant(project, target, out, variant, env):
    out.mkdir()
    sources = {}
    for relative in stage(project, variant):
        dest = out/'overlay'/relative
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes((project/relative).read_bytes())
        sources[str(relative)] = sha(dest)
    build = out/'cargo.jsonl'
    built = command(['cargo', '--config', 'scripts/check-profiles.toml', 'test', '--locked', '--offline',
                     '-p', 'sandproof-controller-service', '--features', 'rusqlite/hooks', '--lib',
                     '--no-run', '--message-format=json'], project, env, build)
    if built['exit_code'] != 0:
        raise RuntimeError('actual Rust compilation failed: '+variant)
    rows = [json.loads(line) for line in build.read_text().splitlines() if line.startswith('{')]
    artifact, = [row for row in rows if row.get('reason') == 'compiler-artifact'
                 and row['target']['name'] == 'sandproof_controller_service'
                 and row['profile']['test'] and row.get('executable')]
    binary = Path(artifact['executable']).resolve(strict=True)
    if (artifact['fresh'] is not False or artifact['target']['src_path'] != str(project/'controller-service/src/lib.rs')
            or artifact['manifest_path'] != str(project/'controller-service/Cargo.toml')
            or not binary.is_relative_to(target)):
        raise ValueError('compiled test program does not identify the actual role: '+variant)
    binary_sha = sha(binary)
    names, rejected = VARIANTS[variant]
    log = out/'assertions.log'
    ran = command([binary, '--exact', '--nocapture', *names], project, env, log)
    text = log.read_text()
    expected_count = f'test result: ok. {len(names)} passed; 0 failed;' if rejected is None else 'test result: FAILED. 0 passed; 1 failed;'
    if (ran['exit_code'] != (0 if rejected is None else 101) or expected_count not in text
            or rejected is not None and rejected not in text):
        raise ValueError('wrong SQLite assertion outcome: '+variant)
    if sha(binary) != binary_sha or any(sha(project/name) != digest for name, digest in sources.items()):
        raise ValueError('variant source or binary changed during assertion: '+variant)
    return dict(compilation=built, assertions=ran, source_overlay=sources,
                test_names=list(names), rejection=rejected, binary_sha256=binary_sha)


def run(output, work_parent):
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'native':
        raise ValueError('first-Unload SQLite checks require the native test role')
    os.umask(0o077)
    output.mkdir(mode=0o700)
    path = HERE.parent/'archive.py'
    spec = importlib.util.spec_from_file_location('first_unload_native_archive', path)
    archive = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(archive)
    tar, manifest = output/'source.tar', output/'source-manifest.json'
    source = archive.build(ROOT, tar, manifest)
    work = Path(tempfile.mkdtemp(prefix='first-unload-sqlite.', dir=work_parent))
    info = work.stat()
    identity = [info.st_dev, info.st_ino]
    target = work/'target'
    env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_NET_OFFLINE='true',
               CARGO_INCREMENTAL='0', CARGO_TERM_COLOR='never')
    result = dict(kind='first-unload-sqlite-native', complete=False, source_commit=source['commit'],
                  source_tree=source['tree'], source_archive_sha256=source['archive_sha256'],
                  variants={}, private_work=str(work))
    try:
        for name in VARIANTS:
            project = work/name
            project.mkdir()
            with tarfile.open(tar) as inputs:
                inputs.extractall(project, filter='data')
            result['variants'][name] = native_variant(project, target, output/name, name, env)
        if len({row['binary_sha256'] for row in result['variants'].values()}) != len(VARIANTS):
            raise ValueError('independent Rust variants produced the same binary')
        if archive.verify(tar, manifest) != source:
            raise ValueError('original common candidate archive changed')
        reject_mounts(work)
        result['allocated_bytes_released'] = allocated_bytes(work)
        remove_private_tree(work, identity)
        result['complete'] = True
    finally:
        write_json(output/'result.json', result)
    print('4 original SQLite assertions and 3 compiled mutants passed', flush=True)


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--output', required=True, type=Path)
    cli.add_argument('--work-parent', required=True, type=Path)
    args = cli.parse_args()
    run(args.output.resolve(), args.work_parent.resolve(strict=True))
