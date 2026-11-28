"""Record the original full Stage1 run and publish it on a protected test merge."""
import argparse
from contextlib import contextmanager
from dataclasses import asdict, dataclass
import fcntl
import json
import os
from pathlib import Path
import pwd
import re
import signal
import stat
import subprocess
import sys

from tools import acceptance_receipts as receipts
from tools import formal_builds
from tools.ci_e2e_supervisor import PREFIX, REPO, Checks, git, is_ancestor
from tools.ci_github_app import (APIError, CHECK_PERMISSIONS, PERMISSIONS, SOURCE_PERMISSIONS,
                               GitHubApp, Transport)
from tools.ci_merge_queue import QueueGroup, TARGET, graphql_lookup
from tools.native_task import OWNER as NATIVE_TASK_OWNER, preserve
from tools.system_acceptance import digest, json_file, private_path, write_once

GATE = 'Promotion / Stage1'
REFERENCE = 'Promotion / Stage1 evidence'
E2E = 'E2E / terminal-bench-ci'
GATE_APP = 5175756  # Existing protected E2E App
ACTIONS_APP = 15368
ORDINARY = ('pr-title', 'pr-policy', 'sdk-contract', 'harbor-inputs')
SCOPE = 'stage1-protocol'
ROOT = Path(__file__).resolve().parents[1]
BINARY = ['bash', 'scripts/python-entry.sh']
MOUNT = Path('/mnt')
STORAGE_POLICY = Path('/etc/sandproof-stage1-promotion.json')
FORMAL_TARGET = Path('formal-cargo-target')


def oid(value):
    if not isinstance(value, str) or not re.fullmatch('[0-9a-f]{40}', value):
        raise ValueError('invalid Git object identity')
    return value


@dataclass(frozen=True)
class Candidate:
    pull: int
    source: str
    head: str
    base: str
    merge: str
    tree: str

    def __post_init__(self):
        if self.source not in ('integration', 'main'):
            raise ValueError('only protected source refs may promote to test')
        if type(self.pull) is not int or not 0 < self.pull < 10**9:
            raise ValueError('invalid promotion PR number')
        for value in (self.head, self.base, self.merge, self.tree):
            oid(value)
        if len({self.head, self.base, self.merge}) != 3:
            raise ValueError('promotion must be a distinct two-parent merge')

    def refs(self):
        return 'refs/heads/' + self.source, 'refs/heads/test', f'refs/pull/{self.pull}/merge'

    def observe(self, *, source_client, proxy):
        """Re-read all three refs in one request immediately before publication."""
        names = self.refs()
        lines = git('ls-remote', REPO, *names, source_client=source_client, proxy=proxy).splitlines()
        pairs = [line.split() for line in lines]
        if any(len(pair) != 2 for pair in pairs):
            raise ValueError('malformed GitHub ref readback')
        if len(pairs) != len(names) or {name: sha for sha, name in pairs} != dict(
                zip(names, (self.head, self.base, self.merge))):
            raise ValueError('protected head, base or GitHub test merge moved')
        # Public PR metadata needs no installation permission or credential.
        pull = Transport(proxy)('GET', PREFIX + 'pulls/' + str(self.pull), None)
        repo = REPO.removeprefix('https://github.com/').removesuffix('.git')
        if (pull.get('number') != self.pull or pull.get('state') != 'open'
                or pull.get('draft') is not False or pull.get('mergeable') is not True
                or pull.get('merge_commit_sha') != self.merge
                or pull['head'].get('ref') != self.source or pull['head'].get('sha') != self.head
                or pull['base'].get('ref') != 'test' or pull['base'].get('sha') != self.base
                or any(pull[side]['repo'].get('full_name') != repo for side in ('head', 'base'))):
            raise ValueError('GitHub PR is no longer an open protected promotion on the exact merge')

    def check_checkout(self, checkout):
        current = git('rev-parse', 'HEAD', cwd=checkout)
        if current not in (self.head, self.merge) or git(
                'status', '--porcelain', '--untracked-files=normal', cwd=checkout):
            raise ValueError('promotion requires a clean checkout of its exact source or test merge')
        if (git('rev-parse', 'HEAD^{tree}', cwd=checkout) != self.tree
                or git('show', '-s', '--format=%P', self.merge, cwd=checkout).split() != [self.base, self.head]
                or git('rev-parse', self.head + '^{tree}', cwd=checkout) != self.tree
                or git('rev-parse', self.merge + '^{tree}', cwd=checkout) != self.tree
                or not is_ancestor(self.base, self.head, cwd=checkout)):
            raise ValueError('GitHub test merge is not the source tree on the current target base')


def msra_directory(path):
    directory = private_path(path, directory=True)
    if not directory.is_relative_to(MOUNT) or directory == MOUNT:
        raise ValueError('place promotion evidence under /mnt')
    return directory


