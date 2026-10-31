"""Finite V3 wiring around the original drivers and shared native receipts.

The existing retained component and all original V0 targets remain separate.
This module neither implements a proof nor issues a shared native seal.
"""
import argparse
import ast
import copy
import fcntl
from concurrent.futures import FIRST_COMPLETED, ThreadPoolExecutor, wait
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[3]
HERE = Path('verification/experiments/stage1-protocol')
sys.path.insert(0, str(ROOT))

PROFILES = ('full', 'create-delivery')
CREATE_SCOPE = 'stage1-create-delivery'
# Fixed published package order. This is a complete delivery profile, not an
# operator-supplied subset of the full V3 obligations.
CREATE_CASES = {
    'create-selection': ('candidate', 'wrong-terminal', 'wrong-refusal',
                         'wrong-version', 'wrong-local', 'wrong-deadline'),
    'create-owner-successor': ('candidate', 'wrong-exclusion', 'wrong-sandbox-field',
                               'wrong-high-water', 'wrong-overflow'),
    'adopted-create': ('owners', 'selection'),
    'create-install': ('actual', 'wrong-sandbox', 'wrong-operation', 'wrong-high-water', 'wrong-frame'),
    'create-owner-check': ('actual', 'omit-local', 'skip-first-conflict', 'exclude-owner', 'retag-exhaustion'),
    'create-load-capacity': ('actual', 'wrong-owner-limit'),
    'create-high-water': ('actual', 'wrong-seed', 'wrong-write'),
    'create-deadline': ('actual', 'unchecked-cast', 'wrong-error'),
    'create-attachment-flow': ('actual', 'skip-save', 'eager-world'),
    'create-fixed-identity': ('actual', 'empty-namespace', 'truncate-number'),
    'create-lifetime-sql': ('actual', 'zero-deadline', 'empty-sandbox'),
    'create-standard-adapter': ('actual', 'default-world'),
}
CREATE_KINDS = {
    name: 'actual-'+name+'-development-batch' for name in CREATE_CASES
}
CREATE_KINDS.update({
    'adopted-create': 'adopted-v3-create-positive-correspondence-batch',
    'create-attachment-flow': 'actual-create-complete-flow-development',
    'create-fixed-identity': 'actual-fixed-identity-development-batch',
    'create-lifetime-sql': 'actual-lifetime-sql-development-batch',
    'create-standard-adapter': 'actual-standard-adapter-development-batch',
})
CREATE_NATIVE_COUNTS = dict(zip(CREATE_CASES, (9, 9, 28, 4, 12, 3, 3, 3, 6, 5, 6, 3)))
CREATE_RESTORED = {
    name: ('restored-candidate' if name in ('create-selection', 'create-owner-successor')
           else 'restored-actual' if name in ('adopted-create', 'create-install', 'create-owner-check',
                                             'create-fixed-identity', 'create-lifetime-sql') else 'restored')
    for name in CREATE_CASES
}
CREATE_ADOPTED_MUTANTS = (
    'owner-wrong-owner-guard', 'owner-wrong-world-maximum', 'owner-wrong-high-water', 'owner-wrong-overflow',
    'selection-wrong-terminal', 'selection-wrong-refusal', 'selection-wrong-version',
    'selection-wrong-local', 'selection-wrong-deadline',
    'successor-wrong-exclusion', 'successor-wrong-sandbox-field',
    'successor-wrong-high-water', 'successor-wrong-overflow',
)
CREATE_NATIVE_DRIVERS = tuple(('stage1-'+name, str(HERE/name/'native.py'), False)
                              for name in CREATE_CASES)


def check_profile(profile):
    if profile not in PROFILES:
        raise ValueError('unknown fixed Stage1 profile: '+str(profile))
    return profile


def native_scope(profile):
    return CREATE_SCOPE if check_profile(profile) == 'create-delivery' else 'stage1-protocol'


def _create_native_plan(name, data):
    cases = (('actual', *CREATE_ADOPTED_MUTANTS) if name == 'adopted-create' else CREATE_CASES[name])
    expected = (*cases, CREATE_RESTORED[name])
    if name in ('create-selection', 'create-owner-successor'):
        original = data['native_plan']
        if set(original['expected_failures']) != set(cases[1:]):
            raise ValueError('missing/extra Create native rejection case: '+name)
        result = {case: dict(test_count=original['test_count'],
                            expected_failures=original['expected_failures'].get(case, [])) for case in expected}
    else:
        result = data['native_variants']
    if tuple(result) != expected:
        raise ValueError('missing, extra or reordered Create native variant: '+name)
    for case, row in result.items():
        count = (10 if case.startswith('owner-') else 9) if name == 'adopted-create' and case in CREATE_ADOPTED_MUTANTS else CREATE_NATIVE_COUNTS[name]
        failures = row.get('expected_failures')
        correct = case in (cases[0], CREATE_RESTORED[name])
        if (type(row.get('test_count')) is not int or row['test_count'] != count
                or not isinstance(failures, list) or any(not isinstance(n, str) for n in failures)
                or len(failures) != len(set(failures)) or len(failures) > count
                or bool(failures) != (not correct)):
            raise ValueError('Create native assertion inventory changed: '+name+':'+case)
    return result


def create_profile(root=ROOT):
    """Bind the fixed 12 packages and every declared correct/mutant view."""
    if len(CREATE_CASES) != 12 or sum(map(len, CREATE_CASES.values())) != 42:
        raise ValueError('Create delivery must retain all 12 packages and 42 formal views')
    packages = []
    for name, cases in CREATE_CASES.items():
        path = root/HERE/name/'inventory.json'
        data = _read(path)
        views = data['views'] if name == 'adopted-create' else data['variants']
        if tuple(views) != cases:
            raise ValueError('Create formal view inventory changed: '+name)
        native = _create_native_plan(name, data)
        packages.append(dict(name=name, native_label='stage1-'+name,
            inventory_sha256=_sha(path), formal_cases=list(cases),
            native_views={case: dict(test_count=row['test_count'],
                expected_failures=row['expected_failures'],
                expected_clippy_errors=row.get('expected_clippy_errors', [])) for case, row in native.items()}))
    if sum(len(row['native_views']) for row in packages) != 66:
        raise ValueError('Create delivery must retain all 66 native views')
    return dict(profile='create-delivery', scope=CREATE_SCOPE, packages=packages,
                formal_view_count=42, native_view_count=66, whole_v3=False)


# The inbox labels are the exact labels consumed by the existing formal.py.
TYPE_CASES = ('result-confirmation', 'observation-bindings', 'resume-observation',
              'unload-observation', 'access-identity', 'renewal-confirmation',
              'create-observation')
INBOX_CASES = {
    'inbox-lookup': ('correct', 'key-only'),
    'inbox-admission': ('correct', 'skip-node', 'skip-envelope'),
    'inbox-record': ('correct', 'skip-retention-window', 'wrong-recorded-version'),
    'inbox-append': ('correct', 'skip-sdk-gate', 'skip-prior-capture'),
    'inbox-record-total': ('correct', 'skip-sdk-gate', 'skip-prior-capture'),
    'inbox-state': ('correct', 'always-current', 'drop-operation'),
    'inbox-position': ('correct', 'always-current', 'drop-operation'),
    'inbox-lineage': ('correct', 'advance-unattached-resume', 'repeat-capture', 'skip-final-incarnation'),
    'inbox-retained': ('correct', 'reverse-compaction-bound', 'reverse-confirmation-bound', 'zero-version-adjacency'),
    'inbox-lifetimes': ('correct', 'skip-predecessor', 'keep-gap-predecessor', 'skip-final-deadline'),
    'inbox-renewal-session': ('correct', 'key-only-checkpoint', 'ignore-retained-frontier', 'ignore-unapplied-sdk'),
    'inbox-unload-resolution': ('correct', 'ignore-snapshot-phase', 'ignore-capture-operation', 'ignore-result-maximum'),
    'inbox-entry-validation': ('correct', 'duplicate-cap', 'key-sequence-only', 'requested-deadline-only'),
    'inbox-retired-identities': ('correct', 'key-sequence-only', 'ignore-active-conflicts', 'ignore-prior-session'),
    'inbox-origin-selection': ('correct', 'skip-resume', 'require-local-operation', 'return-first-slot'),
    'inbox-config': ('correct', 'halve-options-reservation', 'omit-manifest-copy', 'omit-global-reserve'),
    'inbox-config-validation': ('correct', 'host-version-range', 'omit-profile-bound', 'double-budget-limit'),
    'terminal-binding': ('correct', 'key-only-observation', 'allow-unload', 'omit-retention-shape', 'omit-version-relation'),
    'retention-lane': ('correct', 'ignore-successor', 'allow-pending', 'ignore-context', 'omit-slot', 'omit-window', 'oversize-encoding'),
    'native-retirement': ('correct', 'local-presence-only', 'omit-intent-binding', 'omit-worker-status', 'omit-quiescence-epoch', 'early-intent-encoding', 'oversize-encoding'),
    'session-shape': ('correct', 'wrong-first-lane', 'wrong-physical', 'wrong-refusal-field', 'wrong-last-session'),
    'inbox-validation': ('correct', 'wrong-owner-result', 'wrong-lane-guard', 'wrong-session-result'),
    'inbox-save': ('correct', 'wrong-validation', 'wrong-budget', 'wrong-zero-row', 'wrong-hash'),
    'inbox-record-validation': ('correct', 'wrong-validation', 'wrong-budget', 'wrong-zero-row', 'wrong-hash'),
    'inbox-load': ('correct', 'wrong-digest', 'wrong-canonical', 'wrong-validation', 'wrong-policy'),
}
NATIVE_DRIVERS = (
    ('stage1-protocol-manifest', 'verification/experiments/stage1-manifest/native.py', True),
    *((('stage1-protocol-' + name), str(HERE/name/'native.py'), True) for name in TYPE_CASES),
    *((('stage1-protocol-' + name), str(HERE/name/'native.py'), False) for name in
      ('envelope-budget', 'envelope-writer', 'serde-binding', 'parser-binding', 'decode-assembly')),
    *((('stage1-' + name), str(HERE/name/'native.py'), False) for name in INBOX_CASES
      if name != 'inbox-record-total'),
    *CREATE_NATIVE_DRIVERS,
)
CRATE_CASES = ('schema-leaves', 'envelope-schema', 'envelope-bytes', 'json-wire')
NATIVE_CASES = tuple(label for label, _, _ in NATIVE_DRIVERS) + tuple(
    'stage1-protocol-'+name for name in CRATE_CASES)
