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
import subprocess
import sys

from tools import acceptance_receipts as receipts
from tools.ci_e2e_supervisor import PREFIX, REPO, Checks, git, is_ancestor
from tools.ci_github_app import APIError, CHECK_PERMISSIONS, SOURCE_PERMISSIONS, GitHubApp, Transport
from tools.system_acceptance import digest, json_file, private_path, write_once

GATE = 'Promotion / Stage1'
REFERENCE = 'Promotion / Stage1 evidence'
E2E = 'E2E / terminal-bench-ci'
GATE_APP = 5175756  # Existing protected E2E App
SCOPE = 'stage1-protocol'
ROOT = Path(__file__).resolve().parents[1]
BINARY = ['bash', 'scripts/python-entry.sh']
MOUNT = Path('/mnt')


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


def run_stage(candidate, checkout, output, *, native=None):
    """Under the usual msra lock and env, run Rust once then the original Lean entry."""
    candidate.check_checkout(checkout)
    output = msra_directory(output)
    original_native = msra_directory(native) if native else None
    native = original_native or output / 'native'
    temporary = output / 'tmp'
    temporary.mkdir(mode=0o700)
    cargo_target = output / 'cargo-target'
    cargo_target.mkdir(mode=0o700)
    env = dict(os.environ, SANDPROOF_NATIVE_EVIDENCE=str(native))
    env.update(SANDPROOF_PROOF_TOOL_ROOT=str(checkout/'.tools'),
               CARGO_TARGET_DIR=str(cargo_target), RUSTUP_AUTO_INSTALL='0')
    env.pop('PYTHONPATH', None)
    # An imported dotenv or launch shell must not leak a GitHub token into a driver.
    for name in ('CREDENTIALS_DIRECTORY', 'GH_TOKEN', 'GITHUB_TOKEN'):
        env.pop(name, None)
    if original_native is None:
        observer = env['SANDPROOF_NATIVE_OBSERVER']
        execution_commit = git('rev-parse', 'HEAD', cwd=checkout)
        with operator_lock(env, 'SANDPROOF_PROOF_HEAVY_LOCK',
                           'SANDPROOF_PROOF_HEAVY_LOCK_FD') as fd:
            native_env = dict(env, SANDPROOF_ACCEPTANCE_ROLE='native')
            native_env.pop('SANDPROOF_LOCK_FD', None)
            run_entry([*BINARY, 'tools/native_task.py', 'run', '--scope', SCOPE,
                       '--candidate', execution_commit, '--output', str(native), '--observer', observer],
                      log=output/'native.log', checkout=checkout,
                      env=native_env, pass_fds=(fd,))
    elif (native/'native-task.json').exists():
        # Older complete original seals predate managed retirement. A new
        # managed invocation with pending retirement must resume it, not rerun.
        retired = json_file(private_path(native/'native-task-retired.json'))
        if retired.get('returncode') != 0 or retired.get('remaining_build_bytes') != 0:
            raise ValueError('original native run does not have successful retirement')
    with operator_lock(env, 'SANDPROOF_LOCK_FILE', 'SANDPROOF_LOCK_FD') as fd:
        # The formal entry itself admits the original complete native seal
        # under current source, environment and tool rules, including functional.
        formal_env = dict(env, SANDPROOF_ACCEPTANCE_ROLE='formal', TMPDIR=str(temporary))
        formal_env.pop('SANDPROOF_PROOF_HEAVY_LOCK_FD', None)
        run_entry(['bash', 'scripts/verify-stage1-protocol.sh'], log=output/'formal.log',
                  checkout=checkout, env=formal_env,
                  pass_fds=(fd,))
        formal, = temporary.glob('sandproof-stage1-protocol.*/registered')
        candidate.check_checkout(checkout)
        bound = attest_evidence(native, formal, candidate, checkout)
        result = dict(schema=1, candidate=asdict(candidate), native_path=str(native),
                      formal_path=str(formal), digests=bound)
        write_once(output/'complete.json', result)
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