def promotion_registry():
    attributes = STORAGE_POLICY.lstat()
    if (STORAGE_POLICY.resolve(strict=True) != STORAGE_POLICY
            or not stat.S_ISREG(attributes.st_mode) or attributes.st_uid != 0
            or attributes.st_mode & 0o022):
        raise ValueError('promotion storage policy must be a root-owned, non-writable host file')
    policy = json_file(STORAGE_POLICY)
    if (set(policy) != {'schema', 'evidence_uid', 'registry'} or policy['schema'] != 1
            or policy['evidence_uid'] != os.geteuid() or os.geteuid() == 0):
        raise ValueError('use the configured promotion evidence account and registry')
    return msra_directory(policy['registry'])


@contextmanager
def registered_promotion(candidate):
    key = receipts.digest(asdict(candidate))
    directory = promotion_registry() / key
    directory.mkdir(mode=0o700, exist_ok=True)
    private_path(directory, directory=True)
    with open(directory/'run.lock', 'a', opener=lambda p, f: os.open(p, f | os.O_NOFOLLOW, 0o600)) as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise ValueError('this exact promotion already has a running or recovering attempt') from error
        yield directory, lock.fileno()


@contextmanager
def operator_lock(env, path_name, fd_name):
    """Use the inherited coordination lock required by the selected suite."""
    path = Path(env[path_name])
    fd = int(env[fd_name])
    expected = private_path(path).stat()
    actual = os.fstat(fd)
    if (expected.st_dev, expected.st_ino) != (actual.st_dev, actual.st_ino):
        raise ValueError('promotion requires the inherited outer coordination lock')
    fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    yield fd


def run_entry(command, *, log, checkout, env, pass_fds=()):
    with log.open('xb') as output:
        subprocess.run(command, cwd=checkout, env=env, stdout=output,
                       stderr=subprocess.STDOUT, check=True, pass_fds=pass_fds)


def check_stage1_registration(root):
    from tools.proof_native_suite import load

    stage1 = root / 'verification/experiments/stage1-protocol'
    registered = load('stage1_promotion_sources', stage1 / 'registered.py')
    registered.validate_registration(root, profile='full')
    # The original drivers check these reviewed input closures separately.
    # Run those same guards before compiling the workspace and native cases.
    for case in ('pause-request', 'source-question', 'source-origin', 'lifetime-row'):
        support = load('stage1_promotion_' + case.replace('-', '_'), stage1 / case / 'support.py')
        try:
            support.source_guard()
        except (OSError, ValueError) as error:
            raise ValueError(f'{case}: {error}') from error


def check_stage1_inputs(checkout, output, env):
    registration = ('from pathlib import Path; '
                    'from tools.ci_promotion_acceptance import check_stage1_registration; '
                    'check_stage1_registration(Path.cwd())')
    checks = (
        ('stage1-freeze.log', ['shasum', '-a', '256', '--check', '--quiet',
                              'verification/stage1-protocol-inputs.sha256']),
        ('stage1-registration.log', [*BINARY, '-c', registration]),
    )
    for filename, command in checks:
        log = output / filename
        try:
            run_entry(command, log=log, checkout=checkout, env=env)
        except subprocess.CalledProcessError:
            print(f'Stage1 input preflight failed: {log}', file=sys.stderr)
            raise


def evidence_path(reference, parent, *, filename):
    path = private_path(reference['path'])
    if not path.is_relative_to(parent) or path.name != filename or digest(path) != reference['sha256']:
        raise ValueError('original promotion evidence missing or changed: ' + filename)
    return path


def attest_evidence(native, formal, candidate, checkout):
    """Read outputs of the trusted original drivers, including their scope and origin."""
    seal_file = private_path(native / 'complete.json')
    formal_file = private_path(formal / 'registered-result.json')
    context_file = private_path(formal / 'execution-inputs.json')
    seal = json_file(seal_file, limit=16 * 1024**2)
    record = json_file(formal_file, limit=16 * 1024**2)
    context = json_file(context_file, limit=16 * 1024**2)
    source = receipts.digest(receipts.repository_inputs(checkout))
    native_sha = digest(seal_file)
    if (seal.get('scope') != SCOPE or seal.get('complete') is not True
            or seal.get('scope_complete') is not True or seal.get('exit_code') != 0
            or seal.get('repository_sha256') != source):
        raise ValueError('original native seal is not the complete current Stage1 scope')
    if (record.get('kind') != 'registered-stage1-formal-drivers' or record.get('profile') != 'full'
            or record.get('complete') is not True or record.get('execution_failures') != {}
            or record.get('shared_seal_sha256') != native_sha
            or record.get('source_snapshot_sha256') != source
            or record.get('template_audits', {}).get('missing') != []
            or context.get('source_tree') != candidate.tree or context.get('profile') != 'full'
            or context.get('native', {}).get('seal_sha256') != native_sha
            or not context.get('runtime')):
        raise ValueError('full formal result did not consume the same native seal, source tree and tools')
    ordinary = evidence_path(record['native_receipts']['functional'], native, filename='receipt.json')
    ordinary_key = json_file(ordinary, limit=16 * 1024**2).get('key', {})
    if (seal.get('commands', {}).get(ordinary.parent.name) != digest(ordinary)
            or ordinary_key.get('label') != 'functional'
            or receipts.load_success(ordinary.parent, ordinary_key) is None):
        raise ValueError('full Rust workspace receipt is absent from the native seal')
    evidence_path(record['retained_result'], formal.parent, filename='result.json')
    rows = record['results']
    if not rows or len({row['name'] for row in rows}) != len(rows):
        raise ValueError('missing or duplicate registration results')
    for row in rows:
        evidence_path(dict(path=row['checkpoint_path'], sha256=row['checkpoint_sha256']),
                      MOUNT, filename='checkpoint.json')
        evidence_path(dict(path=row['result_path'], sha256=row['result_sha256']),
                      MOUNT, filename='result.json')
    return dict(native=native_sha, formal=digest(formal_file), context=digest(context_file),
                native_tools=receipts.digest(seal['runtime']), formal_tools=receipts.digest(context['runtime']))