AUDIT_TEMPLATE_COUNT = 419
AUDIT_DECLARATION_COUNT = 1723


def native_labels(profile='full'):
    return (tuple(label for label, _, _ in CREATE_NATIVE_DRIVERS)
            if check_profile(profile) == 'create-delivery' else NATIVE_CASES)


def _shared():
    from tools import acceptance_receipts
    return acceptance_receipts


def _read(path):
    if path.is_symlink() or not path.is_file():
        raise ValueError('missing original regular result: ' + str(path))
    result = json.loads(path.read_text())
    if not isinstance(result, dict):
        raise ValueError('original result must be a JSON object: ' + str(path))
    return result


def _sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _invoke(root, driver, args, env=None, *, log=None, pass_fds=()):
    # Inherit stdout: only the original actual_files checker emits Checked lines.
    argv = [sys.executable, str(root/driver), *map(str, args)]
    if log is None:
        subprocess.run(argv, cwd=root, env=env, stdin=subprocess.DEVNULL, check=True, pass_fds=pass_fds)
    else:
        with Path(log).open('xb') as stream:
            subprocess.run(argv, cwd=root, env=env, stdin=subprocess.DEVNULL,
                           stdout=stream, stderr=subprocess.STDOUT, check=True, pass_fds=pass_fds)
    return argv


def _evidence(receipt):
    shared = _shared()
    return Path(os.environ['SANDPROOF_NATIVE_EVIDENCE'])/'commands'/shared.digest(receipt['key'])/'evidence'


def _original_file(receipt, relative):
    path = _evidence(receipt)/relative
    wanted = receipt['files'].get('evidence/' + relative)
    if not wanted or path.is_symlink() or not path.is_file() or _sha(path) != wanted:
        raise ValueError('missing/changed sealed original native file: ' + relative)
    return path


def _check_prepared_source(root, project, inputs, changes):
    """Bind the full private copy, including files untouched by preparation."""
    if project.is_symlink() or not project.is_dir():
        raise ValueError('private preparation must be a regular directory')
    expected = dict(inputs)
    expected.update(changes)
    for name, wanted in inputs.items():
        relative = Path(name)
        if relative.is_absolute() or '..' in relative.parts:
            raise ValueError('invalid prepared input path')
        original = root/relative
        if original.is_symlink() or not original.is_file() or _sha(original) != wanted:
            raise ValueError('original preparation input changed: ' + name)
    actual = {}
    for path in project.rglob('*'):
        if path.is_symlink():
            raise ValueError('private preparation contains a symlink')
        if path.is_file():
            actual[str(path.relative_to(project))] = _sha(path)
    if actual != expected:
        raise ValueError('private prepared source differs from original inputs and allowed changes')
    return expected


def _role_preparation(root, output, prepared):
    """Use the original installer to derive its exact permitted changes."""
    inputs = prepared['inputs']
    reference = output/'preparation-reference'
    reference.mkdir(mode=0o700)
    # These are all inputs read by install(..., 'schema-leaves'). A changed
    # installer needing more inputs fails rather than silently broadening this.
    for name in ('verification/extract-foundations/src/lib.rs',
                 str(HERE/'local-view.toml.in'),
                 str(HERE/'schema-leaves/protocol_leaves.rs.in'),
                 str(HERE/'schema-leaves/RoleLeaves.lean.in')):
        source = root/name
        if name not in inputs or _sha(source) != inputs[name]:
            raise ValueError('original role preparation input differs: ' + name)
        destination = reference/name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
    helper = root/HERE/'local_view.py'
    if str(HERE/'local_view.py') not in inputs or _sha(helper) != inputs[str(HERE/'local_view.py')]:
        raise ValueError('original role preparation algorithm changed')
    spec = importlib.util.spec_from_file_location('registered_original_role_installer', helper)
    installer = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(installer)
    changes = installer.install(reference, 'schema-leaves')
    allowed = {'lakefile.toml', 'verification/extract-foundations/src/lib.rs',
               'verification/extract-foundations/src/protocol_leaves.rs',
               'SandboxModel/Lifecycle/Protocol/RoleLeaves.lean'}
    if set(changes) != allowed:
        raise ValueError('unregistered original role preparation changes')
    project = output/'preparation/source'
    _check_prepared_source(root, project, inputs, changes)
    return changes


def _crate_native(root, output, name):
    """Original README commands for the slices without a native.py driver."""
    shared = _shared()
    output.mkdir(mode=0o700, parents=True)
    project = root
    if name == 'schema-leaves':
        preparation = output/'preparation'
        _invoke(root, HERE/name/'formal.py', ['--prepare-only', '--output', preparation])
        prepared = _read(preparation/'result.json')
        if prepared.get('prepared') is not True or prepared.get('complete') is True:
            raise ValueError('role native preparation must not execute a proof')
        project = preparation/'source'
        changes = _role_preparation(root, output, prepared)
        manifest = project/'verification/extract-foundations/Cargo.toml'
    else:
        manifest = root/HERE/name/'Cargo.toml'
    args = ['--locked', '--offline', '--manifest-path', str(manifest)]
    commands = [('check' if name == 'json-wire' else 'test',
                 ['cargo', 'check' if name == 'json-wire' else 'test', *args])]
    if name == 'schema-leaves':
        commands[0][1].append('protocol_leaves::tests::')
    commands.append(('clippy', ['cargo', 'clippy', *args, '--all-targets', '--', '-D', 'warnings']))
    if name == 'envelope-bytes':
        commands.append(('driver-tests', [sys.executable, str(root/HERE/name/'test_formal.py')]))
    records = []
    for label, command in commands:
        log = output/(label+'.log')
        if name == 'schema-leaves':
            _check_prepared_source(root, project, prepared['inputs'], changes)
        shared.native_run('stage1-protocol-'+name+'-'+label, command, root, log)
        if name == 'schema-leaves':
            _check_prepared_source(root, project, prepared['inputs'], changes)
        if label == 'test':
            count = {'schema-leaves': 8, 'envelope-schema': 7, 'envelope-bytes': 4}[name]
            if not re.search(r'test result: ok\. '+str(count)+r' passed; 0 failed; 0 ignored;', log.read_text()):
                raise ValueError('missing original crate test inventory: ' + name)
        records.append(dict(argv=command, log_sha256=_sha(log)))
    result = dict(kind='registered-original-crate-native', complete=True, commands=records,
                  assertions_run=name != 'json-wire', production_modified=False,
                  role_preparation_only=name == 'schema-leaves')
    if name == 'schema-leaves':
        result['preparation_result_sha256'] = _sha(preparation/'result.json')
        result['allowed_preparation_changes'] = changes
        result['prepared_inputs_sha256'] = shared.digest(prepared['inputs'])
        result['prepared_rust_inputs'] = {name: _sha(project/name) for name in (
            'verification/extract-foundations/Cargo.toml',
            'verification/extract-foundations/src/lib.rs',
            'verification/extract-foundations/src/protocol_leaves.rs')}
        template = root/HERE/'schema-leaves/protocol_leaves.rs.in'
        result['original_eight_test_source'] = dict(path=str(template), sha256=_sha(template),
            private_path=str(project/'verification/extract-foundations/src/protocol_leaves.rs'),
            filter='protocol_leaves::tests::', required_passed=8)
    (output/'result.json').write_text(json.dumps(result, indent=2)+'\n')


def _variant_choices(path):
    tree = ast.parse(path.read_text())
    call, = [node for node in ast.walk(tree) if isinstance(node, ast.Call)
             and isinstance(node.func, ast.Attribute) and node.func.attr == 'add_argument'
             and node.args and isinstance(node.args[0], ast.Constant)
             and node.args[0].value == '--variant']
    choices, = [keyword.value for keyword in call.keywords if keyword.arg == 'choices']
    result = []
    for node in choices.elts:
        if isinstance(node, ast.Constant) and isinstance(node.value, str):
            result.append(node.value)
        elif (isinstance(node, ast.Starred) and isinstance(node.value, ast.Attribute)
                and node.value.attr in ('MUTATIONS', 'FAILURES')):
            # The complete inbox drivers keep their real mutation cases beside
            # the Rust mutation function; record-validation uses that same save
            # mutation set, with additional assertions for its composition.
            sources = {
                'inbox-validation': HERE/'inbox-validation/mutations.py',
                'inbox-save': HERE/'inbox-save/mutations.py',
                'inbox-record-validation': HERE/'inbox-save/mutations.py',
                'inbox-load': HERE/'inbox-load/mutations.py',
            }
            source = (path.parents[4]/sources[path.parent.name]
                      if path.parent.name in sources else path.parent/'prepare.py')
            preparation = ast.parse(source.read_text())
            mutations, = [statement.value for statement in preparation.body
                          if isinstance(statement, ast.Assign) and any(
                              isinstance(t, ast.Name) and t.id == node.value.attr for t in statement.targets)]
            if not isinstance(mutations, ast.Dict):
                raise ValueError('mutation cases must remain an explicit dictionary: ' + str(source))
            result += [ast.literal_eval(key) for key in mutations.keys]
        else:
            raise ValueError('unregistered original variant declaration: ' + str(path))
    return tuple(result)


