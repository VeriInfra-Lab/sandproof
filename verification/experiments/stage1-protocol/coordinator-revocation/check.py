"""Extract both coordinator callers and check the joint Inbox/decision Lean project."""
import argparse
from contextlib import contextmanager
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile

import codec_reads
import codec_receipt
from guard_binding import check_guard
from journal_entry import CODEC, CONTROLLER, DATABASE, INBOX, PROVIDER, RECORD, RECORDING, TRANSACTION, check_entry, translated_entry
from journal_entry_regressions import check_entry_rejections
from journal_hook import check_joint_hook, check_journal_hook
from hook_regressions import check_parent_rejections, check_joint_rejection
from sender_binding import inspect_senders
from sender_binding_regressions import check_guard_rejections, check_rejections
from wiring import COORD, DECIDE, MODEL, MODULE, SERVICE, assignments_before, declaration, inspect_parent, nodes, origin, require

ROOT = Path(__file__).resolve().parents[4]
BASE = 'd642a5f45253a716dbb8411a9942b8b9581dd5fb'
AREA = Path('verification/experiments/stage1-protocol')
PKG = AREA / 'coordinator-revocation'
CALLER = Path('controller-service/src/application/coordinator/lifecycle_intent.rs')
STATE = Path('controller-service/src/controller/lifecycle_inbox/model.rs')
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
GENERATED = Path('verification/aeneas/stage1-protocol/request/SandproofStage1Request')
SEED = PROOFS / 'CoordinatorRevocation.lean'
TRANSACTION_PROVIDER = Path('verification/experiments/create-transaction-link/current')
RECORD_SOURCE = Path('controller-service/src/controller/lifecycle_inbox/recording.rs')
JOURNAL_SOURCE = RECORD_SOURCE.parent.with_suffix('.rs')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load(path):
    spec = importlib.util.spec_from_file_location(path.stem, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def git(root, *args):
    return subprocess.check_output(['git', *args], cwd=root, text=True).strip()


def unpack(ref, directory):
    directory.mkdir(mode=0o700)
    with subprocess.Popen(['git', 'archive', ref], cwd=ROOT, stdout=subprocess.PIPE) as process:
        with tarfile.open(fileobj=process.stdout, mode='r|') as archive:
            archive.extractall(directory, filter='data')
        require(process.wait() == 0, 'source archive failed')


def run(project, folder, label, argv):
    argv = list(map(str, argv))
    (folder / (label + '.argv.json')).write_text(json.dumps(argv) + '\n')
    logfile = folder / (label + '.log')
    with logfile.open('xb') as log:
        completed = subprocess.run(argv, cwd=project, check=True, stdin=subprocess.DEVNULL,
                                   stdout=log, stderr=subprocess.STDOUT)
    return dict(argv=argv, cwd=str(project), exit_code=completed.returncode, log_sha256=sha(logfile))


def capture(source, evidence, tools, fixed, *, joint=False, refactored=False):
    evidence.mkdir(mode=0o700)
    os.environ['CARGO_TARGET_DIR'] = str(evidence / 'target')
    if joint:
        roots = [MODULE + '::proof_data_revocation', MODEL + '::proof_state', MODULE + '::proof_never_crash',
                 MODEL + '::proof_find',
                 *['crate::' + n for n in ('request_validate', 'replay_matches', 'accepted_validate', 'recorded_matches')]]
        include = ['sandproof_protocol', 'sandbox_controller_core', SERVICE]
        opaque = ['rusqlite::error::Error', 'sandproof_protocol::ContractError']
        retain = []
        manifest = AREA / 'inbox-record/Cargo.toml'
    else:
        roots = [COORD + '::ControllerApplication::record_lifecycle_intent', PROVIDER]
        retain = [MODULE + '::*::record_lifecycle_intent']
        include = [*retain, INBOX+'::*::record_lifecycle_intent', CONTROLLER, DATABASE, RECORD, CODEC, PROVIDER, TRANSACTION,
                   RECORDING+'::RecordOutcome', COORD + '::*::revoke_data', 'sandbox_controller_core::ids::SandboxId',
                   SERVICE + '::NeverCrash', SERVICE + '::CrashInjector', SERVICE + '::CrashPoint',
                   SERVICE + '::*::hit', COORD + '::ControllerApplication', COORD + '::Inner', MODEL + '::LifecycleWorkState',
                   SERVICE + '::JournalError', 'sandproof_protocol::lifecycle::authority::AcceptedLifecycleIntent',
                   'sandproof_protocol::lifecycle::request::LifecycleRequest',
                   'sandproof_protocol::lifecycle::request::LifecycleOperation',
                   SERVICE + '::controller::internal_inbox::model::InternalLocalBinding']
        if refactored:
            include += [MODULE + '::DataRevocation', DECIDE]
        opaque = [SERVICE, 'sandbox_controller_core', 'sandproof_protocol', 'tokio', 'rusqlite', 'serde_json']
        manifest = 'controller-service/Cargo.toml'
    ir = evidence / 'actual.llbc'
    options = [arg for switch, items in [('--start-from', roots), ('--start-from-if-exists', retain),
                                         ('--include', include), ('--opaque', opaque)] for item in items for arg in (switch, item)]
    run(source, evidence, 'charon', [tools/'aeneas/charon/bin/charon', 'cargo', '--preset=aeneas', '--error-on-warnings',
        '--sysroot', 'default', *options, '--dest-file', ir, '--', '--lib', '--manifest-path', manifest, '--locked', '--offline',
                                     '--target', fixed['target']])
    raw = json.loads(ir.read_bytes())
    trans = raw['translated']
    opts = trans['options']
    require(raw['has_errors'] is False and [v['key'] for v in trans['target_information']] == [fixed['target']]
            and [opts[k] for k in ('start_from', 'start_from_if_exists', 'include', 'opaque')] == [roots, retain, include, opaque]
            and not any(opts[k] for k in ('exclude', 'skip_borrowck', 'no_typecheck', 'monomorphize')), 'unexpected extraction')
    view = load(source/AREA/'serde-binding/binding.py').Original(raw)
    if not joint:
        guard = check_guard(view)
        broadcast = inspect_parent(raw, refactored=refactored, guard=guard)
        senders = inspect_senders(view, broadcast)
        report = {'ir': sha(ir), 'caller': sha(source/CALLER), 'senders': senders, 'guard_local': guard,
                  'journal_entry': check_entry(view), 'journal_hook': check_journal_hook(view, source),
                  'coordinator': sha(source/'controller-service/src/application/coordinator.rs')}
        if refactored:
            senders['rejected_metadata'] = check_rejections(view, broadcast)
            report['rejected_guard_metadata'] = check_guard_rejections(view)
            report['rejected_hook_metadata'] = check_parent_rejections(view, source)
            report['rejected_entry_metadata'] = check_entry_rejections(view)
        return report
    load(source/AREA/'inbox-lookup/journal_binding.py').admit(raw)
    dest = evidence/'generated'
    run(source, evidence, 'aeneas', [tools/'aeneas/bin/aeneas', '-backend', 'lean', '-abort-on-error', '-warnings-as-errors',
        '-split-files', '-emit-json', '-namespace', 'SandproofStage1RequestRust', '-subdir', 'SandproofStage1Request',
                                     '-dest', dest, ir])
    mapping = json.loads((dest/'translation.json').read_bytes())
    functions = mapping['functions']
    external = {f['lean_name'] for f in functions if f['is_opaque']}
    require(external == {'Bool.Insts.CoreCmpPartialEqBool.ne', 'core.option.Option.Insts.CoreCmpPartialEqOption.eq',
                         'alloc.string.String.Insts.CoreCmpPartialEqString.eq', 'alloc.string.String.Insts.CoreCloneClone.clone',
                         'alloc.vec.Vec.is_empty'}, 'unexpected function binding')
    carriers = {t['lean_name'] for t in mapping['types']
                if t['lean_file'].endswith('TypesExternal_Template.lean')}
    require(carriers == {'core.io.error.Error', 'serde_json.error.Error', 'rusqlite.error.Error',
                         'sandproof_protocol.ContractError'}, 'unexpected type binding')
    for wrapper, actual, arity in [(MODULE+'::proof_data_revocation', DECIDE, 2),
                                   (MODEL+'::proof_state', MODEL+'::{impl}::state', 3),
                                   (MODEL+'::proof_find', MODEL+'::{impl}::find', 2)]:
        caller = declaration(trans['fun_decls'], wrapper)
        callee = declaration(trans['fun_decls'], actual)
        block = caller['body']['Structured']['body']
        call_stmt, = [s for s in block['statements'] if isinstance(s['kind'], dict) and 'Call' in s['kind']]
        call = call_stmt['kind']['Call']['call']
        at = block['statements'].index(call_stmt)
        for stmt in block['statements'][:at]:
            kind = stmt['kind']
            require(isinstance(kind, dict) and ('StorageLive' in kind or
                    ('Assign' in kind and kind['Assign'][0]['kind'].get('Local', 0) > arity)),
                    'forwarding root changes an argument or takes another path: '+wrapper)
        require(all('StorageDead' in s['kind'] for s in block['statements'][at+1:-1]),
                'forwarding root changes its result: '+wrapper)
        require(len(list(nodes(block, 'Call'))) == 1 and len(caller['signature']['inputs']) == arity
                and call['func']['Regular']['kind'] == {'Fun': callee['def_id']}
                and call['dest']['kind'] == {'Local': 0} and block['statements'][-1]['kind'] == 'Return'
                and [origin(arg['Move'], assignments_before(block, call_stmt)) for arg in call['args']]
                == [(i,) for i in range(1, arity + 1)], 'forwarding root changed arguments, result or callee: '+wrapper)
        require('Structured' in callee['body'] and any(f['def_id'] == callee['def_id'] and f['is_opaque'] is False and
                f['lean_file'] == 'SandproofStage1Request/Funs.lean' for f in functions), 'missing generated body: '+actual)
    return {'ir': sha(ir), 'mapping': sha(dest/'translation.json'),
            'journal_hook': check_joint_hook(view, mapping, source),
            'rejected_hook_metadata': check_joint_rejection(view, mapping, source)}


@contextmanager
def transaction_roots(source, *, public=False):
    provider = source/TRANSACTION_PROVIDER
    profile = json.loads((provider/'caller-profile.json').read_bytes())
    root = provider/'caller-root.rs.in'
    path = source/RECORD_SOURCE
    require(profile['roots'] == [PROVIDER] and profile['append_only'] ==
            {'path': str(RECORD_SOURCE), 'sha256': sha(root)}, 'unexpected transaction proof root')
    original = path.read_bytes()
    inputs = dict(root_sha256=sha(root), recording_sha256=sha(path), journal_sha256=sha(source/JOURNAL_SOURCE))
    fragment = (source/PKG/'public-journal-root.rs.in').read_bytes() if public else b''
    try:
        path.write_bytes(original+root.read_bytes()+fragment)
        yield profile, inputs
    finally:
        path.write_bytes(original)


def capture_parent(source, evidence, tools, fixed, *, refactored=False):
    with transaction_roots(source) as (_, inputs):
        result = capture(source, evidence, tools, fixed, refactored=refactored)
    result['journal_entry'].update(inputs)
    return result


def capture_public(source, folder, tools, fixed, *, include_lease=False):
    folder.mkdir(mode=0o700)
    os.environ['CARGO_TARGET_DIR'] = str(folder/'target')
    ir = folder/'actual.llbc'
    wrapper = RECORDING+'::proof_public_journal'
    with transaction_roots(source, public=True) as (profile, inputs):
        profile['roots'].append(wrapper)
        profile['include'].extend([INBOX+'::*::record_lifecycle_intent', CONTROLLER, DATABASE, CODEC, wrapper])
        if include_lease:
            graphic = SERVICE+'::capture::StandardGraphicBytes'
            profile['include'].extend([INBOX+'::'+name for name in (
                'native_retirement::StandardRetirementEncoding', 'model::StandardTerminalEncoding',
                'renewal_confirmation::StandardRenewalEncoding', 'unload_resolution::StandardUnloadEncoding')])
            profile['include'].extend([graphic, SERVICE+'::capture::{impl '+SERVICE+'::capture::GraphicBytes for '+
                                       graphic+'}::all_graphic', SERVICE+'::controller::ControllerLease'])
        options = [arg for k in ('include', 'opaque', 'exclude', 'roots') for value in profile[k]
                   for arg in ('--start-from' if k == 'roots' else '--'+k, value)]
        run(source, folder, 'charon', [tools/'aeneas/charon/bin/charon', 'cargo', '--preset=aeneas', '--error-on-warnings',
            '--sysroot', 'default', *options, '--dest-file', ir, '--', '--manifest-path', 'controller-service/Cargo.toml',
            '--lib', '--locked', '--offline', '--target', fixed['target']])
    raw = json.loads(ir.read_bytes())
    trans = raw['translated']
    opts = trans['options']
    require(raw['has_errors'] is False and [t['key'] for t in trans['target_information']] == [fixed['target']]
            and opts['start_from'] == profile['roots'] and all(opts[k] == profile[k] for k in ('include', 'opaque', 'exclude'))
            and not any(opts[k] for k in ('start_from_if_exists', 'skip_borrowck', 'no_typecheck', 'monomorphize')),
            'unexpected public journal extraction')
    view = load(source/AREA/'serde-binding/binding.py').Original(raw)
    dest = folder/'generated'
    run(source, folder, 'aeneas', [tools/'aeneas/bin/aeneas', '-backend', 'lean', '-abort-on-error', '-warnings-as-errors',
        '-split-files', '-emit-json', '-namespace', 'SandproofActualRecordRust', '-subdir', 'SandproofActualRecord',
        '-dest', dest, ir])
    mapping = json.loads((dest/'translation.json').read_bytes())
    generated = translated_entry(view, mapping, wrapper)
    files = ['translation.json', 'SandproofActualRecord/Funs.lean', 'SandproofActualRecord/Types.lean',
             'SandproofActualRecord/FunsExternal_Template.lean', 'SandproofActualRecord/TypesExternal_Template.lean']
    return {'ir': sha(ir), 'journal_entry': check_entry(view) | inputs, 'translated': generated,
            'generated': {n: sha(dest/n) for n in files}, 'external': {
                k: sorted(row['lean_name'] for row in mapping[k] if row['lean_file'].endswith('External_Template.lean'))
                for k in ('functions', 'types')}, 'kernel_checked': False}


def project_files(project, generated):
    for n in ('Funs.lean', 'Types.lean'):
        shutil.copy2(generated/'SandproofStage1Request'/n, project/GENERATED/n)
    for package, stem in [('inbox-state', 'FunsExternal'), ('inbox-lookup', 'TypesExternal')]:
        shutil.copy2(project/AREA/package/(stem+'.lean.in'), project/GENERATED/(stem+'.lean'))
    for package, stem in [('inbox-record', 'InboxRecordCloning'), ('inbox-state', 'InboxStateModel'),
                          ('inbox-state', 'InboxState'), ('inbox-state', 'InboxStateValues'),
                          ('inbox-lookup', 'InboxLookupModel'),
                          ('inbox-lookup', 'InboxLookupSupport'), ('inbox-lookup', 'InboxLookupValues'),
                          ('inbox-lookup', 'InboxLookup'), ('inbox-position', 'InboxPositionLookup'),
                          ('coordinator-revocation', 'CoordinatorRevocation')]:
        shutil.copy2(project/AREA/package/(stem+'.lean.in'), project/PROOFS/(stem+'.lean'))


def dependencies(project, packages):
    revisions = {}
    for pkg in json.loads((project/'lake-manifest.json').read_bytes())['packages']:
        where = packages / pkg['name']
        actual = git(where, 'rev-parse', 'HEAD')
        require(actual == pkg['rev'] and not git(where, 'status', '--porcelain', '--untracked-files=no'),
                'fixed Lean package differs: ' + pkg['name'])
        revisions[pkg['name']] = actual
    return revisions


def check_decoders(current, args, fixed):
    folder = args.output/'codec-read'
    folder.mkdir(mode=0o700)
    project = folder/'project'
    unpack(current, project)
    original = load(project/AREA/'serde-binding/binding.py').Original
    public = original(json.loads((args.output/'public-journal/actual.llbc').read_bytes()))
    result = codec_reads.capture(project, folder, args.tools, fixed, public, original, run)
    files = codec_reads.materialize(project, folder, load(project/'tools/proof_batch.py').closure)
    (project/'.lake').mkdir(exist_ok=True)
    (project/'.lake/packages').symlink_to(args.packages, target_is_directory=True)
    (project/'.tools').symlink_to(args.tools, target_is_directory=True)
    (project/'artifacts').mkdir(exist_ok=True)
    run(project, folder, 'build', ['lake', 'build', codec_receipt.ROOT])
    commands = []

    def kernel(project, output, label, argv):
        commands.append(run(project, output, label, argv))

    checker = load(project/AREA/'formal.py')
    result['actual'] = checker.actual_files(project, files, folder, os.environ, kernel)
    result['kernel_commands'] = commands
    codec_receipt.check_page(project, result)
    return result


def check(args, report):
    fixed = json.loads((ROOT/'verification/foundations-environment.json').read_bytes())
    for n, path in [('charon', 'aeneas/charon/bin/charon'), ('aeneas', 'aeneas/bin/aeneas')]:
        require(sha(args.tools/path) == fixed[n]['binary_sha256'], 'fixed tool differs: '+n)
    current = git(ROOT, 'rev-parse', 'HEAD')
    require(not git(ROOT, 'status', '--porcelain', '--untracked-files=normal'),
            'commit the candidate before checking')
    subprocess.run(['git', 'merge-base', '--is-ancestor', BASE, current], cwd=ROOT, check=True)
    report.update(candidate=current, original=BASE, tree=git(ROOT, 'rev-parse', current+'^{tree}'),
                  tools={n: fixed[n]['binary_sha256'] for n in ('charon', 'aeneas')})
    old, project = args.output/'original', args.output/'project'
    unpack(BASE, old)
    unpack(current, project)
    report['original_parent'] = capture_parent(old, args.output/'original-parent', args.tools, fixed)
    report['candidate_parent'] = capture_parent(
        project, args.output/'candidate-parent', args.tools, fixed, refactored=True)
    report['public_journal'] = capture_public(project, args.output/'public-journal', args.tools, fixed)
    require(report['candidate_parent']['journal_entry'] == report['public_journal']['journal_entry'],
            'public translation differs from the real coordinator journal capture')
    before = dependencies(project, args.packages)
    report['codec_read'] = check_decoders(current, args, fixed)
    for path, fragment in [(STATE, AREA/'inbox-state/roots.rs.in'), (STATE, AREA/'inbox-lookup/extract-root.rs.in'),
                           (CALLER, PKG/'root.rs.in')]:
        with (project/path).open('ab') as stream:
            stream.write(b'\n' + (project/fragment).read_bytes())
    report['joint'] = capture(project, args.output/'joint', args.tools, fixed, joint=True)
    require(report['candidate_parent']['journal_hook'] == report['joint']['journal_hook'],
            'coordinator and Lean selected different hook implementations')
    project_files(project, args.output/'joint/generated')
    for link, target in [(project/'.lake/packages', args.packages), (project/'.tools/lean-mcp', args.tools/'lean-mcp')]:
        link.parent.mkdir(parents=True, exist_ok=True)
        link.symlink_to(target, target_is_directory=True)
    (project/'artifacts').mkdir(exist_ok=True)
    sys.path.insert(0, str(project/'tools'))
    import proof_batch
    files = sorted(proof_batch.closure(project, [str(SEED)], active_view='lakefile.toml'))
    require(all('_Template' not in f for f in files), 'unexpected external template')
    run(project, args.output, 'build', ['lake', 'build',
        'SandboxModel.Lifecycle.Protocol.CoordinatorRevocation'])
    checker = load(project/AREA/'formal.py')
    commands = []

    def kernel(project, folder, label, argv):
        commands.append(run(project, folder, label, argv))

    report['physical'] = checker.actual_files(project, files, args.output, os.environ, kernel)
    report['kernel_commands'] = commands
    require(before == dependencies(project, args.packages), 'dependencies changed during checking')
    report.update(dependencies=before, complete=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ('tools', 'packages', 'lock', 'output'):
        parser.add_argument('--'+key, required=True, type=lambda path: Path(path).resolve())
    parser.add_argument('--lock-fd', type=int)
    args = parser.parse_args()
    require(os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') == 'formal', 'formal role required')
    os.umask(0o077)
    os.sched_setaffinity(0, sorted(os.sched_getaffinity(0))[:4])
    os.environ.update(CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4', CARGO_INCREMENTAL='0',
                      LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536', PYTHONDONTWRITEBYTECODE='1')
    for opt in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        os.environ.pop(opt, None)
    args.output.mkdir(mode=0o700)
    report = {'kind': 'coordinator-revocation', 'complete': False, 'native_acceptance': False, 'gateway_verified': False}
    try:
        if args.lock_fd is not None or 'SANDPROOF_LOCK_FD' in os.environ:
            fd, identity = load(ROOT/AREA/'registered.py').inherited_lock()
            require(args.lock_fd in (None, fd) and args.lock.samefile(identity['path']),
                    'coordinator lock differs from the inherited batch lock')
            check(args, report)
        else:
            with args.lock.open('r+') as lock:
                fcntl.flock(lock, fcntl.LOCK_EX)
                check(args, report)
    finally:
        (args.output/'result.json').write_text(json.dumps(report, indent=2)+'\n')