def attempt_record(candidate, directory):
    attempt = json_file(private_path(directory/'attempt.json'))
    if attempt.get('schema') != 1 or attempt.get('candidate') != asdict(candidate):
        raise ValueError('resume requires the same PR, head, base, merge and source tree')
    return attempt


def previous_attempt(candidate, directory):
    directory = msra_directory(directory)
    if os.path.lexists(directory/'complete.json'):
        raise ValueError('previous run already completed; use its original output for publication')
    attempt = attempt_record(candidate, directory)
    if os.path.lexists(directory/'formal-ready.json') or formal_exited_zero(candidate, directory):
        raise ValueError('original formal process exited zero; close out the original output before publication')
    native = msra_directory(attempt['native_path'])
    seal = private_path(native/'complete.json')
    expected = directory/'native-seal.json'
    if os.path.lexists(expected) and json_file(private_path(expected)) != {'sha256': digest(seal)}:
        raise ValueError('original native seal changed since the interrupted attempt')
    records = list(directory.glob('tmp/sandproof-stage1-protocol.*/registered/registered-result.json'))
    if len(records) > 1:
        raise ValueError('multiple formal attempts in one original output')
    prior = private_path(records[0]) if records else None
    if prior is not None:
        formal = json_file(prior, limit=16 * 1024**2)
        if formal.get('profile') != 'full' or formal.get('shared_seal_sha256') != digest(seal):
            raise ValueError('original formal checkpoints do not consume this complete native seal')
    return native, prior


def formal_exited_zero(candidate, output):
    exit_path = output/formal_builds.DIRECTORY/'native-task-exit.json'
    if not os.path.lexists(exit_path):
        return False
    _, enrollment, targets = formal_builds.admission(output)
    terminal = json_file(private_path(exit_path))
    if (enrollment['candidate'] not in (candidate.head, candidate.merge)
            or targets != [output/FORMAL_TARGET]
            or type(terminal.get('returncode')) is not int
            or type(terminal.get('process_group')) is not int or terminal['process_group'] <= 1):
        raise ValueError('original formal execution record differs from the registered attempt')
    return terminal['returncode'] == 0


def retry_before_formal(candidate, directory):
    if not os.path.lexists(directory/'attempt.json'):
        return not any(directory.iterdir())  # Registration interrupted before launching anything.
    attempt = attempt_record(candidate, directory)
    native = directory/'native'
    if (Path(attempt['native_path']) != native or os.path.lexists(native/'complete.json')
            or os.path.lexists(directory/'native-seal.json')):
        return False
    if not os.path.lexists(native):
        return True  # No managed native run was enrolled.
    retired_path = native/'native-task-retired.json'
    if not os.path.lexists(retired_path):
        return False
    private_path(native, directory=True)
    retired = json_file(private_path(retired_path))
    code = retired.get('returncode')
    enrollment = json_file(private_path(native/'native-task.json'))
    terminal = json_file(private_path(native/'native-task-exit.json'))
    identity = native.stat()
    executed = retired.get('candidate')
    return (executed in (candidate.head, candidate.merge) and retired.get('scope') == SCOPE
            and enrollment.get('kind') == NATIVE_TASK_OWNER and enrollment.get('scope') == SCOPE
            and enrollment.get('candidate') == executed
            and enrollment.get('identity') == [identity.st_dev, identity.st_ino]
            and type(code) is int and code != 0 and terminal.get('returncode') == code
            and retired.get('status') in ('failed', 'cancelled', 'no-build-allocated')
            and retired.get('remaining_build_bytes') == 0)


def registered_output(candidate, record):
    row = json_file(private_path(record))
    if row.get('schema') != 1 or row.get('candidate') != asdict(candidate):
        raise ValueError('original promotion registration changed')
    output = msra_directory(row['output'])
    actual = output.stat()
    if row.get('output_identity') != [actual.st_dev, actual.st_ino]:
        raise ValueError('previously registered promotion output was replaced')
    return output