def validate_metadata_inputs(root=ROOT, *, profile='full'):
    """Check explicit current input digests, not historical patches or globals."""
    check_profile(profile)
    directories = {HERE/name for name in CREATE_CASES}
    if profile == 'full':
        directories.update((HERE, Path('verification/experiments/stage1-manifest')))
        directories.update(Path(path).parent for path in audit_template_paths(root))
        directories.update(Path(driver).parent for _, driver, _ in NATIVE_DRIVERS)
        directories.update(Path(job['driver']).parent for job in formal_plan(Path('/unused'), root=root))
    names = subprocess.check_output(['git', 'ls-files', '-z', '--',
        str(HERE), 'verification/experiments/stage1-manifest'], cwd=root).decode().split('\0')
    checked = {}
    for name in sorted(filter(None, names)):
        path = Path(name)
        if path.parent not in directories or path.name not in ('inventory.json', 'candidate.json', 'frozen.json'):
            continue
        data = _read(root/path)
        fields = ('inputs', 'base_sources', 'production_inputs')
        if path.parent in {HERE/name for name in CREATE_CASES}:
            fields += ('source_inputs',)
        if path.name == 'frozen.json':
            fields += ('extraction_inputs', 'native_rust_inputs')
        for field in fields:
            if field not in data:
                continue
            if not isinstance(data[field], dict):
                raise ValueError('invalid explicit source digest field: ' + name + ':' + field)
            for relative, wanted in data[field].items():
                source = Path(relative)
                if (source.is_absolute() or '..' in source.parts or not isinstance(wanted, str)
                        or re.fullmatch('[0-9a-f]{64}', wanted) is None):
                    raise ValueError('invalid explicit source digest entry: ' + name)
                actual = root/source
                if actual.is_symlink() or not actual.is_file() or _sha(actual) != wanted:
                    raise ValueError('registered source digest differs: ' + name + ':' + field + ':' + relative)
            checked[name+':'+field] = len(data[field])
    return checked


def validate_registration(root=ROOT, *, profile='full'):
    """Fail before execution if a required native case or Rust mutant was lost."""
    check_profile(profile)
    create_profile(root)
    if len(NATIVE_CASES) != 53 or len(set(NATIVE_CASES)) != 53:
        raise ValueError('duplicate native registration')
    if profile == 'full':
        for name, variants in INBOX_CASES.items():
            if variants != _variant_choices(root/HERE/name/'formal.py'):
                raise ValueError('missing/extra original variant: ' + name)
        if INBOX_CASES['inbox-append'] != INBOX_CASES['inbox-record-total']:
            raise ValueError('total must cover exactly the original append variants')
    for job in formal_plan(Path('/unused'), root=root, profile=profile):
        if not set(job['native']) <= set(NATIVE_CASES):
            raise ValueError('missing native registration: ' + job['name'])
        if not (root/job['driver']).is_file():
            raise ValueError('missing original driver: ' + job['driver'])
        if '--case' in job['args']:
            tree = ast.parse((root/job['driver']).read_text())
            for statement in tree.body:
                if isinstance(statement, ast.Assign) and any(
                        isinstance(t, ast.Name) and t.id == 'CASES' for t in statement.targets):
                    if set(job['cases']) != set(ast.literal_eval(statement.value)):
                        raise ValueError('missing/extra original CASES: ' + job['name'])
    if profile == 'full':
        audit_template_paths(root)
    validate_metadata_inputs(root, profile=profile)


def run_native(root, work, *, profile='full', scope=None):
    """Call after the retained stage1 case, inside the original ning suite."""
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'native':
        raise ValueError('native registration must execute on the native host')
    validate_registration(root, profile=profile)
    scope = native_scope(profile) if scope is None else scope
    if scope not in ('all', native_scope(profile)):
        raise ValueError('native scope differs from the explicit Stage1 profile')
    shared = _shared()
    work = Path(work)/'stage1-registered'
    work.mkdir(parents=True, mode=0o700)
    # A second request for the *same* complete case uses the original cache.
    # Unlike formal consumption, native_case requires an action in native role.
    archive = None
    if profile == 'full':
        retained = shared.native_case('stage1-protocol', root, work/'retained', lambda:
            _invoke(root, HERE/'component.py', ['--native-only', '--output', work/'retained']), scope=scope)
        archive = _original_file(retained, 'source.tar')
    target = Path(os.environ['CARGO_TARGET_DIR'])/'stage1-protocol'
    receipts = {}
    drivers = CREATE_NATIVE_DRIVERS if profile == 'create-delivery' else NATIVE_DRIVERS
    for label, driver, needs_archive in drivers:
        output = work/label
        args = ['--output', output]
        if needs_archive:
            args += ['--source-archive', archive, '--target', target]

        def action(driver=Path(driver), args=args, output=output,
                   success_field='success' if needs_archive else 'complete'):
            _invoke(root, driver, args)
            result = _read(output/'result.json')
            if result.get(success_field) is not True:
                raise ValueError('original native driver did not complete: ' + str(driver))
            if str(driver) in {path for _, path, _ in CREATE_NATIVE_DRIVERS}:
                _check_create_native_result(root, Path(driver).parent.name, output, result)

        receipt = shared.native_case(label, root, output, action, scope=scope)
        original = _original_file(receipt, 'result.json')
        if driver in {path for _, path, _ in CREATE_NATIVE_DRIVERS}:
            _check_create_native_result(root, Path(driver).parent.name, original.parent, _read(original))
        receipts[label] = receipt
    for name in CRATE_CASES if profile == 'full' else ():
        label = 'stage1-protocol-'+name
        output = work/label
        receipt = shared.native_case(label, root, output,
            lambda name=name, output=output: _crate_native(root, output, name), scope=scope)
        _original_file(receipt, 'result.json')
        receipts[label] = receipt
    return receipts


def _check_create_native_result(root, name, folder, result):
    """Read every original log; an exit code alone is not a mutant witness."""
    plans = _create_native_plan(name, _read(root/HERE/name/'inventory.json'))
    shared = _shared()
    # Component reuse follows the existing complete native key. A changed
    # unrelated report needs a new functional receipt, not a relabelled source
    # snapshot or an unnecessary rebuild of unchanged Rust assertions.
    native_inputs = dict(shared.rust_inputs(root), **shared.rules(root), **shared.contract_inputs(root))
    source = result.get('inputs', {})
    if (result.get('kind') != name+'-native-development' or result.get('complete') is not True
            or result.get('prepare_only') or tuple(result.get('variants', {})) != tuple(plans)
            or not isinstance(source, dict) or not source
            or any(source.get(n) != digest for n, digest in native_inputs.items()
                   if not n.startswith('@configuration:'))):
        raise ValueError('incomplete or changed Create native source/views: '+name)
    commands = result.get('commands', [])
    if [(r.get('variant'), r.get('label')) for r in commands] != [
            (variant, label) for variant in plans for label in ('format', 'test', 'clippy')]:
        raise ValueError('missing/extra Create native command: '+name)
    for variant, plan in plans.items():
        checked = result['variants'][variant]
        if checked.get('complete') is not True or not checked.get('inputs'):
            raise ValueError('incomplete Create native view: '+name+':'+variant)
        failures = plan['expected_failures']
        for row in [r for r in commands if r['variant'] == variant]:
            label = row['label']
            log = folder/variant/(label+'.log')
            if log.is_symlink() or not log.is_file() or _sha(log) != row.get('log_sha256'):
                raise ValueError('changed original Create native log: '+str(log))
            code = 101 if ((label == 'test' and failures)
                           or (label == 'clippy' and plan.get('expected_clippy_errors'))) else 0
            if type(row.get('exit_code')) is not int or row['exit_code'] != code:
                raise ValueError('wrong original Create native exit: '+name+':'+variant+':'+label)
            if label == 'test':
                text = log.read_text()
                tally = f"{plan['test_count']-len(failures)} passed; {len(failures)} failed; 0 ignored;"
                block = text.rsplit('\nfailures:\n', 1)[-1].split('\ntest result:', 1)[0]
                observed = [line.strip().rsplit('::', 1)[-1] for line in block.splitlines()
                            if line.startswith('    controller::')]
                if (tally not in text or sorted(observed) != sorted(failures)
                        or bool(row.get('property_rejected')) != bool(failures)):
                    raise ValueError('Create native mutant did not reach the same assertions: '+name+':'+variant)
            if label == 'clippy' and plan.get('expected_clippy_errors'):
                if (row.get('clippy_errors') != plan['expected_clippy_errors']
                        or row.get('expected_mutant_lint_rejection') is not True):
                    raise ValueError('Create mutant lint is not the declared separate diagnostic')


