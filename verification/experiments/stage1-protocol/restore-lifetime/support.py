"""Exact current-source, generated-view and import admission for restore lifetime."""
import importlib.util
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[4]
HERE = Path(__file__).resolve().parent
BASE = '9c50da7cc9391518514f14405e6c779e5c5cbe2c'
PROOFS = Path('SandboxModel/Lifecycle/Protocol/RestoreLifetime')
LIBRARY = Path('verification/aeneas/stage1-protocol/restore-lifetime')
INCLUDE = ['sandproof_protocol', 'sandbox_controller_core', 'sandproof_controller_service']
START = ['crate::controller::lifecycle_inbox::source_work::restore::continued_exit::lifetime_matches']
CASES = ('correct', 'omit-maximum-field', 'allow-expired-end')
COMMON = {'Spec': 'Spec.lean.in', 'Witnesses': 'Witnesses.lean.in'}
SPECIFIC = {
    'correct': {'Core': 'Core.lean.in', 'Boundaries': 'Boundaries.lean.in'},
    'omit-maximum-field': {'Negative': 'NegativeMaximum.lean.in'},
    'allow-expired-end': {'Negative': 'NegativeExpiry.lean.in'},
}


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


native = module('restore_lifetime_native_provider', HERE / 'native.py')
mapping = module('restore_lifetime_fixed_mapping', HERE.parent / 'formal_runtime.py')
resolver = module('restore_lifetime_original_closure', ROOT / 'tools/proof_batch.py')
checker = module('restore_lifetime_original_physical_files', HERE.parent / 'formal.py')
admission = module('restore_lifetime_existing_error_boundary', HERE.parent / 'inbox-load/admission.py')
sha, require = native.sha, native.require


def mode(path):
    return '100755' if path.stat().st_mode & 0o111 else '100644'


def source_guard():
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    require(subprocess.run(['git', 'merge-base', '--is-ancestor', BASE, head], cwd=ROOT,
                           check=False).returncode == 0, 'source is not based on fixed integration')
    # Frozen Rust/provider hashes and the complete recapture bind the current
    # implementation. A later coordinated commit may also contain other proof
    # packages; reject only unrelated dirty edits in this working checkout.
    names = subprocess.check_output(['git', 'diff', '--name-only', '-z', 'HEAD'], cwd=ROOT).decode().split('\0')
    names += subprocess.check_output(['git', 'ls-files', '--others', '--exclude-standard', '-z'],
                                     cwd=ROOT).decode().split('\0')
    require(all(name.startswith((str(HERE.relative_to(ROOT)) + '/', str(LIBRARY) + '/')) or
                name == 'docs/formal/03-protocol-inbox.md' for name in filter(None, names)),
            'unowned source differs from fixed integration')
    return head


def inventory():
    data = json.loads((HERE / 'inventory.json').read_text())
    require(data['source_base'] == BASE and tuple(data['cases']) == CASES,
            'fixed restore lifetime scope differs')
    require(admission.OPAQUE == ['rusqlite::error::Error'], 'existing error boundary differs')
    for section in ('inputs', 'providers', 'templates', 'drivers'):
        for name, digest in data[section].items():
            path = ROOT / name
            require(path.is_file() and not path.is_symlink() and sha(path) == digest,
                    'restore lifetime input differs: ' + name)
    for case in CASES:
        for name, digest in data['cases'][case]['generated'].items():
            require(sha(ROOT / LIBRARY / case / 'SandproofRestoreValidation' / name) == digest,
                    'literal original generated file differs')
    return data


def copy_source(project, inputs):
    project.mkdir(parents=True, mode=0o700)
    for name, digest in inputs.items():
        path = project / name
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / name, path)
        require(sha(path) == digest, 'source changed while copying')
    (project / '.tools').symlink_to(mapping.tool_root(ROOT), target_is_directory=True)
    (project / '.lake').mkdir()
    (project / '.lake/packages').symlink_to(mapping.package_root(ROOT), target_is_directory=True)
    (project / 'artifacts/extract-target').mkdir(parents=True)


def mutate(project, case):
    if case == 'correct':
        return None
    before, after = native.MUTATIONS[case]
    path = project / native.SOURCE
    original = path.read_bytes()
    text = original.decode()
    require(text.count(before) == 1, 'actual Rust mutation site changed')
    path.write_text(text.replace(before, after))
    require(path.read_bytes() != original, 'actual Rust mutation did not occur')
    return dict(source=native.SOURCE, before=before, after=after,
                original_sha256=hashlib.sha256(original).hexdigest(), mutated_sha256=sha(path))


