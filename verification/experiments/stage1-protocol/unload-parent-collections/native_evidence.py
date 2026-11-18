"""Read back the original collection native cases without executing a binary."""
import argparse
from contextlib import contextmanager
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import subprocess
import tarfile

from registration_preflight import COLLECTION, ROOT, sha

CASES = ('correct', 'attempt-zero', 'restored')
TEST_FILTER = 'collection_link_contract'
TEST_PREFIX = 'controller::lifecycle_inbox::source_work::snapshot::' + TEST_FILTER + '::'
IDENTITY = TEST_PREFIX + 'full_identity_contract'
RECEIPTS = TEST_PREFIX + 'full_receipt_contract'
CONTRACT_ASSERTION = b'assert_eq!(original_action_identities(&values), expected);'
COMPILE = ['cargo', 'test', '--manifest-path', 'controller-service/Cargo.toml', '--lib',
           '--locked', '--offline', '--no-run', '--message-format=json']


class Evidence:
    def __init__(self, directory, bundle):
        self.directory, self.bundle = directory, bundle
        self.members = {}
        if bundle is not None:
            for member in bundle:
                if member.name in self.members:
                    raise ValueError('duplicate native evidence member: ' + member.name)
                self.members[member.name] = member

    @contextmanager
    def open(self, name):
        if self.bundle is None:
            relative = Path(name)
            path = self.directory / relative
            if (relative.is_absolute() or '..' in relative.parts or path.resolve() != path
                    or not path.is_file()):
                raise ValueError('missing or linked native evidence: ' + name)
            stream = path.open('rb')
        else:
            member = self.members.get(name)
            if member is None or not member.isfile():
                raise ValueError('missing or linked native evidence member: ' + name)
            stream = self.bundle.extractfile(member)
        with stream:
            yield stream

    def read(self, name):
        with self.open(name) as stream:
            return stream.read()

    def digest(self, name):
        with self.open(name) as stream:
            return hashlib.file_digest(stream, 'sha256').hexdigest()


def source_for_case(name, production, fixture, profile):
    source = production
    variant = 'attempt-zero' if name == 'attempt-zero' else 'correct'
    if variant == 'attempt-zero':
        before = profile['mutation']['before'].encode()
        if not before or source.count(before) != 1:
            raise ValueError('collection field omission is missing or ambiguous')
        source = source.replace(before, profile['mutation']['after'].encode(), 1)
    mutation = dict(path=profile['source'], variant=variant, original_sha256=sha(production),
                    source_sha256=sha(source))
    return source + b'\n' + fixture, mutation


def verify_assertions(name, code, log, source_path, source):
    lines = log.decode().splitlines()
    identity_status = 'FAILED' if name == 'attempt-zero' else 'ok'
    status = 'FAILED. 1 passed; 1 failed;' if name == 'attempt-zero' else 'ok. 2 passed; 0 failed;'
    if (code != (101 if name == 'attempt-zero' else 0)
            or f'test {IDENTITY} ... {identity_status}' not in lines
            or f'test {RECEIPTS} ... ok' not in lines
            or not any(line.startswith(f'test result: {status} 0 ignored;') for line in lines)):
        raise ValueError('native cases did not reach the original identity and receipt outcomes: ' + name)
    if name == 'attempt-zero':
        locations = [(i, line.index(CONTRACT_ASSERTION) + 1)
                     for i, line in enumerate(source.splitlines(), 1) if line.strip() == CONTRACT_ASSERTION]
        if len(locations) != 1:
            raise ValueError('missing unique complete identity assertion in actual source')
        line, column = locations[0]
        panic = (rf"^thread '{re.escape(IDENTITY)}' \([0-9]+\) panicked at "
                 rf"{re.escape(source_path)}:{line}:{column}:\nassertion `left == right` failed$")
        if not re.search(panic, log.decode(), re.M):
            raise ValueError('mutant did not reach the exact full identity assertion')


def compile_artifact(log, expected):
    rows = []
    for line in log.splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if isinstance(row, dict):
            rows.append(row)
    artifacts = [row for row in rows if row.get('reason') == 'compiler-artifact'
                 and row.get('executable') and row['target']['name'] == 'sandproof_controller_service'
                 and row['profile']['test'] is True]
    finished = [row.get('success') for row in rows if row.get('reason') == 'build-finished']
    if (len(finished) != 1 or finished[0] is not True or artifacts != [expected]
            or expected.get('fresh') is not False):
        raise ValueError('missing complete fresh native compiler artifact')