def _append_create_jobs(jobs, output, root, profile):
    definition = create_profile(root)
    for package in definition['packages']:
        name = package['name']
        data = _read(root/HERE/name/'inventory.json')
        recipes = data['views'] if name == 'adopted-create' else data['variants']
        views = {}
        for case, recipe in recipes.items():
            audits = recipe.get('expected_audits', recipe.get('audits'))
            files = recipe.get('expected_files', recipe.get('files'))
            if (not isinstance(audits, list) or not audits or len(audits) != len(set(audits))
                    or any(not isinstance(a, str) for a in audits)):
                raise ValueError('missing/duplicate fixed Create view audits: '+name+':'+case)
            views[case] = dict(audits=audits, files=files,
                               seeds=recipe.get('seed_files', []),
                               property_rejected=name != 'adopted-create' and case != CREATE_CASES[name][0])
        args = [] if name in ('adopted-create', 'create-attachment-flow') else ['--variant', 'all']
        jobs.append(dict(name=name, driver=str(HERE/name/'formal.py'), args=args,
            output=str(output/name), native=[package['native_label']], cases=list(CREATE_CASES[name]),
            actual_files=True, create_package=name, profile=profile,
            expected_kind=CREATE_KINDS[name], inventory_sha256=package['inventory_sha256'], views=views))
    return jobs


def formal_plan(output, *, root=ROOT, profile='full'):
    """Concrete CLI entries; native evidence paths are supplied by admission."""
    output = Path(output)
    jobs = []
    check_profile(profile)

    def add(name, driver, args=(), *, native=(), cases=(), actual=True):
        jobs.append(dict(name=name, driver=str(driver), args=list(map(str, args)),
                         output=str(output/name), native=list(native), cases=list(cases),
                         actual_files=actual))

    if profile == 'create-delivery':
        return _append_create_jobs(jobs, output, root, profile)
    for prop, wrong in [('replay', 'key-only-replay'), ('validation', 'same-incarnation'),
                        ('envelope', 'session-only-envelope')]:
        add('replay-'+prop, HERE/'replay_formal.py', ['--property', prop], cases=('correct', wrong))
    manifest = _read(root/HERE/'manifest-validation/inventory.json')
    add('manifest-validation', HERE/'manifest_validation_formal.py', ['--case', 'all'],
        native=('stage1-protocol-manifest',), cases=tuple(manifest['cases']))
    for name in TYPE_CASES:
        data = _read(root/HERE/name/'inventory.json')
        cases = ('correct', *data['mutations']) if name != 'create-observation' else ('correct',)
        args = ['--packages', str(root/'.lake/packages')]
        if name != 'create-observation':
            args += ['--case', 'all']
        add(name, HERE/name/'formal.py', args, native=('stage1-protocol-'+name,), cases=cases)
    for name, driver, cases in [
        ('create-conversion', 'conversion_candidate.py', ('correct',)),
        ('create-guard', 'guard_candidate.py', ('correct', 'create-response-key-only',
            'create-current-retention-omitted', 'create-refused-after-success', 'create-renewal-negative-wrap')),
    ]:
        args = ['--packages', str(root/'.lake/packages')]
        if name == 'create-guard':
            args += ['--case', 'all']
        add(name, HERE/'create-observation'/driver, args,
            native=('stage1-protocol-create-observation',), cases=cases)
    add('json-wire', HERE/'json-wire/formal.py', ['--case', 'all'],
        native=('stage1-protocol-json-wire',), cases=('correct', 'skip-envelope-guard'))
    for name in ('schema-leaves', 'envelope-schema', 'envelope-bytes', 'envelope-budget'):
        add(name, HERE/name/'formal.py', native=('stage1-protocol-'+name,), cases=('correct',))
    for variant in ('correct', 'swallow-error'):
        add('envelope-writer-'+variant, HERE/'envelope-writer/formal.py', ['--variant', variant],
            native=('stage1-protocol-envelope-writer',), cases=(variant,))
    for name in ('serde-binding', 'parser-binding', 'decode-assembly'):
        driver = 'formal.py' if name == 'parser-binding' else 'admit.py'
        add(name, HERE/name/driver, native=('stage1-protocol-'+name,), cases=('correct',),
            actual=name == 'parser-binding')
        add(name+'-negative', HERE/name/'negative.py', native=('stage1-protocol-'+name,),
            cases=('actual-source-rejection',), actual=False)
    for name, variants in INBOX_CASES.items():
        # Total invokes this same append checker with additional bound proofs.
        if name == 'inbox-append':
            continue
        labels = ('stage1-inbox-record', 'stage1-inbox-append') if name == 'inbox-record-total' else ('stage1-'+name,)
        for variant in variants:
            add(name+'-'+variant, HERE/name/'formal.py', ['--require-native', '--variant', variant],
                native=labels, cases=(variant,))
    return _append_create_jobs(jobs, output, root, profile)


def _actuals(result):
    if result.get('kind') in CREATE_KINDS.values():
        return [row.get('actual', {}) for row in result.get('views', result.get('variants', {})).values()]
    if 'cases' in result:
        return list(result['cases'].values())
    return [result.get('actual', {})]


def _check_actual(result, job):
    if result.get('complete') is not True:
        raise ValueError('original formal driver incomplete: ' + job['name'])
    if 'create_package' in job:
        if (result.get('kind') != job['expected_kind'] or result.get('prepare_only')
                or result.get('selected_variant') is not None):
            raise ValueError('wrong or preparation-only Create result: '+job['name'])
        rows = result.get('views' if job['name'] == 'adopted-create' else 'variants', {})
        if tuple(rows) != tuple(job['cases']):
            raise ValueError('missing/extra fixed Create formal view: '+job['name'])
        all_flag = ('all_three_variants_complete' if job['name'] in (
            'create-attachment-flow', 'create-fixed-identity', 'create-lifetime-sql')
            else 'all_variants_complete' if job['name'] == 'create-standard-adapter' else None)
        if all_flag is not None and result.get(all_flag) is not True:
            raise ValueError('Create batch does not certify every fixed view: '+job['name'])
        for case, recipe in job['views'].items():
            row = rows[case]
            actual = row.get('actual', {})
            if (row.get('complete') is not True
                    or row.get('property_rejected') is not recipe['property_rejected']
                    or not actual.get('files') or not actual.get('service_log_sha256')
                    or actual.get('files') != row.get('files')
                    or sorted(actual.get('audits', [])) != sorted(recipe['audits'])
                    or sorted(row.get('audits', [])) != sorted(recipe['audits'])
                    or (recipe['files'] is not None and set(actual['files']) != set(recipe['files']))
                    or not set(recipe['seeds']) <= actual['files'].keys()):
                raise ValueError('incomplete Create source/file/audit/rejection view: '+job['name']+':'+case)
        return
    if 'cases' in result:
        if set(result['cases']) != set(job['cases']):
            raise ValueError('missing/extra original formal case: ' + job['name'])
        if any(row.get('complete') is not True for row in result['cases'].values()):
            raise ValueError('incomplete original formal case: ' + job['name'])
    if '--variant' in job['args'] and result.get('variant') != job['cases'][0]:
        raise ValueError('wrong actual variant result: ' + job['name'])
    if job['actual_files']:
        for actual in _actuals(result):
            if not actual.get('files') or not actual.get('audits') or not actual.get('service_log_sha256'):
                raise ValueError('missing original physical-file/service/audit evidence: ' + job['name'])
    if job['name'] in ('serde-binding-negative', 'parser-binding-negative', 'decode-assembly-negative'):
        if result.get('source_admission_rejected') is not True:
            raise ValueError('missing actual source-admission rejection: ' + job['name'])


def _append_coverage(result, variant, *, root=ROOT):
    """Admit only the direct total -> same-variant append inclusion."""
    actual = result['actual']
    required = ['InboxRecord'+n for n in ('Cloning', 'Selection', 'Predicates', 'Replay',
                'FreshModel', 'FreshSupport', 'NewSessionModel', 'Values')]
    required += ['InboxAppend'+n for n in ('Frame', 'Shape', 'Arithmetic', 'Model',
                 'CallerModel', 'Values', 'Selections', 'WitnessSupport')]
    required += {'correct': ['InboxAppendPredicates', 'InboxAppendDestroy', 'InboxAppendNonterminal',
                            'InboxAppend', 'InboxAppendWitnesses'],
                 'skip-sdk-gate': ['InboxAppendWrongSdk'],
                 'skip-prior-capture': ['InboxAppendWrongCapture']}[variant]
    prefix = 'SandboxModel/Lifecycle/Protocol/'
    if not {prefix+n+'.lean' for n in required} <= set(actual['files']):
        raise ValueError('total did not check every original append seed')
    for stem in required:
        part = 'inbox-record' if stem.startswith('InboxRecord') else 'inbox-append'
        template = root/HERE/part/(stem+'.lean.in')
        if actual['files'][prefix+stem+'.lean'] != _sha(template):
            raise ValueError('total changed an original append seed: ' + stem)
        audits = re.findall(r'^#audit_controller_proof (\S+)$', template.read_text(), re.M)
        if not set(audits) <= set(actual['audits']):
            raise ValueError('total omitted an original append seed audit: ' + stem)
    generated = 'verification/aeneas/stage1-protocol/request/SandproofStage1Request/'
    if not {generated+n+'.lean' for n in ('Types', 'Funs', 'TypesExternal', 'FunsExternal')} <= set(actual['files']):
        raise ValueError('total omitted original append generated files')
    if result.get('extension_admission', {}).get('exit_code') != 0 or not result.get('fresh_native_admission'):
        raise ValueError('total missing its original extension/fresh-native admission')
    if set(result.get('total_record', {}).get('native_cases', ())) != {'stage1-inbox-record', 'stage1-inbox-append'}:
        raise ValueError('total missing original native prerequisite cases')
    return dict(module='inbox-append', variant=variant, covered_by='inbox-record-total-'+variant)


