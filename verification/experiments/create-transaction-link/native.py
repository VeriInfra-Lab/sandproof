"""Recheck the original stateful codec and transaction cell assertions."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
STAGE1 = HERE.parent / 'stage1-protocol'
sys.path.insert(0, str(ROOT))
from tools.acceptance_receipts import sha, write_json

TRANSACTION = Path('controller-service/src/controller/transaction.rs')
CELLS = 'controller::cache_tests::uncertain_commit_reconciles_both_durable_outcomes_for_every_write_family'
CODEC = ('controller::lifecycle_inbox::tests::'
         'record_codec_keeps_full_inputs_state_and_first_error_on_the_original_connection')
ORIGINAL = b'            committed: &self.committed.value,\n            candidate: &self.candidate,'
SWAPPED = b'            committed: &self.candidate,\n            candidate: &self.committed.value,'


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def run(output):
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'native':
        raise ValueError('transaction assertions require the native suite role')
    cpus = sorted(os.sched_getaffinity(0))[:4]
    os.sched_setaffinity(0, cpus)
    output.mkdir(parents=True, mode=0o700)
    project, target = output/'source', output/'target'
    project.mkdir(mode=0o700)
    started = time.monotonic()
    result = dict(kind='actual-transaction-native-component', complete=False, cpus=cpus, commands=[], cases={})
    env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_NET_OFFLINE='true',
               CARGO_BUILD_JOBS=str(len(cpus)), CARGO_INCREMENTAL='0', CARGO_TERM_COLOR='never')
    archive = load('transaction_committed_archive', STAGE1/'archive.py')
    legacy = load('transaction_original_artifacts', STAGE1/'source-origin/native.py')
    registry = load('transaction_original_registry', STAGE1/'envelope-writer/native.py')
    registry.HERE = Path('.')
    require = legacy.require

    def command(label, args, expected=0):
        log = output/(label+'.log')
        with log.open('xb') as stream:
            child = subprocess.run(list(map(str, args)), cwd=project, env=env, stdin=subprocess.DEVNULL,
                                   stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        result['commands'].append(dict(label=label, argv=list(map(str, args)), exit_code=child.returncode,
                                       log=str(log), log_sha256=sha(log)))
        require(child.returncode == expected, 'unexpected command result: '+label)
        return log

    def exact(phase, program, test, reject=False):
        log = command(phase, [program, test, '--exact', '--test-threads=1'], 101 if reject else 0)
        text = log.read_text()
        expected = 'FAILED' if reject else 'ok'
        require(re.findall(r'^test \S+ \.\.\. .*$', text, re.M) == [f'test {test} ... {expected}'],
                'exact assertion result differs: '+phase)
        if reject:
            require('assertion `left == right` failed' in text and 'controller/cache_tests.rs:' in text
                    and 'test result: FAILED. 0 passed; 1 failed; 0 ignored;' in text,
                    'wrong program did not fail the original cache assertion')
        else:
            require('test result: ok. 1 passed; 0 failed; 0 ignored;' in text,
                    'exact assertion did not pass once: '+phase)

    try:
        source = archive.build(ROOT, output/'source.tar', output/'source-manifest.json')
        require(archive.verify(output/'source.tar', output/'source-manifest.json') == source,
                'source archive changed before the native run')
        with tarfile.open(output/'source.tar') as tar:
            tar.extractall(project, filter='data')
        transaction = project/TRANSACTION
        original = transaction.read_bytes()
        require(original.count(ORIGINAL) == 2, 'original commit/drop references differ')
        wrong = original.replace(ORIGINAL, SWAPPED, 1)
        libraries = registry.registry()
        tools = {name: Path(subprocess.check_output(['rustup', 'which', name], cwd=project,
                     env=env, text=True).strip()) for name in ('cargo', 'rustc', 'clippy-driver')}
        tool_hashes = {name: sha(path) for name, path in tools.items()}
        cargo = ['--locked', '--offline', '-p', 'sandproof-controller-service', '--all-features', '--lib']
        result.update(source_head=source['commit'], source_tree=source['tree'],
                      source_archive_sha256=source['archive_sha256'], libraries=libraries, tools=tool_hashes)
        for phase, body in (('correct', original), ('swapped-cells', wrong), ('restored', original)):
            transaction.write_bytes(body)
            built = command(phase+'-build', ['cargo', 'test', *cargo, '--no-run', '--message-format=json'])
            if phase == 'correct':
                rows = legacy.artifacts(built, project, target)
                protocol, = [row for row in rows if row['target']['name'] == 'sandproof_protocol']
                linked, = [Path(name) for name in protocol['filenames'] if name.endswith('.rlib')]
                linked_sha = sha(linked)
            else:
                rows = []
                for line in built.read_text().splitlines():
                    try:
                        row = json.loads(line)
                    except ValueError:
                        continue
                    if row.get('reason') == 'compiler-artifact' and row['target']['name'] == 'sandproof_controller_service':
                        rows.append(row)
                require(sha(linked) == linked_sha, 'linked protocol library changed')
            artifact, = [row for row in rows if row['target']['name'] == 'sandproof_controller_service']
            require(artifact['manifest_path'] == str(project/'controller-service/Cargo.toml')
                    and artifact['profile']['test'] is True
                    and artifact['target']['kind'] == ['lib'] and artifact['fresh'] is False,
                    'actual controller-service test binary was not freshly compiled')
            program = Path(artifact['executable'])
            require(not program.is_symlink() and program.resolve().is_relative_to(target.resolve()),
                    'test executable is outside the candidate target')
            with program.open('rb') as stream:
                require(stream.read(4) == b'\x7fELF', 'actual Linux test executable required')
            program_hash = sha(program)
            listing = command(phase+'-list', [program, '--list']).read_text().splitlines()
            require(listing.count(CELLS+': test') == 1, 'original cache assertion missing')
            exact(phase+'-cells', program, CELLS, phase == 'swapped-cells')
            if phase == 'correct':
                require(listing.count(CODEC+': test') == 1, 'original codec assertion missing')
                exact('correct-codec', program, CODEC)
            require(sha(program) == program_hash and transaction.read_bytes() == body,
                    'compiled test or its input changed')
            result['cases'][phase] = dict(executable=program_hash, source=hashlib.sha256(body).hexdigest())
        command('strict-clippy', ['cargo', 'clippy', '--locked', '--offline', '-p',
            'sandproof-controller-service', '--all-features', '--all-targets', '--', '-D', 'warnings'])
        current, _ = archive.committed_inputs(ROOT)
        require(current == {key: source[key] for key in ('schema', 'commit', 'tree', 'files')}
                and archive.verify(output/'source.tar', output/'source-manifest.json') == source
                and all(sha(project/name) == item['sha256'] for name, item in source['files'].items())
                and registry.registry() == libraries
                and all(sha(path) == tool_hashes[name] for name, path in tools.items())
                and all(sha(Path(row['log'])) == row['log_sha256'] for row in result['commands']),
                'candidate, tool, library or log changed during checks')
        result.update(complete=True, rejected_by_same_assertion=True, source_restored=True)
    except Exception as error:
        result['error'] = repr(error)
        raise
    finally:
        result['seconds'] = time.monotonic()-started
        write_json(output/'result.json', result)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    run(parser.parse_args().output.resolve())