def run_registered(candidate, checkout, output, *, native=None, resume_from=None):
    candidate.check_checkout(checkout)
    output = msra_directory(output)
    with registered_promotion(candidate) as (registry, lock_fd):
        if output.is_relative_to(registry.parent) or registry.parent.is_relative_to(output):
            raise ValueError('promotion output and registry require separate directories')
        records = sorted(registry.glob('attempt-*.json'))
        if records:
            current = registered_output(candidate, records[-1])
            if resume_from is None or msra_directory(resume_from) != current:
                raise ValueError('exact promotion already registered; use its latest recorded output to resume or publish')
        if any(output.iterdir()):
            raise ValueError('provide a new empty output and preserve the earlier promotion records')
        restart = False
        if resume_from is not None:
            resume_from = msra_directory(resume_from)
            if not os.path.lexists(resume_from/'complete.json') and retry_before_formal(candidate, resume_from):
                if not records:
                    raise ValueError('retry of failed native work requires an original registration')
                restart = True
            else:
                previous_attempt(candidate, resume_from)
                if native is not None:
                    raise ValueError('resume must keep the existing complete native seal')
        actual = output.stat()
        record = dict(schema=1, candidate=asdict(candidate), output=str(output),
                      output_identity=[actual.st_dev, actual.st_ino],
                      previous=str(resume_from) if resume_from is not None else None)
        write_once(registry / f'attempt-{len(records) + 1:08d}.json', record)
        return run_stage(candidate, checkout, output, native=native,
                         resume_from=None if restart else resume_from, registration_fd=lock_fd)


def closeout_registered(candidate, checkout, output):
    output = msra_directory(output)
    with registered_promotion(candidate) as (registry, _):
        records = sorted(registry.glob('attempt-*.json'))
        if not records or registered_output(candidate, records[-1]) != output:
            raise ValueError('formal closeout requires the original latest registered promotion output')
        return closeout_formal(candidate, checkout, output, Path(os.environ['SANDPROOF_NATIVE_OBSERVER']).resolve(strict=True))


def closeout_formal(candidate, checkout, output, observer):
    candidate.check_checkout(checkout)
    _, enrollment, targets = formal_builds.admission(output)
    if (targets != [output/FORMAL_TARGET]
            or enrollment['candidate'] != git('rev-parse', 'HEAD', cwd=checkout)):
        raise ValueError('formal closeout requires the original execution commit and single promotion target')
    attempt = output/'attempt.json'
    native = msra_directory(attempt_record(candidate, output)['native_path'])
    native_seal = private_path(native/'complete.json')
    bound_seal = private_path(output/'native-seal.json')
    seal_digest = digest(native_seal)
    if json_file(bound_seal) != {'sha256': seal_digest}:
        raise ValueError('original native seal changed since the formal execution')
    ready = output/'formal-ready.json'
    complete = output/'complete.json'

    def readback(terminal):
        files = {attempt: digest(attempt), bound_seal: digest(bound_seal), native_seal: seal_digest}
        code = terminal['returncode']
        if code != 0:
            if code in (-signal.SIGKILL, 128 + signal.SIGKILL) or any(os.path.lexists(p) for p in (ready, complete)):
                raise ValueError('original formal execution has an unknown or inconsistent terminal state')
            # The helper separately binds the actually waited-for exit and its raw log.
            return files
        formal, = (output/'tmp').glob('sandproof-stage1-protocol.*/registered')
        formal = private_path(formal, directory=True)
        bound = attest_evidence(native, formal, candidate, checkout)
        preserve(ready, dict(schema=1, candidate=asdict(candidate), native_path=str(native),
                             formal_path=str(formal), digests=bound))
        files.update({ready: digest(ready), formal/'registered-result.json': bound['formal'],
                      formal/'execution-inputs.json': bound['context']})
        return files

    retired = formal_builds.retire(output, observer, readback)
    if retired['returncode'] != 0:
        return retired
    result = json_file(private_path(ready))
    preserve(complete, result)
    return result