def audit_templates(root=ROOT):
    """All annotated templates, including negatives; none silently excluded."""
    names = subprocess.check_output(['git', 'ls-files', '-z', '--', str(HERE)], cwd=root).decode().split('\0')
    result = {}
    for name in sorted(filter(None, names)):
        if not name.endswith('.lean.in'):
            continue
        path = root/name
        if path.is_symlink() or not path.is_file():
            raise ValueError('missing original audit template: ' + name)
        audits = re.findall(r'^#audit_controller_proof (\S+)$', path.read_text(), re.M)
        if audits:
            result[name] = audits
    if len(result) != AUDIT_TEMPLATE_COUNT or sum(map(len, result.values())) != AUDIT_DECLARATION_COUNT:
        raise ValueError('missing/extra finite audit template or declaration')
    return result


def audit_template_paths(root=ROOT):
    return tuple(audit_templates(root))


def _object_sha(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def _retained_result(record):
    saved = record.get('retained_result', {})
    path = Path(saved.get('path', '/missing-retained-result'))
    result = _read(path)
    if _sha(path) != saved.get('sha256'):
        raise ValueError('original retained component result changed')
    cases = result.get('cases', {})
    expected = {'correct', 'same-incarnation', 'key-only-replay', 'skip-capture-binding', 'session-only-envelope'}
    if (result.get('complete') is not True or result.get('all_cases_complete') is not True
            or set(cases) != expected or any(case.get('complete') is not True for case in cases.values())
            or set(cases['correct'].get('views', {})) != {'carriers', 'request', 'manifest'}):
        raise ValueError('original retained component requires all original cases and correct views')
    return path, result


def retained_audits(record):
    """Include original positive and negative views, bound to their service logs."""
    path, retained = _retained_result(record)
    observed = set()
    for case, row in retained['cases'].items():
        views = row.get('views' if case == 'correct' else 'proofs', {})
        if not views:
            raise ValueError('missing original retained audit views: ' + case)
        for view, actual in views.items():
            if (not re.fullmatch('[A-Za-z][A-Za-z0-9_-]*', view)
                    or not actual.get('files') or not actual.get('audits')):
                raise ValueError('missing original retained physical audit evidence')
            log = path.parent/case/view/'physical-files/service.log'
            if (log.is_symlink() or not log.is_file()
                    or _sha(log) != actual.get('service_log_sha256')):
                raise ValueError('original retained audit service log changed or missing')
            messages = set(re.findall(r'Checked ([\w.]+): \d+ standard logical dependencies', log.read_text()))
            if not set(actual['audits']) <= messages:
                raise ValueError('original retained audit absent from service log')
            observed.update(actual['audits'])
    return observed


def compute_retained_coverage(root, record):
    """Attribute the original 62 sources to this run's exact correct views."""
    inventory = _read(root/HERE/'retained-protocol.json')
    names = inventory.get('modules', [])
    if (inventory.get('schema') != 1 or not isinstance(names, list) or len(names) != 62 or len(set(names)) != 62
            or any(not isinstance(name, str) or not re.fullmatch('[A-Za-z][A-Za-z0-9_]*', name) for name in names)):
        raise ValueError('retained Protocol inventory must contain exactly the original 62 modules')
    # Same directory-set admission as source_inventory.retained, also on reuse.
    paths = list((root/'SandboxModel/Lifecycle/Protocol').glob('*.lean'))
    if (any(path.is_symlink() or not path.is_file() for path in paths)
            or {path.stem for path in paths} != set(names)):
        raise ValueError('unregistered or missing retained protocol proof file')
    sources = {}
    for name in names:
        relative = 'SandboxModel/Lifecycle/Protocol/'+name+'.lean'
        path = root/relative
        if path.is_symlink() or not path.is_file():
            raise ValueError('missing current retained Protocol source: ' + relative)
        sources[relative] = _sha(path)
    jobs = {job['name']: job for job in formal_plan(Path('/unused'), root=root)}
    rows = record.get('results', [])
    if len(rows) != len(jobs) or {row['name'] for row in rows} != set(jobs):
        raise ValueError('missing/duplicate original registered result')
    coverage = {}

    def include(actual, folder, job, result_path, result_sha256, view):
        if not actual.get('files') or not actual.get('audits') or not actual.get('service_log_sha256'):
            raise ValueError('missing original correct-view physical evidence')
        log = folder/'physical-files/service.log'
        if log.is_symlink() or not log.is_file() or _sha(log) != actual['service_log_sha256']:
            raise ValueError('original retained physical service log changed or missing')
        for relative, wanted in sources.items():
            if relative in coverage or actual['files'].get(relative) != wanted:
                continue
            coverage[relative] = dict(source_sha256=wanted, job=job, case='correct', view=view,
                result_path=str(result_path), result_sha256=result_sha256,
                actual_files_sha256=_object_sha(actual['files']), audits_sha256=_object_sha(actual['audits']),
                service_log_path=str(log), service_log_sha256=actual['service_log_sha256'])

    retained_path, retained = _retained_result(record)
    for profile, actual in retained['cases']['correct']['views'].items():
        include(actual, retained_path.parent/'correct'/profile, 'retained-stage1-protocol',
                retained_path, record['retained_result']['sha256'], profile)
    for row in rows:
        job = jobs[row['name']]
        result_path = Path(row['result_path'])
        if result_path.name != 'result.json' or result_path.parent.name != job['name']:
            raise ValueError('original result belongs to a different registered view')
        result = _read(result_path)
        if _sha(result_path) != row['result_sha256']:
            raise ValueError('original retained producer result changed')
        _check_actual(result, job)
        if not job['actual_files']:
            continue
        if 'cases' in result:
            if 'correct' not in result['cases']:
                continue
            actual = result['cases']['correct']
            folder = result_path.parent/'correct'
        elif job['cases'] == ['correct']:
            actual = result['actual']
            folder = result_path.parent
        else:
            continue
        include(actual, folder, job['name'], result_path, row['result_sha256'], job['name'])
    if set(coverage) != set(sources):
        raise ValueError('retained Protocol lacks exact correct-view coverage: ' + repr(sorted(set(sources)-set(coverage))))
    return coverage


def validate_retained_coverage(root, record):
    """The caller must first admit the whole original stage1 module receipt."""
    if record.get('kind') != 'registered-stage1-formal-drivers' or record.get('complete') is not True:
        raise ValueError('retained coverage requires a complete registered run')
    if record.get('retained_inventory_sha256') != _sha(root/HERE/'retained-protocol.json'):
        raise ValueError('original retained Protocol inventory changed')
    coverage = compute_retained_coverage(root, record)
    if record.get('retained_source_files') != coverage:
        raise ValueError('recorded retained source coverage differs from original results')
    return coverage


def _execution_helper(root):
    spec = importlib.util.spec_from_file_location('stage1_execution_helpers', root/'tools/proof_batch.py')
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)
    if helper.ROOT.resolve() != root.resolve():
        raise ValueError('execution helper belongs to another source root')
    # A private module instance in the parent, never shared driver globals.
    helper.MODULES = ('stage1-protocol',)
    return helper


def _job_description(job):
    names = ('name', 'driver', 'args', 'native', 'cases', 'actual_files',
             'create_package', 'profile', 'expected_kind', 'inventory_sha256', 'views')
    return {name: job[name] for name in names if name in job}


def inherited_lock():
    """Keep the operator's already-open lock across exec; never reopen/unlock it."""
    value = os.environ.get('SANDPROOF_LOCK_FD', '')
    path = Path(os.environ.get('SANDPROOF_LOCK_FILE', ''))
    if not re.fullmatch(r'[0-9]+', value) or not path.is_absolute() or path.is_symlink():
        raise ValueError('Create requires the original SANDPROOF_LOCK_FILE and inherited SANDPROOF_LOCK_FD')
    fd = int(value)
    actual, expected = os.fstat(fd), path.stat()
    if (not stat.S_ISREG(actual.st_mode) or not stat.S_ISREG(expected.st_mode)
            or (actual.st_dev, actual.st_ino) != (expected.st_dev, expected.st_ino)):
        raise ValueError('inherited coordination descriptor identifies a different lock')
    fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    return fd, dict(path=str(path), device=actual.st_dev, inode=actual.st_ino)


def _source_modes(root):
    """Bind tracked Git modes and the actual regular file's executable bit."""
    modes = {}
    for entry in subprocess.check_output(['git', 'ls-files', '--stage', '-z'], cwd=root).decode().split('\0'):
        if not entry:
            continue
        metadata, name = entry.split('\t', 1)
        mode, _, stage = metadata.split()
        path = root/name
        if (stage != '0' or mode not in ('100644', '100755') or path.is_symlink()
                or not path.is_file() or bool(path.stat().st_mode & 0o100) != (mode == '100755')):
            raise ValueError('checkpoint source requires regular committed files: ' + name)
        modes[name] = mode
    return modes


