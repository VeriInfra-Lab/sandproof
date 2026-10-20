"""Isolated native checks. Run on the ordinary Linux test host, never the proof host."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import time
import traceback

MUTATIONS = {
    'node-id-only': (
        'if self.node != *node {', 'if self.node.node_id != node.node_id {',
        'exact_request_node_binding_includes_deployment_node_and_generation',
        'same node text must not bypass the original deployment/generation binding'),
    'no-upper-bound': (
        'self.node != request.node || self.retention_slots == 0 || self.retention_slots > 4096',
        'self.node != request.node || self.retention_slots == 0',
        'every_u16_retention_count_has_exact_guard_outcome',
        'original retention range must reject zero and every value above 4096'),
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[3])
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root = args.root.resolve()
    out = args.output.resolve()
    out.mkdir(mode=0o700)
    os.umask(0o077)
    started = time.monotonic()
    record = {'kind': 'schema-domains-native-module-not-full-project-acceptance', 'variants': {}}
    try:
        target = Path(os.environ['CARGO_TARGET_DIR']).resolve()
        if not target.is_dir():
            raise ValueError('provide the allocated stable Cargo target; do not make a new CI environment')
        if subprocess.check_output(['git', 'status', '--porcelain', '--untracked-files=no'],
                                   cwd=root, text=True):
            raise ValueError('native evidence must bind a clean committed source')
        commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip()
        archive = subprocess.check_output(['git', 'archive', commit], cwd=root)
        record['commit'] = commit
        record['archive_sha256'] = hashlib.sha256(archive).hexdigest()
        record['rustc'] = subprocess.check_output(['rustc', '-vV'], text=True)
        if not record['rustc'].startswith('rustc 1.97.0 '):
            raise ValueError('wrong native Rust toolchain')
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_INCREMENTAL='0',
                   RUST_TEST_THREADS='1', PYTHONDONTWRITEBYTECODE='1')
        binaries = out / 'binaries'
        binaries.mkdir()

        def run(command, source, log):
            with log.open('xb') as stream:
                return subprocess.run(command, cwd=source, env=env, stdout=stream,
                                      stderr=subprocess.STDOUT, timeout=1200).returncode

        for variant in ('correct', *MUTATIONS):
            source = out / (variant + '-source')
            source.mkdir(mode=0o700)
            with tarfile.open(fileobj=io.BytesIO(archive)) as package:
                package.extractall(source, filter='data')
            status = source / 'sandbox-internal/src/status.rs'
            expected = json.loads((source / 'verification/schema-domains-environment.json').read_text())
            if digest(status) != expected['source_files']['sandbox-internal/src/status.rs']:
                raise ValueError('original status source drifted')
            if variant != 'correct':
                before, after, test_name, failure = MUTATIONS[variant]
                text = status.read_text()
                if text.count(before) != 1:
                    raise ValueError('mutation source no longer matches exactly')
                status.write_text(text.replace(before, after))
            else:
                test_name = ''
            manifest = 'verification/extract-schema-domains/Cargo.toml'
            prefix = ['cargo', 'test', '--manifest-path', manifest, '--locked', '--offline']
            build_log = out / (variant + '-build.log')
            if run(prefix + ['--no-run', '--message-format=json'], source, build_log):
                raise ValueError('compile failure is not a semantic counterexample')
            executable_records = []
            for line in build_log.read_text().splitlines():
                try:
                    item = json.loads(line)
                except ValueError:
                    continue
                if (item.get('reason') == 'compiler-artifact' and item.get('executable')
                        and item['target']['name'] == 'status'):
                    executable_records.append(item)
            if len(executable_records) != 1 or executable_records[0]['fresh'] is not False:
                raise ValueError('expected one freshly compiled status test binary')
            executable = Path(executable_records[0]['executable']).resolve()
            if not executable.is_relative_to(target):
                raise ValueError('binary is outside the allocated target')
            binary = binaries / variant
            shutil.copy2(executable, binary)
            log = out / (variant + '-test.log')
            command = [str(binary), '--nocapture', '--test-threads=1']
            if test_name:
                command += ['--exact', test_name]
            code = run(command, source, log)
            text = log.read_text()
            if variant == 'correct':
                if code != 0 or 'test result: ok. 17 passed; 0 failed; 0 ignored;' not in text:
                    raise ValueError('correct Status schema tests failed or were omitted')
                commands = [
                    ['cargo', 'fmt', '--manifest-path', manifest, '--', '--check'],
                    ['cargo', 'clippy', '--manifest-path', manifest, '--all-targets', '--locked',
                     '--offline', '--', '-D', 'warnings'],
                    ['cargo', 'test', '-p', 'sandproof-internal', '--test', 'secure_probe',
                     'status_', '--locked', '--offline', '--', '--nocapture', '--test-threads=1'],
                    ['bash', 'scripts/python-entry.sh', '-m', 'unittest',
                     'tools.tests.test_schema_domains', '-v'],
                    ['bash', 'scripts/python-entry.sh', 'tools/check_schema_domains.py'],
                ]
                for index, command in enumerate(commands):
                    command_log = out / ('correct-step-' + str(index) + '.log')
                    if run(command, source, command_log):
                        raise ValueError('required native step failed: ' + repr(command))
                    record.setdefault('required_steps', []).append({
                        'command': command, 'exit_code': 0,
                        'log': command_log.name, 'log_sha256': digest(command_log),
                    })
                    if index == 2 and '3 passed; 0 failed;' not in command_log.read_text():
                        raise ValueError('original transport status tests were not all run')
            elif (code != 101 or failure not in text
                  or 'test result: FAILED. 0 passed; 1 failed; 0 ignored;' not in text):
                raise ValueError('wrong implementation was not rejected by the unchanged property')
            record['variants'][variant] = {
                'source_sha256': digest(status), 'binary_sha256': digest(binary),
                'build_log_sha256': digest(build_log), 'test_log_sha256': digest(log),
                'compile_exit': 0, 'fresh': True, 'test_exit': code, 'filter': test_name,
            }
            (out / 'progress.json').write_text(json.dumps(record, indent=2) + '\n')
        record.update(exit_code=0, complete=True)
    except BaseException as error:
        record.update(exit_code=1, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic() - started
    (out / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    if record.get('complete') is True:
        (out / 'complete.json').write_text(json.dumps(record, indent=2) + '\n')
    print(json.dumps(record))
    raise SystemExit(record['exit_code'])


if __name__ == '__main__':
    main()