def run_stage(candidate, checkout, output, *, native=None, resume_from=None, registration_fd=None):
    """Under the usual msra lock and env, run Rust once then the original Lean entry."""
    candidate.check_checkout(checkout)
    output = msra_directory(output)
    prior = None
    if resume_from is not None:
        original_native, prior = previous_attempt(candidate, resume_from)
    else:
        original_native = msra_directory(native) if native else None
    native = original_native or output / 'native'
    write_once(output/'attempt.json', dict(schema=1, candidate=asdict(candidate),
                                          native_path=str(native)))
    temporary = output / 'tmp'
    temporary.mkdir(mode=0o700)
    env = dict(os.environ, SANDPROOF_NATIVE_EVIDENCE=str(native))
    env.update(SANDPROOF_PROOF_TOOL_ROOT=str(checkout/'.tools'),
               RUSTUP_AUTO_INSTALL='0')
    env.pop('PYTHONPATH', None)
    # An imported dotenv or launch shell must not leak a GitHub token into a driver.
    for name in ('CREDENTIALS_DIRECTORY', 'GH_TOKEN', 'GITHUB_TOKEN'):
        env.pop(name, None)
    observer = Path(env['SANDPROOF_NATIVE_OBSERVER']).resolve(strict=True)
    execution_commit = git('rev-parse', 'HEAD', cwd=checkout)
    if original_native is None:
        configured = Path(env.get('CARGO_TARGET_DIR', '.'))
        if not configured.is_absolute():
            raise ValueError('set the operator-approved shared native Cargo target under /mnt')
        cache = configured.resolve(strict=True)
        cache_stat = cache.stat()
        if (not cache.is_relative_to(MOUNT) or cache == MOUNT or not cache.is_dir()
                or cache_stat.st_uid != os.geteuid() or stat.S_IMODE(cache_stat.st_mode) & 0o022
                or not cache_stat.st_mode & stat.S_IWUSR
                or any(cache.is_relative_to(path) or path.is_relative_to(cache)
                       for path in (checkout, output))):
            raise ValueError('native Cargo cache must be owner-writable and separate from source and evidence')
        check_stage1_inputs(checkout, output, env)
        with operator_lock(env, 'SANDPROOF_PROOF_HEAVY_LOCK',
                           'SANDPROOF_PROOF_HEAVY_LOCK_FD') as fd:
            native_env = dict(env, SANDPROOF_ACCEPTANCE_ROLE='native')
            native_env.pop('SANDPROOF_LOCK_FD', None)
            run_entry([*BINARY, 'tools/native_task.py', 'run', '--scope', SCOPE,
                       '--candidate', execution_commit, '--output', str(native), '--observer', str(observer)],
                      log=output/'native.log', checkout=checkout,
                      env=native_env, pass_fds=(fd,) if registration_fd is None else (fd, registration_fd))
    elif (native/'native-task.json').exists():
        # Older complete original seals predate managed retirement. A new
        # managed invocation with pending retirement must resume it, not rerun.
        retired = json_file(private_path(native/'native-task-retired.json'))
        if retired.get('returncode') != 0 or retired.get('remaining_build_bytes') != 0:
            raise ValueError('original native run does not have successful retirement')
    write_once(output/'native-seal.json', dict(sha256=digest(private_path(native/'complete.json'))))
    with operator_lock(env, 'SANDPROOF_LOCK_FILE', 'SANDPROOF_LOCK_FD') as fd:
        # The formal entry itself admits the original complete native seal
        # under current source, environment and tool rules, including functional.
        formal_builds.enroll(output, candidate=execution_commit, observer=observer, targets=(FORMAL_TARGET,))
        cargo_target = output / FORMAL_TARGET
        cargo_target.mkdir(mode=0o700)
        formal_env = dict(env, SANDPROOF_ACCEPTANCE_ROLE='formal', TMPDIR=str(temporary),
                          CARGO_TARGET_DIR=str(cargo_target))
        formal_env.pop('SANDPROOF_PROOF_HEAVY_LOCK_FD', None)
        if resume_from is not None:
            formal_env.pop('SANDPROOF_STAGE1_RESUME_FROM', None)
            if prior is not None:
                formal_env['SANDPROOF_STAGE1_RESUME_FROM'] = str(prior)
        command = ['bash', 'scripts/verify-stage1-protocol.sh']
        exit_record = formal_builds.execute(output, command, cwd=checkout, env=formal_env,
            pass_fds=(fd,) if registration_fd is None else (fd, registration_fd))
        code = exit_record['returncode']
    try:
        result = closeout_formal(candidate, checkout, output, observer)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f'Formal build closeout pending in {output}: {error}', file=sys.stderr)
        if code == 0:
            raise subprocess.CalledProcessError(75, ['closeout', str(output)]) from error
    if code != 0:
        raise subprocess.CalledProcessError(code if code > 0 else 128-code, command)
    return result


def read_stage(candidate, checkout, output):
    candidate.check_checkout(checkout)
    result = json_file(private_path(output / 'complete.json'))
    if result.get('schema') != 1 or result.get('candidate') != asdict(candidate):
        raise ValueError('promotion evidence belongs to another candidate')
    native = msra_directory(result['native_path'])
    formal = private_path(result['formal_path'], directory=True)
    if not formal.is_relative_to(output):
        raise ValueError('formal output must remain inside its original promotion run')
    if attest_evidence(native, formal, candidate, checkout) != result.get('digests'):
        raise ValueError('promotion evidence changed since the full run')
    return result['digests']


def operator_readback(candidate, checkout, output, uid):
    """Read the original evidence as its owner, without the App's credentials."""
    if type(uid) is not int or uid <= 0 or uid == os.geteuid():
        raise ValueError('a separate non-root evidence owner is required')
    account = pwd.getpwuid(uid)
    if checkout.is_relative_to(ROOT) or ROOT.is_relative_to(checkout):
        raise ValueError('install the reviewed publisher outside the proof checkout')
    checked = subprocess.run(
        [sys.executable, '-E', '-s', '-m', 'tools.ci_promotion_acceptance', 'readback',
         '--checkout', str(checkout), '--output', str(output)], cwd=ROOT,
        user=uid, group=account.pw_gid, extra_groups=[], umask=0o077,
        env=dict(PATH='/usr/bin:/bin', HOME=account.pw_dir),
        input=json.dumps(asdict(candidate)), text=True, capture_output=True, timeout=120,
        check=True,
    )
    return json.loads(checked.stdout)


def check_runs(api, sha):
    found = api.request('GET', PREFIX + 'commits/' + oid(sha) +
                        '/check-runs?per_page=100&filter=all')
    if found['total_count'] > 100 or found['total_count'] != len(found['check_runs']):
        raise ValueError('incomplete GitHub check inventory')
    return found['check_runs']