def _execution_context(root, shared, helper, receipts, *, profile='full'):
    """Keep full admission inputs, including the report and Git file modes."""
    tree = subprocess.check_output(['git', 'rev-parse', 'HEAD^{tree}'], cwd=root, text=True).strip()
    if subprocess.check_output(['git', 'status', '--porcelain', '--untracked-files=normal'], cwd=root):
        raise ValueError('checkpoint execution requires a clean committed source')
    repository = shared.repository_inputs(root)
    modes = _source_modes(root)
    if set(modes) != set(repository) or receipts['functional']['key'].get('repository') != repository:
        raise ValueError('current complete functional admission must match every tracked input')
    inherited_lock()
    return dict(schema=2, reuse_scope='same-tree-or-reviewed-state-report', source_tree=tree,
        profile=check_profile(profile), create_definition=create_profile(root),
        repository=dict(files=repository, modes=modes),
        rust=shared.rust_inputs(root), rules=shared.rules(root),
        contracts=shared.contract_inputs(root), source_archives=shared.source_archive_inputs(),
        runtime=helper.runtime(), driver_python=dict(version=sys.version,
            executable_sha256=_sha(Path(sys.executable).resolve())),
        native=dict(seal_sha256=_sha(Path(os.environ['SANDPROOF_NATIVE_EVIDENCE'])/'complete.json'),
            receipts={name: dict(key=receipt['key'], sha256=_sha(
                _evidence(receipt).parent/'receipt.json')) for name, receipt in receipts.items()}))


def _context_reuse_basis(previous, current, job, result):
    if previous == current:
        return dict(kind='same-source-tree')
    # Reviewed only for this producer's correct/always-current variants:
    # inbox-state/formal.py admits the transparent Inbox::state root and exactly
    # five value-only external functions (Bool/Option/String equality, clone,
    # Vec.is_empty). Its report bytes enter snapshot/hash/copy guards, not that
    # Rust body or the generated/Lean import closure. ProofAuditCommand inspects
    # theorem axioms; proof_services/lean_session read actual Lean files.
    # Do not extend this to other drivers, report names, or arbitrary docs.
    variants = {'inbox-state-correct':'correct', 'inbox-state-always-current':'always-current'}
    variant = variants.get(job['name'])
    if (variant is None or job['driver'] != str(HERE/'inbox-state/formal.py')
            or job['args'] != ['--require-native', '--variant', variant]
            or job['native'] != ['stage1-inbox-state'] or job['cases'] != [variant]
            or job['actual_files'] is not True
            or result.get('kind') != 'inbox-state-formal-development'
            or result.get('variant') != variant):
        raise ValueError('changed inputs: producer has no reviewed report-independent closure')
    report = 'docs/proof-coverage.md'
    normalized = []
    inventories = []
    for original in (previous, current):
        if original.get('schema') != 2:
            raise ValueError('report continuation requires original schema-2 source/mode evidence')
        context = copy.deepcopy(original)
        inventory = context.pop('repository')
        if not isinstance(inventory, dict):
            raise ValueError('missing original full source inventory')
        files, modes = inventory['files'], inventory['modes']
        if (not isinstance(files, dict) or not isinstance(modes, dict)
                or set(files) != set(modes) or modes.get(report) != '100644'):
            raise ValueError('report continuation requires the same regular tracked report')
        # Functional checks include report/link checks, so their CURRENT native
        # receipt is still mandatory. Only that receipt may be replaced; all
        # non-functional prerequisites, rules, tools and contracts remain exact.
        native = context['native']
        if not isinstance(native, dict) or not isinstance(native.get('receipts'), dict):
            raise ValueError('missing original native receipt identity')
        functional = native['receipts']['functional']
        if not isinstance(functional, dict) or not isinstance(functional.get('key'), dict):
            raise ValueError('missing original complete functional admission')
        key = functional['key']
        if (key.get('label') != 'functional' or key.get('command') != ['component-case']
                or key.get('repository') != files):
            raise ValueError('functional receipt does not admit its complete source inventory')
        key.pop('repository')
        functional.pop('sha256')
        native.pop('seal_sha256')
        context.pop('source_tree')
        inventories.append(inventory)
        normalized.append(context)
    old, new = inventories
    if (set(old['files']) != set(new['files']) or old['modes'] != new['modes']
            or {name for name in old['files'] if old['files'][name] != new['files'][name]} != {report}
            or normalized[0] != normalized[1]):
        raise ValueError('source/rules/native/tool change exceeds the reviewed report-only scope')
    if result.get('inputs') != old['files']:
        raise ValueError('original state proof does not bind the full saved source inventory')
    return dict(kind='reviewed-state-coverage-report-only', report=report,
        original_tree=previous['source_tree'], current_tree=current['source_tree'],
        original_report_sha256=old['files'][report], current_report_sha256=new['files'][report],
        original_functional_sha256=previous['native']['receipts']['functional']['sha256'],
        current_functional_sha256=current['native']['receipts']['functional']['sha256'],
        report_checks_reused=False)


def _actual_views(value):
    if isinstance(value, dict):
        if {'files', 'audits', 'service_log_sha256'} <= value.keys():
            yield value
        else:
            for child in value.values():
                yield from _actual_views(child)
    elif isinstance(value, list):
        for child in value:
            yield from _actual_views(child)


def _kernel_commands(value):
    if isinstance(value, dict):
        if value.get('argv', [])[:3] == ['lake', 'env', 'lean']:
            yield value
        for child in value.values():
            yield from _kernel_commands(child)
    elif isinstance(value, list):
        for child in value:
            yield from _kernel_commands(child)


def _saved_physical_view(actual, project, physical, commands):
    """Read the same complete/fresh diagnostic predicates as proof_services.

    No service is replayed or invoked here; these are original saved messages.
    Kernel logs, physical bytes and exact source audit declarations are bound.
    """
    for name in ('requests.jsonl', 'service.log'):
        if (physical/name).is_symlink() or not (physical/name).is_file():
            raise ValueError('missing or linked original service evidence')
    requests = [json.loads(line) for line in (physical/'requests.jsonl').read_text().splitlines()]
    if not requests or requests.pop() != {'tool': 'exit'}:
        raise ValueError('missing original service exit request')
    files = actual['files']
    expected_requests = [dict(tool='lean_diagnostic_messages', arguments={'file_path':name},
                              expected_sha256=digest) for name, digest in files.items()]
    if requests != expected_requests or _sha(physical/'service.log') != actual['service_log_sha256']:
        raise ValueError('original physical service inventory or log changed')
    rows = []
    for line in (physical/'service.log').read_text().splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if row.get('request', {}).get('tool') == 'lean_diagnostic_messages':
            rows.append(row)
    if len(rows) != len(requests):
        raise ValueError('missing original physical service response')
    expected_audits, observed_audits = [], []
    for index, (request, row) in enumerate(zip(requests, rows)):
        name = request['arguments']['file_path']
        if Path(name).is_absolute() or '..' in Path(name).parts:
            raise ValueError('physical source must stay within its original project')
        source = project/name
        if (source.is_symlink() or not source.is_file() or _sha(source) != files[name]
                or row.get('request') != request or row.get('stale') is not False
                or row.get('revision', {}).get('file_sha256') != files[name]):
            raise ValueError('changed or stale original physical proof')
        kernel = physical.parent/('kernel-'+str(index)+'.log')
        if kernel.is_symlink() or not kernel.is_file() or not any(
                command.get('argv') == ['lake', 'env', 'lean', name]
                and command.get('cwd') == str(project) and command.get('exit_code') == 0
                and command.get('log_sha256') == _sha(kernel) for command in commands):
            raise ValueError('missing original successful kernel log: '+name)
        result = row['result']
        for _ in range(3):
            if 'success' in result:
                break
            result = result.get('structuredContent', result.get('result', {}))
        if (result.get('success') is not True or result.get('partial') is not False
                or result.get('timed_out') is not False or result.get('failed_dependencies')
                or result.get('still_elaborating_lines') not in (None, [])
                or any(item.get('severity') in ('warning', 'error') for item in result.get('items', []))):
            raise ValueError('original saved diagnostics incomplete or rejected')
        expected = re.findall(r'^#audit_controller_proof (\S+)$', source.read_text(), re.M)
        checked = []
        for item in result.get('items', []):
            match = re.fullmatch(r'Checked ([\w.]+): \d+ standard logical dependencies',
                                 str(item.get('message', item.get('data', ''))))
            if match:
                checked.append(match[1])
        if set(expected) != set(checked):
            raise ValueError('original service audit differs from physical source')
        expected_audits.extend(expected)
        observed_audits.extend(checked)
    if (len(set(expected_audits)) != len(expected_audits)
            or sorted(expected_audits) != sorted(observed_audits)
            or sorted(expected_audits) != sorted(actual['audits'])):
        raise ValueError('missing/duplicate original component audit')