def capture(project, output, command):
    fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_text())
    for name, suffix in [('charon', 'charon/bin/charon'), ('aeneas', 'bin/aeneas')]:
        require(sha(ROOT / '.tools/aeneas' / suffix) == fixed[name]['binary_sha256'],
                'fixed proof tool changed')
    virtual = mapping.VIRTUAL
    wrapper = ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
               '--tmpfs', '/tmp', '--tmpfs', '/home', '--ro-bind', str(Path.home()), str(Path.home()),
               '--bind', str(project), str(virtual), '--chdir', str(virtual), '--setenv',
               'CARGO_TARGET_DIR', str(virtual / 'artifacts/extract-target'), '--setenv',
               'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4', '--unsetenv',
               'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--']
    args = [str(virtual / '.tools/aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
            '--error-on-warnings', '--sysroot', 'default']
    for flag, values in [('--include', INCLUDE), ('--start-from', START), ('--opaque', admission.OPAQUE)]:
        for value in values:
            args += [flag, value]
    ir = project / 'artifacts/current-restore-lifetime.llbc'
    args += ['--dest-file', str(virtual / 'artifacts' / ir.name), '--', '--manifest-path',
             'controller-service/Cargo.toml', '--lib', '--locked', '--offline', '--target', fixed['target']]
    command(project, output, 'charon', wrapper + args)
    raw = json.loads(ir.read_text())
    options = raw['translated']['options']
    require(raw['has_errors'] is False and options['include'] == INCLUDE and options['start_from'] == START and
            options['opaque'] == admission.OPAQUE and not options['exclude'] and not options['skip_borrowck'] and
            not options['no_typecheck'] and not options['monomorphize'] and not options['remove_unused_clauses'],
            'incomplete or weakened capture')
    expected = [{'Ident': [name, 0]} for name in
                ('sandproof_controller_service', 'controller', 'lifecycle_inbox', 'source_work', 'restore',
                 'continued_exit', 'lifetime_matches')]
    roots = [function for function in raw['translated']['fun_decls'] if function and
             function['item_meta']['name'] == expected]
    require(len(roots) == 1 and roots[0]['item_meta']['opacity'] == 'Transparent' and
            set(roots[0]['body']) == {'Structured'}, 'original work root is missing or opaque')
    require({value['key'] for value in raw['translated']['target_information']} == {fixed['target']},
            'capture uses a different target')
    return ir, dict(path=str(ir), sha256=sha(ir), root_definition=roots[0]['def_id'],
                    requested_external_type_opacity=admission.OPAQUE, business_opacity_added=False)


def render(project, case, generated, data):
    installed = {}
    target = LIBRARY / case / 'SandproofRestoreValidation'
    for name, digest in data['cases'][case]['emitted'].items():
        require(sha(generated / 'SandproofRestoreValidation' / name) == digest,
                'original emitted declarations or bodies differ')
    for name, digest in data['cases'][case]['generated'].items():
        path = generated / 'SandproofRestoreValidation' / name
        require(sha(path) == digest, 'actual generated body differs from original tool output')
        destination = project / target / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, destination)
        installed[str(target / name)] = sha(destination)
    for name, template in {**COMMON, **SPECIFIC[case]}.items():
        destination = project / PROOFS / (name + '.lean')
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(HERE / template, destination)
        installed[str(PROOFS / (name + '.lean'))] = sha(destination)
    lake = project / 'lakefile.toml'
    correct_stanza = ('[[lean_lib]]\nname = "SandproofRestoreValidation"\nsrcDir = "' +
                      str(LIBRARY / 'correct') + '"\n')
    selected_stanza = ('[[lean_lib]]\nname = "SandproofRestoreValidation"\nsrcDir = "' +
                       str(LIBRARY / case) + '"\n')
    text = lake.read_text()
    if 'name = "SandproofRestoreValidation"' in text:
        require(text.count(correct_stanza) == 1, 'different existing restore lifetime Lake view')
        text = text.replace(correct_stanza, selected_stanza)
    else:
        text += '\n' + selected_stanza
    lake.write_text(text)
    installed['lakefile.toml'] = sha(lake)
    seeds = [name for name in installed if name.endswith('.lean')]
    files = sorted(resolver.closure(project, seeds, active_view='lakefile.toml'))
    audits = [name for file in files for name in re.findall(
        r'^#audit_controller_proof (\S+)$', (project / file).read_text(), re.M)]
    require(len(audits) == len(set(audits)), 'duplicate current-view audits')
    if case != 'correct':
        require(not set(files) & {str(PROOFS / 'Core.lean'), str(PROOFS / 'Boundaries.lean')},
                'wrong environment imports a correct business theorem')
    for file in files:
        require('_Template' not in file and not re.search(r'^\s*(?:axiom|sorry|admit)\b',
                (project / file).read_text(), re.M), 'unproved external template or placeholder imported')
    return installed, files, audits