def latest_check(rows, name, sha, app_id):
    matches = [row for row in rows if row.get('name') == name and row.get('head_sha') == sha
               and (row.get('app') or {}).get('id') == app_id]
    return max(matches, default=None, key=lambda row: row['id'])


def succeeded(row):
    return bool(row and row.get('status') == 'completed' and row.get('conclusion') == 'success')


def publish_check(api, directory, record, *, name, sha):
    """Persist creation intent first; if a reply is lost only reconcile it."""
    identity = receipts.digest(record)
    suffix = {GATE: 'gate', REFERENCE: 'reference', E2E: 'e2e'}[name]
    intent = directory / (identity + '-' + suffix + '.json')
    matches = [row for row in check_runs(api, sha) if row.get('external_id') == identity
               and row.get('name') == name and row.get('head_sha') == sha
               and (row.get('app') or {}).get('id') == api.app_id]
    if len(matches) > 1:
        raise ValueError('duplicate promotion GitHub check identity')
    if matches and succeeded(matches[0]):
        if json.loads(matches[0]['output']['summary']) != record:
            raise ValueError('existing promotion publication has different evidence')
        return
    title = '原 E2E 验收通过' if name == E2E else '完整 Rust 与 Stage1 验收通过'
    output = dict(title=title, summary=json.dumps(record, sort_keys=True))
    if matches:
        api.request('PATCH', PREFIX + 'check-runs/' + str(matches[0]['id']),
                    dict(status='completed', conclusion='success', output=output))
        return
    write_once(intent, dict(name=name, sha=sha, external_id=identity))
    observed = api.request('POST', PREFIX + 'check-runs', dict(
        name=name, head_sha=sha, external_id=identity, status='completed',
        conclusion='success', output=output))
    if observed.get('head_sha') != sha or observed.get('external_id') != identity:
        raise ValueError('created check does not match the reviewed promotion result')


def promotion_inputs(api, candidate, ordinary_app):
    ordinary = Checks(api).ordinary_receipts(candidate.head, ordinary_app,
                                            pull=candidate.pull, base=candidate.base)
    if ordinary is None:
        raise ValueError('ordinary checks pending, failed, absent or from the wrong SHA/App')
    row = latest_check(check_runs(api, candidate.head), E2E, candidate.head, api.app_id)
    if not succeeded(row):
        raise ValueError('existing E2E is pending, failed, absent or from the wrong SHA/App')
    expected = dict(schema=1, source=candidate.source, target='test', head=candidate.head,
                    base=candidate.base, tree=candidate.tree)
    if json.loads(row['output'].get('text') or 'null') != expected:
        raise ValueError('existing E2E is bound to a different source, base or tree')
    return ordinary, row['id']


def require_queue_policy(api, ruleset_id):
    if type(ruleset_id) is not int or ruleset_id <= 0 or api.app_id != GATE_APP:
        raise ValueError('exact approved test merge queue ruleset and App required')
    original = api.request('GET', PREFIX + 'branches/test/protection')
    classic = original['required_status_checks']
    required = {**{name: ACTIONS_APP for name in ORDINARY}, E2E: GATE_APP}
    rows = classic['checks']
    if (classic.get('strict') is not True or original.get('enforce_admins', {}).get('enabled') is not True
            or {row['context']: row['app_id'] for row in rows} != required
            or len(rows) != len(required)):
        raise ValueError('existing five native test checks and administrator enforcement are required')
    ruleset = api.request('GET', PREFIX + 'rulesets/' + str(ruleset_id))
    if (ruleset.get('id') != ruleset_id or ruleset.get('enforcement') != 'active'
            or ruleset.get('target') != 'branch' or ruleset.get('source_type') != 'Repository'
            or ruleset.get('source') != REPO.removeprefix('https://github.com/').removesuffix('.git')
            or ruleset.get('bypass_actors') != []
            or ruleset.get('conditions') != {'ref_name': {'include': [TARGET], 'exclude': []}}):
        raise ValueError('approved test queue ruleset is inactive, unbound or allows bypass')
    active = api.request('GET', PREFIX + 'rules/branches/test')
    configured = [rule for rule in active if rule.get('type') == 'merge_queue']
    status = [rule['parameters'] for rule in active if rule.get('type') == 'required_status_checks'
              and rule.get('ruleset_id') == ruleset_id]
    if (len(configured) != 1 or configured[0].get('ruleset_id') != ruleset_id or len(status) != 1
            or status[0].get('strict_required_status_checks_policy') is not True
            or status[0].get('required_status_checks') != [{'context': GATE, 'integration_id': GATE_APP}]):
        raise ValueError('test queue and native App-bound Stage1 check are not jointly active')
    parameters = configured[0]['parameters']
    for key, expected in (('merge_method', 'MERGE'), ('grouping_strategy', 'ALLGREEN'),
                          ('max_entries_to_build', 1), ('max_entries_to_merge', 1),
                          ('min_entries_to_merge', 1)):
        if parameters.get(key) != expected:
            raise ValueError('test queue must validate one candidate with all checks')