def _checkpoint_evidence(helper, job, result, result_path, log):
    """Require the original output tree, not merely files a receipt self-reports."""
    folder = result_path.parent.resolve()
    if 'create_package' in job and job['name'] != 'create-attachment-flow':
        rows = result['views' if job['name'] == 'adopted-create' else 'variants']
        for case, row in rows.items():
            atomic_path = folder/case/'result.json'
            atomic = _read(atomic_path)
            # The adopted positive view adds only its explicit non-rejection
            # flag in the aggregate; retain its original atomic JSON unchanged.
            keys = {'result_sha256', 'result_path', 'relative_path'}
            if job['name'] == 'adopted-create':
                keys.add('property_rejected')
            if (_sha(atomic_path) != row.get('result_sha256')
                    or {k:v for k,v in row.items() if k not in keys} != atomic):
                raise ValueError('Create aggregate differs from its original atomic result: '+case)
    evidence = helper.evidence_files(folder, include_sources=True)
    for parent, dirs, names in os.walk(folder):
        dirs[:] = [name for name in dirs if not ('target' in name or name in ('.git', '.lake', '.tools', 'node_modules'))]
        for name in names:
            path = Path(parent)/name
            if path.suffix in ('.in', '.stdout'):
                if path.is_symlink() or not path.is_file():
                    raise ValueError('linked original proof input')
                evidence[str(path.resolve())] = _sha(path)
    if not any(Path(name).suffix == '.llbc' and 'artifacts' in Path(name).relative_to(folder).parts
               for name in evidence):
        raise ValueError('missing original raw LLBC evidence')
    commands = list(_kernel_commands(result))
    views = list(_actual_views(result))
    if job['actual_files'] and not views:
        raise ValueError('missing original actual-file views')
    service_paths = [Path(name) for name in evidence if name.endswith('/physical-files/service.log')]
    for actual in views:
        projects = dict.fromkeys(command['cwd'] for command in commands)
        project = next((Path(name) for name in projects if all(
            (Path(name)/file).is_file() and not (Path(name)/file).is_symlink()
            and _sha(Path(name)/file) == digest for file, digest in actual['files'].items())), None)
        physical = next((path.parent for path in service_paths
                         if evidence[str(path)] == actual['service_log_sha256']), None)
        if project is None or physical is None or not project.resolve().is_relative_to(folder.resolve()):
            raise ValueError('original physical project/service evidence is missing')
        _saved_physical_view(actual, project, physical, commands)
        for name, digest in actual['files'].items():
            if evidence.get(str((project/name).resolve())) != digest:
                raise ValueError('physical proof is absent from the complete evidence inventory')
    if log.is_symlink() or not log.is_file():
        raise ValueError('missing original command log')
    evidence[str(log.resolve())] = _sha(log)
    return evidence


def _resume_rows(path):
    if path is None:
        return {}
    record = _read(Path(path))
    rows = record.get('results', [])
    if (record.get('kind') != 'registered-stage1-formal-drivers'
            or record.get('execution_schema') != 1
            or len({row['name'] for row in rows}) != len(rows)):
        raise ValueError('resume requires an original checkpoint-producing registered run')
    return {row['name']: row for row in rows}


def _reuse_job(helper, job, context, previous):
    if previous is None:
        return None, 'no prior successful checkpoint'
    try:
        receipt_path = Path(previous['checkpoint_path'])
        if _sha(receipt_path) != previous['checkpoint_sha256']:
            raise ValueError('checkpoint digest changed')
        receipt = _read(receipt_path)
        if (receipt.get('schema') != 1 or receipt.get('complete') is not True
                or type(receipt.get('exit_code')) is not int or receipt['exit_code'] != 0
                or receipt.get('job') != _job_description(job)):
            raise ValueError('checkpoint does not describe this complete job')
        saved_context = Path(receipt['context_path'])
        if _sha(saved_context) != receipt['context_sha256']:
            raise ValueError('original execution context digest changed')
        original_context = _read(saved_context)
        result_path = Path(previous['result_path'])
        result = _read(result_path)
        if (_sha(result_path) != previous['result_sha256']
                or receipt['result_path'] != str(result_path)
                or receipt['result_sha256'] != previous['result_sha256']
                or receipt['command_log'] != previous['command_log']
                or receipt['argv'] != previous['argv']
                or result_path.parent.name != job['name']):
            raise ValueError('original result digest or location changed')
        _check_actual(result, job)
        basis = _context_reuse_basis(original_context, context, job, result)
        if _checkpoint_evidence(helper, job, result, result_path,
                Path(receipt['command_log'])) != receipt['evidence']:
            raise ValueError('original evidence inventory changed')
        row = dict(previous, use='REUSE', reuse_seconds=0, reuse_basis=basis)
        return row, None
    except (OSError, ValueError, KeyError, TypeError) as error:
        return None, repr(error)


def _launch_job(root, job, directory, paths, runtime_seed=None):
    started = time.monotonic()
    directory.mkdir(mode=0o700, parents=True)
    for name in ('runtime-cache', 'cargo-target'):
        (directory/name).mkdir(mode=0o700)
    seed_sha256 = None
    if runtime_seed is not None:
        if runtime_seed.is_symlink() or not runtime_seed.is_file():
            raise ValueError('missing original verified runtime metadata seed')
        content = runtime_seed.read_bytes()
        seed = json.loads(content)
        if seed.get('schema') != 1 or not isinstance(seed.get('files'), dict):
            raise ValueError('invalid original runtime metadata seed')
        # Copy only the parent's verified metadata, never a caller's runtime
        # profile. The original helper still rechecks stat/content in the child.
        with (directory/'runtime-cache/runtime-file-hashes.json').open('xb') as stream:
            stream.write(content)
        seed_sha256 = hashlib.sha256(content).hexdigest()
    args = list(job['args'])
    pass_fds = ()
    if 'create_package' in job:
        fd, lock = inherited_lock()
        pass_fds = (fd,)
        args += ['--lock', lock['path'], '--lock-fd', str(fd)]
    if job['name'] in ('serde-binding-negative', 'parser-binding-negative', 'decode-assembly-negative'):
        args += ['--native-evidence', str(paths[job['native'][0]])]
    args += ['--output', job['output']]
    env = dict(os.environ, SANDPROOF_FORMAL_CACHE=str(directory/'runtime-cache'),
               CARGO_TARGET_DIR=str(directory/'cargo-target'), PYTHONDONTWRITEBYTECODE='1')
    transport = dict(pass_fds=pass_fds) if pass_fds else {}
    argv = _invoke(root, Path(job['driver']), args, env, log=directory/'command.log', **transport)
    return dict(name=job['name'], argv=argv, result_path=str(Path(job['output'])/'result.json'),
                seconds=time.monotonic()-started, command_log=str(directory/'command.log'),
                runtime_seed_sha256=seed_sha256)


def execute_formal_jobs(root, output, plan, context, paths, *, jobs=1, resume=None, recompute=False,
                        source_guard=None, progress=None, runtime_seed=None, profile='full'):
    """Fixed-plan jobs only. Threads launch independent Python processes.

    The parent alone validates and publishes checkpoints. Failures stop new
    dispatch; already-started independent jobs finish and retain their evidence.
    """
    if jobs not in (1, 2, 4):
        raise ValueError('registered worker count must be 1, 2 or 4')
    if len({job['name'] for job in plan}) != len(plan):
        raise ValueError('duplicate registered execution job')
    expected = {job['name']:_job_description(job) for job in formal_plan(output, root=root, profile=profile)}
    for job in plan:
        if (re.fullmatch('[a-z][a-z0-9-]*', job['name']) is None
                or expected.get(job['name']) != _job_description(job)
                or Path(job['output']).resolve() != (output/job['name']).resolve()
                or Path(job['output']).exists()):
            raise ValueError('job must use its original descriptor and new private output')
    helper, shared = _execution_helper(root), _shared()
    context_path = output/'execution-inputs.json'
    shared.write_json(context_path, context)
    old = _resume_rows(resume)
    results, failures, misses = {}, {}, {}
    pending = []
    for job in plan:
        started = time.monotonic()
        row, reason = _reuse_job(helper, job, context, old.get(job['name'])) if not recompute else (None, 'explicit recompute')
        if row is not None:
            row['reuse_seconds'] = time.monotonic()-started
            results[job['name']] = row
        else:
            misses[job['name']] = reason
            pending.append(job)
    def publish():
        if progress is not None:
            progress([results[job['name']] for job in plan if job['name'] in results], dict(failures), dict(misses))
    publish()
    stopped = False
    with ThreadPoolExecutor(max_workers=jobs) as executor:
        inflight = {}
        while pending or inflight:
            while pending and len(inflight) < jobs and not stopped:
                job = pending.pop(0)
                future = executor.submit(_launch_job, root, job, output/'execution'/job['name'], paths,
                                         runtime_seed)
                inflight[future] = job
            if not inflight:
                break
            done, _ = wait(inflight, return_when=FIRST_COMPLETED)
            for future in done:
                job = inflight.pop(future)
                directory = output/'execution'/job['name']
                try:
                    row = future.result()
                    result_path = Path(row['result_path'])
                    result = _read(result_path)
                    _check_actual(result, job)
                    evidence = _checkpoint_evidence(helper, job, result, result_path, Path(row['command_log']))
                    if source_guard is not None:
                        source_guard()
                    receipt = dict(schema=1, complete=True, exit_code=0, job=_job_description(job),
                        context_path=str(context_path), context_sha256=_sha(context_path),
                        result_path=str(result_path), result_sha256=_sha(result_path),
                        command_log=row['command_log'], argv=row['argv'],
                        evidence=evidence, seconds=row['seconds'])
                    checkpoint = directory/'checkpoint.json'
                    shared.write_json(checkpoint, receipt)
                    row.update(use='RUN', result_sha256=receipt['result_sha256'],
                        checkpoint_path=str(checkpoint), checkpoint_sha256=_sha(checkpoint))
                    results[job['name']] = row
                except Exception as error:
                    stopped = True
                    failures[job['name']] = dict(complete=False, error=repr(error), output=job['output'],
                        command_log=str(directory/'command.log'), context_sha256=_sha(context_path))
                    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
                    shared.write_json(directory/'failure.json', failures[job['name']])
            publish()
    return [results[job['name']] for job in plan if job['name'] in results], failures


