"""Extract both coordinator callers and check the joint Inbox/decision Lean project."""
import argparse
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

from wiring import COORD, DECIDE, MODEL, MODULE, SERVICE, declaration, inspect_parent, nodes, require

ROOT = Path(__file__).resolve().parents[4]
BASE = 'd642a5f45253a716dbb8411a9942b8b9581dd5fb'
AREA = Path('verification/experiments/stage1-protocol')
PKG = AREA / 'coordinator-revocation'
CALLER = Path('controller-service/src/application/coordinator/lifecycle_intent.rs')
STATE = Path('controller-service/src/controller/lifecycle_inbox/model.rs')
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
GENERATED = Path('verification/aeneas/stage1-protocol/request/SandproofStage1Request')
SEED = PROOFS / 'CoordinatorRevocation.lean'


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
    (folder / (label + '.argv.json')).write_text(json.dumps(list(map(str, argv))) + '\n')
    with (folder / (label + '.log')).open('xb') as log:
        subprocess.run(argv, cwd=project, check=True, stdin=subprocess.DEVNULL,
                       stdout=log, stderr=subprocess.STDOUT)


def capture(source, evidence, tools, fixed, *, joint=False, refactored=False):
    evidence.mkdir(mode=0o700)
    os.environ['CARGO_TARGET_DIR'] = str(evidence / 'target')
    if joint:
        roots = [MODULE + '::proof_data_revocation', MODEL + '::proof_state',
                 *['crate::' + n for n in ('request_validate', 'replay_matches', 'accepted_validate', 'recorded_matches')]]
        include = ['sandproof_protocol', 'sandbox_controller_core', SERVICE]
        opaque = ['rusqlite::error::Error', 'sandproof_protocol::ContractError']
        retain = []
        manifest = AREA / 'inbox-record/Cargo.toml'
    else:
        roots = [COORD + '::ControllerApplication::record_lifecycle_intent']
        retain = [MODULE + '::*::record_lifecycle_intent']
        include = [*retain, COORD + '::ControllerApplication', COORD + '::Inner', MODEL + '::LifecycleWorkState',
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
    if not joint:
        inspect_parent(raw, refactored=refactored)
        return {'ir': sha(ir), 'caller': sha(source/CALLER)}
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
    for wrapper, actual in [(roots[0], DECIDE), (roots[1], MODEL+'::{impl}::state')]:
        caller = declaration(trans['fun_decls'], wrapper)
        callee = declaration(trans['fun_decls'], actual)
        invoked = list(nodes(caller['body']['Structured']['body'], 'Call'))
        require(len(invoked) == 1 and invoked[0]['call']['func']['Regular']['kind'] == {'Fun': callee['def_id']},
                'the forwarding root does not call the selected production function')
    for actual in (DECIDE, MODEL + '::{impl}::state'):
        decl = declaration(trans['fun_decls'], actual)
        require('Structured' in decl['body'] and any(f['def_id'] == decl['def_id'] and f['is_opaque'] is False and
                f['lean_file'] == 'SandproofStage1Request/Funs.lean' for f in functions), 'missing generated body: '+actual)
    return {'ir': sha(ir), 'mapping': sha(dest/'translation.json')}


def project_files(project, generated):
    for n in ('Funs.lean', 'Types.lean'):
        shutil.copy2(generated/'SandproofStage1Request'/n, project/GENERATED/n)
    for package, stem in [('inbox-state', 'FunsExternal'), ('inbox-lookup', 'TypesExternal')]:
        shutil.copy2(project/AREA/package/(stem+'.lean.in'), project/GENERATED/(stem+'.lean'))
    for package, stem in [('inbox-record', 'InboxRecordCloning'), ('inbox-state', 'InboxStateModel'),
                          ('inbox-state', 'InboxState'), ('coordinator-revocation', 'CoordinatorRevocation')]:
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
    report['original_parent'] = capture(old, args.output/'original-parent', args.tools, fixed)
    report['candidate_parent'] = capture(
        project, args.output/'candidate-parent', args.tools, fixed, refactored=True)
    for path, fragment in [(STATE, AREA/'inbox-state/roots.rs.in'), (CALLER, PKG/'root.rs.in')]:
        with (project/path).open('ab') as stream:
            stream.write(b'\n' + (project/fragment).read_bytes())
    report['joint'] = capture(project, args.output/'joint', args.tools, fixed, joint=True)
    project_files(project, args.output/'joint/generated')
    for link, target in [(project/'.lake/packages', args.packages), (project/'.tools/lean-mcp', args.tools/'lean-mcp')]:
        link.parent.mkdir(parents=True, exist_ok=True)
        link.symlink_to(target, target_is_directory=True)
    (project/'artifacts').mkdir(exist_ok=True)
    sys.path.insert(0, str(project/'tools'))
    import proof_batch
    files = sorted(proof_batch.closure(project, [str(SEED)], active_view='lakefile.toml'))
    require(all('_Template' not in f for f in files), 'unexpected external template')
    before = dependencies(project, args.packages)
    run(project, args.output, 'build', ['lake', 'build',
        'SandboxModel.Lifecycle.Protocol.CoordinatorRevocation'])
    checker = load(project/AREA/'formal.py')
    report['physical'] = checker.actual_files(project, files, args.output, os.environ, run)
    require(before == dependencies(project, args.packages), 'dependencies changed during checking')
    report.update(dependencies=before, complete=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ('tools', 'packages', 'lock', 'output'):
        parser.add_argument('--'+key, required=True, type=lambda path: Path(path).resolve())
    args = parser.parse_args()
    require(os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') == 'formal', 'formal role required')
    os.umask(0o077)
    os.sched_setaffinity(0, sorted(os.sched_getaffinity(0))[:4])
    os.environ.update(CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4', CARGO_INCREMENTAL='0',
                      LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536', PYTHONDONTWRITEBYTECODE='1')
    for opt in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        os.environ.pop(opt, None)
    args.output.mkdir(mode=0o700)
    report = {'complete': False, 'native_acceptance': False, 'gateway_verified': False}
    try:
        with args.lock.open('r+') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            check(args, report)
    finally:
        (args.output/'result.json').write_text(json.dumps(report, indent=2)+'\n')