def evidence_record(candidate, digests, ordinary, e2e):
    return dict(schema=2, candidate=asdict(candidate), digests=digests,
                ordinary_app_id=ACTIONS_APP, ordinary_check_ids=ordinary, e2e_check_id=e2e)


def admit(candidate, checkout, output, *, api, source_client, proxy, directory, policy,
          readback=None):
    policy()
    candidate.observe(source_client=source_client, proxy=proxy)
    digests = (readback or read_stage)(candidate, checkout, output)
    admission = promotion_inputs(api, candidate, ACTIONS_APP)
    candidate.observe(source_client=source_client, proxy=proxy)
    if promotion_inputs(api, candidate, ACTIONS_APP) != admission:
        raise ValueError('ordinary or E2E check attempt changed during queue admission')
    policy()
    record = dict(evidence_record(candidate, digests, *admission), publication='queue-admission')
    # GitHub requires the same status names on the original PR and the queue.
    # The active no-bypass queue policy is what makes this source status safe.
    publish_check(api, directory, record, name=GATE, sha=candidate.head)


def queue_binding(candidate, group, lookup, commit):
    row = group.observe(lookup, commit)
    if any(getattr(candidate, k) != row[k] for k in ('pull', 'source', 'head', 'base', 'tree')):
        raise ValueError('live queue PR, base or tree differs from the original complete acceptance')
    return dict(**asdict(group), entry=row['entry'])


def publish(candidate, checkout, output, *, group, api, lookup, commit, directory, policy,
            readback=None):
    policy()
    binding = queue_binding(candidate, group, lookup, commit)
    digests = (readback or read_stage)(candidate, checkout, output)
    admission = promotion_inputs(api, candidate, ACTIONS_APP)
    record = evidence_record(candidate, digests, *admission)
    checks = check_runs(api, candidate.head)
    head = latest_check(checks, GATE, candidate.head, GATE_APP)
    admitted_record = dict(record, publication='queue-admission')
    if (not succeeded(head) or head.get('external_id') != receipts.digest(admitted_record)
            or json.loads(head['output']['summary']) != admitted_record):
        raise ValueError('original PR has no matching successful Stage1 queue admission')
    ordinary = Checks(api)
    queue_checks = ordinary.ordinary_receipts(group.sha, ACTIONS_APP)
    if queue_checks is None:
        raise ValueError('require all four current Actions checks to complete with success on the exact queue SHA')
    record.update(publication='merge-queue', queue=binding, queue_ordinary_check_ids=queue_checks)
    if queue_binding(candidate, group, lookup, commit) != binding:
        raise ValueError('queue entry changed before publication')
    policy()
    # GitHub still enforces the required Actions contexts independently. The
    # App additionally excludes skipped and neutral checks, which GitHub may accept.
    publish_check(api, directory, record, name=REFERENCE, sha=candidate.head)
    if ordinary.ordinary_receipts(group.sha, ACTIONS_APP) != queue_checks:
        raise ValueError('queue Actions checks changed before E2E publication')
    publish_check(api, directory, queue_e2e(record), name=E2E, sha=group.sha)
    if (queue_binding(candidate, group, lookup, commit) != binding
            or promotion_inputs(api, candidate, ACTIONS_APP) != admission
            or ordinary.ordinary_receipts(group.sha, ACTIONS_APP) != queue_checks):
        raise ValueError('queue, ordinary checks or E2E changed during publication')
    policy()
    publish_check(api, directory, record, name=GATE, sha=group.sha)


def queue_e2e(record):
    return dict(schema=2, candidate=record['candidate'], queue=record['queue'],
                e2e_check_id=record['e2e_check_id'])


class ReadAPI:
    def __init__(self, transport, token):
        self.transport = transport
        self.token = token

    def request(self, method, path):
        if method != 'GET' or not path.startswith(PREFIX):
            raise ValueError('push receipt may only read repository records')
        return self.transport(method, path, self.token)


def equivalent_push(event, api):
    """Only omit the duplicate workspace when GitHub shows the prior exact gate."""
    if (event.get('ref') != 'refs/heads/test'
            or event.get('repository', {}).get('full_name') != REPO.removeprefix(
                'https://github.com/').removesuffix('.git')
            or event.get('forced') is not False or event.get('deleted') is not False):
        return False
    after, before = oid(event['after']), oid(event['before'])
    commit = api.request('GET', PREFIX + 'git/commits/' + after)
    parents = [parent['sha'] for parent in commit['parents']]
    if commit['sha'] != after or len(parents) != 2 or parents[0] != before:
        return False
    head = oid(parents[1])
    reference = latest_check(check_runs(api, head), REFERENCE, head, GATE_APP)
    if not succeeded(reference):
        return False
    record = json.loads(reference['output']['summary'])
    candidate = Candidate(**record['candidate'])
    if (record.get('schema') != 2 or record.get('publication') != 'merge-queue'
            or set(record.get('queue_ordinary_check_ids', {})) != set(ORDINARY)
            or candidate.base != before or candidate.head != head
            or candidate.tree != commit['tree']['sha']
            or reference.get('external_id') != receipts.digest(record)):
        return False
    group = QueueGroup(**{key: record['queue'][key] for key in ('ref', 'sha', 'base')})
    if group.base != before:
        return False
    gate = latest_check(check_runs(api, group.sha), GATE, group.sha, GATE_APP)
    if (not succeeded(gate) or gate.get('external_id') != reference['external_id']
            or json.loads(gate['output']['summary']) != record):
        return False
    e2e = latest_check(check_runs(api, group.sha), E2E, group.sha, GATE_APP)
    return succeeded(e2e) and json.loads(e2e['output']['summary']) == queue_e2e(record)