def verify_case(evidence, name, case, source, mutation):
    if (case['name'] != name or case.get('complete') is not True or case['mutation'] != mutation
            or case['tested_source_sha256'] != sha(source)):
        raise ValueError('changed or incomplete native case: ' + name)
    build, assertions = case['commands']
    artifact = case['artifact']
    manifest = PurePosixPath(artifact['manifest_path'])
    executable = PurePosixPath(artifact['executable'])
    original_source = manifest.parent.parent
    original_case = original_source.parent
    if (not manifest.is_absolute() or '..' in manifest.parts or '..' in executable.parts
            or original_case.name != name or original_source.name != 'source'
            or manifest != original_source / 'controller-service/Cargo.toml'
            or artifact['target']['src_path'] != str(original_source / 'controller-service/src/lib.rs')
            or not executable.is_relative_to(original_case / 'target')):
        raise ValueError('compiler artifact is outside the original isolated case: ' + name)
    if (build['name'] != 'compile' or build['argv'] != COMPILE or build['exit_code'] != 0
            or assertions['name'] != 'assertions' or assertions['argv'] != [str(executable), TEST_FILTER]
            or build.get('interrupted') or assertions.get('interrupted')):
        raise ValueError('different or incomplete original native commands: ' + name)
    build_log = evidence.read(name + '/compile.log')
    test_log = evidence.read(name + '/assertions.log')
    if sha(build_log) != build['log_sha256'] or sha(test_log) != assertions['log_sha256']:
        raise ValueError('changed original native log: ' + name)
    compile_artifact(build_log, artifact)
    if evidence.bundle is None:
        actual_source = name + '/source/' + mutation['path']
        binary = str(PurePosixPath(name) / executable.relative_to(original_case))
    else:
        actual_source = name + '/actual-source/' + mutation['path']
        binary = name + '/business-test-binary'
    if evidence.read(actual_source) != source or evidence.digest(binary) != case['binary_sha256']:
        raise ValueError('changed actual compiled source or native test binary: ' + name)
    verify_assertions(name, assertions['exit_code'], test_log, mutation['path'], source)
    return original_case.parent


def git_archive_sha(head):
    with subprocess.Popen(['git', 'archive', head], cwd=ROOT, stdout=subprocess.PIPE) as archive:
        digest = hashlib.file_digest(archive.stdout, 'sha256').hexdigest()
        if archive.wait() != 0:
            raise ValueError('could not read the exact candidate Git archive')
    return digest


def audit(evidence, head):
    raw = evidence.read('result.json')
    report = json.loads(raw)
    if (type(report.get('schema')) is not int or report['schema'] != 1
            or report.get('head') != head or report.get('complete') is not True
            or report.get('development_only') is not True or report.get('native_acceptance') is not False
            or report.get('public_acceptance') is not False or not report['cargo_version'].startswith('cargo 1.97.0 ')
            or [case['name'] for case in report['cases']] != list(CASES)):
        raise ValueError('native record is incomplete or from a different candidate')

    def git_file(path):
        return subprocess.check_output(['git', 'show', f'{head}:{path}'], cwd=ROOT)

    raw_profile = git_file(COLLECTION / 'capture-profile.json')
    profile = json.loads(raw_profile)
    production = git_file(profile['source'])
    fixture = git_file(COLLECTION / 'native-fixture.rs')
    original = report['source_archive_sha256']
    if (sha(production) != profile['source_sha256'] or sha(fixture) != report['fixture_sha256']
            or sha(raw_profile) != report['profile_sha256']
            or evidence.digest('source.tar') != original or git_archive_sha(head) != original):
        raise ValueError('native inputs do not match the exact candidate Git archive')
    locations = set()
    for name, case in zip(CASES, report['cases']):
        source, mutation = source_for_case(name, production, fixture, profile)
        locations.add(verify_case(evidence, name, case, source, mutation))
    if len(locations) != 1:
        raise ValueError('native cases do not share the same original isolation root')
    return dict(kind='unload-collection-native-readback', head=head, source_archive_sha256=original,
                original_result_sha256=sha(raw), cases=list(CASES), evidence_matches=True,
                native_executed=False, native_acceptance=False, public_acceptance=False)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument('--run-directory', type=Path)
    source.add_argument('--preserved-archive', type=Path)
    parser.add_argument('--head', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if not re.fullmatch(r'[0-9a-f]{40}', args.head):
        parser.error('--head must be the exact candidate commit')
    if args.run_directory is not None:
        report = audit(Evidence(args.run_directory.resolve(strict=True), None), args.head)
    else:
        with tarfile.open(args.preserved_archive, 'r:*') as bundle:
            report = audit(Evidence(None, bundle), args.head)
    with args.output.open('x') as stream:
        json.dump(report, stream, indent=2)
        stream.write('\n')