def _execution_options(jobs=None, resume_from=None, recompute=False):
    sources = dict(workers='argument' if jobs is not None else 'default',
                   resume_from='argument' if resume_from is not None else 'default')
    if jobs is None:
        value = os.environ.get('SANDPROOF_STAGE1_JOBS')
        if value is not None:
            if value not in ('1', '2', '4'):
                raise ValueError('SANDPROOF_STAGE1_JOBS must be 1, 2 or 4')
            jobs = int(value)
            sources['workers'] = 'SANDPROOF_STAGE1_JOBS'
        else:
            jobs = 1
    if type(jobs) is not int or jobs not in (1, 2, 4):
        raise ValueError('registered worker count must be 1, 2 or 4')
    if resume_from is None and os.environ.get('SANDPROOF_STAGE1_RESUME_FROM'):
        resume_from = Path(os.environ['SANDPROOF_STAGE1_RESUME_FROM'])
        sources['resume_from'] = 'SANDPROOF_STAGE1_RESUME_FROM'
        if not resume_from.is_absolute():
            raise ValueError('SANDPROOF_STAGE1_RESUME_FROM must be an absolute original record path')
    if resume_from is not None:
        resume_from = Path(resume_from).absolute()
    return dict(workers=jobs, resume_from=str(resume_from) if resume_from is not None else None,
                recompute=recompute or os.environ.get('SANDPROOF_PROOF_RECOMPUTE') == '1', sources=sources)


def run_formal(output, retained_result=None, *, root=ROOT, jobs=None, resume_from=None,
               recompute=False, profile='full'):
    """Call after the unchanged retained component, on the proof host only."""
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal':
        raise ValueError('formal registration consumes shared native receipts only')
    options = _execution_options(jobs, resume_from, recompute)
    jobs, resume_from = options['workers'], options['resume_from']
    validate_registration(root, profile=profile)
    if (profile == 'full') != (retained_result is not None):
        raise ValueError('only the full profile requires its original retained component result')
    shared = _shared()
    cache = Path(os.environ['SANDPROOF_NATIVE_EVIDENCE'])
    scope = native_scope(profile)
    shared.validate_native(cache, scope=scope)
    # Validate the exact current source/rules/runtime of every prerequisite.
    labels = ('functional', *(('stage1-protocol',) if profile == 'full' else ()), *native_labels(profile))
    receipts = {name: shared.native_case(name, root, None, scope=scope) for name in labels}
    paths = {name: _evidence(receipt) for name, receipt in receipts.items()}
    for name in native_labels(profile):
        _original_file(receipts[name], 'result.json')
    for label, driver, _ in CREATE_NATIVE_DRIVERS:
        original = _original_file(receipts[label], 'result.json')
        _check_create_native_result(root, Path(driver).parent.name, original.parent, _read(original))
    source_before = shared.repository_inputs(root)
    output = Path(output)
    output.mkdir(mode=0o700, parents=True)
    record = dict(kind='registered-stage1-formal-drivers', complete=False, results=[], covered=[],
                  profile=profile, create_definition=create_profile(root), whole_V3_acceptance=False,
                  shared_seal_sha256=_sha(cache/'complete.json'),
                  source_snapshot_sha256=shared.digest(source_before),
                  native_receipts={name: dict(path=str(paths[name].parent/'receipt.json'),
                      sha256=_sha(paths[name].parent/'receipt.json')) for name in receipts},
                  stdout_evidence='original stdout retained by the public module command.log',
                  retained_component_replaced=False, whole_V0_acceptance=False,
                  execution_schema=1, execution=dict(**options, reuse_scope='same-tree-or-reviewed-state-report',
                    affinity=sorted(os.sched_getaffinity(0)) if hasattr(os, 'sched_getaffinity') else None,
                    private_worker_caches=True, outer_coordination_lock='retained by the calling operator'))
    def source_guard():
        if (_sha(cache/'complete.json') != record['shared_seal_sha256']
                or shared.repository_inputs(root) != source_before
                or _source_modes(root) != context['repository']['modes']):
            raise ValueError('shared native seal or registered source changed during execution')
    def progress(rows, failures, misses):
        record.update(results=rows, execution_failures=failures, reuse_misses=misses)
        shared.write_json(output/'registered-result.json', record)
    try:
        if profile == 'full':
            record['retained_result'] = dict(path=str(retained_result), sha256=_sha(retained_result))
            _retained_result(record)
        helper = _execution_helper(root)
        context = _execution_context(root, shared, helper, receipts, profile=profile)
        plan = formal_plan(output, root=root, profile=profile)
        rows, failures = execute_formal_jobs(root, output, plan, context, paths, jobs=jobs,
            resume=resume_from, recompute=options['recompute'],
            source_guard=source_guard, progress=progress, profile=profile,
            runtime_seed=Path(os.environ['SANDPROOF_FORMAL_CACHE'])/'runtime-file-hashes.json')
        record['results'] = rows
        # Only original admitted child output is forwarded, in original plan order.
        for row in rows:
            sys.stdout.write(Path(row['command_log']).read_text())
        if failures or len(rows) != len(plan):
            raise ValueError('registered jobs incomplete; successful checkpoints retained: ' + repr(failures))
        for job, row in zip(plan, rows):
            if row['name'] != job['name']:
                raise ValueError('registered result order differs from the original plan')
            if job['name'].startswith('inbox-record-total-'):
                record['covered'].append(_append_coverage(_read(Path(row['result_path'])), job['cases'][0], root=root))
        # These are the original LLBC/receipt admission regressions, not Rust
        # unit tests. Their required original captures are never optional.
        record['source_admission_regressions'] = []
        for name, suffix in ([('serde-binding', 'encode'), ('parser-binding', 'decode'),
                             ('decode-assembly', 'decode')] if profile == 'full' else []):
            producer, = [row for row in rows if row['name'] == name]
            capture = Path(producer['result_path']).parent/'source/artifacts'/('bytes-recorded_'+suffix+'.llbc')
            if capture.is_symlink() or not capture.is_file():
                raise ValueError('missing original LLBC for admission regressions: ' + name)
            prefix = 'SANDPROOF_' + name.upper().replace('-', '_')
            env = dict(os.environ, **{prefix+'_LLBC': str(capture),
                                      prefix+'_NATIVE': str(paths['stage1-protocol-'+name])})
            argv = _invoke(root, HERE/name/'test_binding.py', [], env)
            record['source_admission_regressions'].append(dict(name=name, argv=argv,
                                                               llbc_sha256=_sha(capture)))
        expected_audits = ({audit for audits in audit_templates(root).values() for audit in audits}
            if profile == 'full' else {a for job in plan for view in job['views'].values() for a in view['audits']})
        observed_audits = retained_audits(record) if profile == 'full' else set()
        for checked in record['results']:
            result = _read(Path(checked['result_path']))
            for actual in _actuals(result):
                observed_audits.update(actual.get('audits', ()))
        record['template_audits'] = dict(expected=sorted(expected_audits), observed=sorted(observed_audits),
            missing=sorted(expected_audits-observed_audits),
            scope=('all template declarations; retained component audits also enter the public total gate'
                   if profile == 'full' else 'all 42 fixed Create views; each physical view checked separately'))
        if record['template_audits']['missing']:
            raise ValueError('registered template audit declarations missing: '
                             + repr(record['template_audits']['missing']))
        if profile == 'full':
            record['retained_source_files'] = compute_retained_coverage(root, record)
            record['retained_inventory_sha256'] = _sha(root/HERE/'retained-protocol.json')
        elif expected_audits != observed_audits:
            raise ValueError('unexpected Create audit declarations')
        shared.validate_native(cache, scope=scope)
        source_guard()
        if helper.runtime() != context['runtime']:
            raise ValueError('actual formal tools/libraries/options changed during execution')
        record['complete'] = True
    finally:
        shared.write_json(output/'registered-result.json', record)
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('native', 'formal', 'plan'))
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--retained-result', type=Path)
    parser.add_argument('--jobs', type=int, choices=(1, 2, 4))
    parser.add_argument('--resume-from', type=Path)
    parser.add_argument('--recompute', action='store_true')
    parser.add_argument('--profile', choices=PROFILES, default='full')
    args = parser.parse_args()
    os.umask(0o077)
    if args.mode == 'native':
        run_native(ROOT, args.output.absolute(), profile=args.profile)
    elif args.mode == 'formal':
        if args.profile == 'full' and args.retained_result is None:
            parser.error('formal requires --retained-result from the same original component run')
        run_formal(args.output.absolute(), args.retained_result.absolute() if args.retained_result is not None else None, jobs=args.jobs,
                   resume_from=args.resume_from.absolute() if args.resume_from is not None else None,
                   recompute=args.recompute, profile=args.profile)
    else:
        print(json.dumps(dict(profile=args.profile, create_definition=create_profile(),
                              native_cases=native_labels(args.profile),
                              formal_jobs=formal_plan(args.output, profile=args.profile),
                              audit_templates=audit_templates() if args.profile == 'full' else None), indent=2))


if __name__ == '__main__':
    main()