def read_push_report(event):
    try:
        proxy = os.environ.get('https_proxy') or os.environ.get('HTTPS_PROXY')
        api = ReadAPI(Transport(proxy), os.environ['GH_TOKEN'])
        print('true' if equivalent_push(json_file(event), api) else 'false')
    except (APIError, ValueError, KeyError, TypeError, OSError):
        print('false')


def main():
    if len(sys.argv) == 3 and sys.argv[1] == 'reused-push':
        read_push_report(Path(sys.argv[2]))
        return
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('run', 'closeout', 'readback', 'admit', 'publish'))
    parser.add_argument('--candidate', type=Path, help='private reviewed Candidate JSON')
    parser.add_argument('--checkout', type=Path, default=ROOT)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--native', type=Path, help='original native seal to admit instead of rerunning')
    parser.add_argument('--resume-from', type=Path, help='latest interrupted promotion output')
    parser.add_argument('--config', type=Path, help='private publication settings, no credentials in checkout')
    parser.add_argument('--queue-ref')
    parser.add_argument('--queue-sha')
    args = parser.parse_args()
    if args.mode == 'publish':
        if not all((args.queue_ref, args.queue_sha)):
            parser.error('publishing requires the live merge queue ref and SHA')
    elif args.queue_ref is not None or args.queue_sha is not None:
        parser.error('queue ref and SHA belong to publish only')
    os.umask(0o077)
    if args.mode == 'readback':
        if any(x is not None for x in (args.candidate, args.config, args.native, args.resume_from)):
            parser.error('readback takes the approved candidate on stdin and no credentials')
        candidate = Candidate(**json.loads(sys.stdin.read(4096)))
        print(json.dumps(read_stage(candidate, args.checkout.resolve(strict=True),
                                    msra_directory(args.output)), sort_keys=True))
        return
    if args.candidate is None:
        parser.error('run, closeout and publication require a private candidate file')
    candidate = Candidate(**json_file(private_path(args.candidate)))
    checkout = args.checkout.resolve(strict=True)
    if args.mode == 'closeout':
        if any(x is not None for x in (args.config, args.native, args.resume_from)):
            parser.error('closeout uses only the original candidate, checkout and output')
        print(json.dumps(closeout_registered(candidate, checkout, args.output), sort_keys=True))
        return
    if args.mode == 'run':
        if args.config is not None:
            parser.error('proof execution does not take publication credentials')
        try:
            run_registered(candidate, checkout, args.output, native=args.native, resume_from=args.resume_from)
        except subprocess.CalledProcessError as error:
            # The native wrapper's 75 keeps its finished seal for retirement
            # resume. There is no promotion success until the proof also runs.
            return error.returncode
        return
    if args.config is None or args.native is not None or args.resume_from is not None or os.geteuid() != 0:
        parser.error('publication requires the installed protected App process and existing full evidence')
    config = json_file(private_path(args.config))
    if config.get('test_queue_approved') is not True or config.get('app_id') != GATE_APP:
        raise ValueError('test promotion publication is disabled until protected queue rollout approval')
    directory = private_path(config['state_root'], directory=True)
    credentials = Path(os.environ['CREDENTIALS_DIRECTORY'])
    transport = Transport(config['proxy'])
    clients = [GitHubApp(config['client_id'], credentials/'app-key', transport,
                         config['installation_id'], permissions=permissions)
               for permissions in (CHECK_PERMISSIONS, SOURCE_PERMISSIONS, PERMISSIONS)]
    for client in clients:
        if client.verify_installation()['app_id'] != config['app_id']:
            raise ValueError('publication installation differs from the protected branch App')
    with receipts.locked(directory):
        def policy():
            require_queue_policy(clients[2], config['test_queue_ruleset_id'])

        def readback(*values):
            return operator_readback(*values, config['evidence_uid'])

        if args.mode == 'admit':
            admit(candidate, checkout, args.output, api=clients[0], source_client=clients[1],
                  proxy=config['proxy'], directory=directory, policy=policy, readback=readback)
        else:
            group = QueueGroup(args.queue_ref, args.queue_sha, candidate.base)
            publish(candidate, checkout, args.output, group=group, api=clients[0],
                    lookup=graphql_lookup(transport, clients[1].access_token()),
                    commit=lambda sha: clients[1].request('GET', PREFIX + 'git/commits/' + sha),
                    directory=directory, policy=policy, readback=readback)


if __name__ == '__main__':
    raise SystemExit(main())