def publish(candidate, checkout, output, *, api, source_client, proxy, ordinary_app, directory,
            readback=None):
    candidate.observe(source_client=source_client, proxy=proxy)
    digests = (readback or read_stage)(candidate, checkout, output)
    ordinary, e2e = promotion_inputs(api, candidate, ordinary_app)
    record = dict(schema=1, candidate=asdict(candidate), digests=digests,
                  ordinary_app_id=ordinary_app, ordinary_check_ids=ordinary, e2e_check_id=e2e)
    candidate.observe(source_client=source_client, proxy=proxy)
    # Complete the informational pointer first so an immediate merge cannot
    # race a redundant workspace run before its evidence is discoverable.
    publish_check(api, directory, record, name=REFERENCE, sha=candidate.head)
    candidate.observe(source_client=source_client, proxy=proxy)
    # The source E2E checkout, test-merge parent identities, and both trees
    # were confirmed above. Project the original App result, without re-running E2E.
    publish_check(api, directory, dict(schema=1, candidate=asdict(candidate), e2e_check_id=e2e),
                  name=E2E, sha=candidate.merge)
    candidate.observe(source_client=source_client, proxy=proxy)
    if promotion_inputs(api, candidate, ordinary_app) != (ordinary, e2e):
        raise ValueError('ordinary or E2E check attempt changed during publication')
    publish_check(api, directory, record, name=GATE, sha=candidate.merge)


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
    if (record.get('schema') != 1 or candidate.base != before or candidate.head != head
            or candidate.tree != commit['tree']['sha']
            or reference.get('external_id') != receipts.digest(record)):
        return False
    gate = latest_check(check_runs(api, candidate.merge), GATE, candidate.merge, GATE_APP)
    if (not succeeded(gate) or gate.get('external_id') != reference['external_id']
            or json.loads(gate['output']['summary']) != record):
        return False
    e2e = latest_check(check_runs(api, candidate.merge), E2E, candidate.merge, GATE_APP)
    return succeeded(e2e) and json.loads(e2e['output']['summary']) == dict(
        schema=1, candidate=asdict(candidate), e2e_check_id=record['e2e_check_id'])


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
    parser.add_argument('mode', choices=('run', 'readback', 'publish'))
    parser.add_argument('--candidate', type=Path, help='private reviewed Candidate JSON')
    parser.add_argument('--checkout', type=Path, default=ROOT)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--native', type=Path, help='original native seal to admit instead of rerunning')
    parser.add_argument('--config', type=Path, help='private publication settings, no credentials in checkout')
    args = parser.parse_args()
    os.umask(0o077)
    if args.mode == 'readback':
        if args.candidate is not None or args.config is not None or args.native is not None:
            parser.error('readback takes the approved candidate on stdin and no credentials')
        candidate = Candidate(**json.loads(sys.stdin.read(4096)))
        print(json.dumps(read_stage(candidate, args.checkout.resolve(strict=True),
                                    msra_directory(args.output)), sort_keys=True))
        return
    if args.candidate is None:
        parser.error('run and publish require a private candidate file')
    candidate = Candidate(**json_file(private_path(args.candidate)))
    checkout = args.checkout.resolve(strict=True)
    if args.mode == 'run':
        if args.config is not None:
            parser.error('proof execution does not take publication credentials')
        try:
            run_stage(candidate, checkout, args.output, native=args.native)
        except subprocess.CalledProcessError as error:
            # The native wrapper's 75 keeps its finished seal for retirement
            # resume. There is no promotion success until the proof also runs.
            return error.returncode
        return
    if args.config is None or args.native is not None or os.geteuid() != 0:
        parser.error('publication requires the installed protected App process and existing full evidence')
    config = json_file(private_path(args.config))
    if config.get('test_cutover_approved') is not True or config.get('app_id') != GATE_APP:
        raise ValueError('test promotion status publication is disabled until protected rollout approval')
    directory = private_path(config['state_root'], directory=True)
    credentials = Path(os.environ['CREDENTIALS_DIRECTORY'])
    transport = Transport(config['proxy'])
    clients = [GitHubApp(config['client_id'], credentials/'app-key', transport,
                         config['installation_id'], permissions=permissions)
               for permissions in (CHECK_PERMISSIONS, SOURCE_PERMISSIONS)]
    for client in clients:
        if client.verify_installation()['app_id'] != config['app_id']:
            raise ValueError('publication installation differs from the protected branch App')
    with receipts.locked(directory):
        publish(candidate, checkout, args.output, api=clients[0], source_client=clients[1],
                proxy=config['proxy'], ordinary_app=config['ordinary_app_id'], directory=directory,
                readback=lambda *values: operator_readback(*values, config['evidence_uid']))


if __name__ == '__main__':
    raise SystemExit(main())
