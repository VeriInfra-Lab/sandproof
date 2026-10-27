"""Finite V3 wiring around the original drivers and shared native receipts.

The existing retained component and all original V0 targets remain separate.
This module neither implements a proof nor issues a shared native seal.
"""
import argparse
import ast
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
HERE = Path('verification/experiments/stage1-protocol')
sys.path.insert(0, str(ROOT))

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
}
NATIVE_DRIVERS = (
    ('stage1-protocol-manifest', 'verification/experiments/stage1-manifest/native.py', True),
    *((('stage1-protocol-' + name), str(HERE/name/'native.py'), True) for name in TYPE_CASES),
    *((('stage1-protocol-' + name), str(HERE/name/'native.py'), False) for name in
      ('envelope-budget', 'envelope-writer', 'serde-binding', 'parser-binding', 'decode-assembly')),
    *((('stage1-' + name), str(HERE/name/'native.py'), False) for name in INBOX_CASES
      if name != 'inbox-record-total'),
)
CRATE_CASES = ('schema-leaves', 'envelope-schema', 'envelope-bytes', 'json-wire')
NATIVE_CASES = tuple(label for label, _, _ in NATIVE_DRIVERS) + tuple(
    'stage1-protocol-'+name for name in CRATE_CASES)
AUDIT_TEMPLATE_COUNT = 162
AUDIT_DECLARATION_COUNT = 623


def native_labels():
    return NATIVE_CASES


def _shared():
    from tools import acceptance_receipts
    return acceptance_receipts


def _read(path):
    if path.is_symlink() or not path.is_file():
        raise ValueError('missing original regular result: ' + str(path))
    return json.loads(path.read_text())


def _sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _invoke(root, driver, args, env=None):
    # Inherit stdout: only the original actual_files checker emits Checked lines.
    argv = [sys.executable, str(root/driver), *map(str, args)]
    subprocess.run(argv, cwd=root, env=env, stdin=subprocess.DEVNULL, check=True)
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
        elif isinstance(node, ast.Starred) and isinstance(node.value, ast.Attribute) and node.value.attr == 'MUTATIONS':
            preparation = ast.parse((path.parent/'prepare.py').read_text())
            mutations, = [statement.value for statement in preparation.body
                          if isinstance(statement, ast.Assign) and any(
                              isinstance(t, ast.Name) and t.id == 'MUTATIONS' for t in statement.targets)]
            result += [ast.literal_eval(key) for key in mutations.keys]
        else:
            raise ValueError('unregistered original variant declaration: ' + str(path))
    return tuple(result)


def validate_metadata_inputs(root=ROOT):
    """Check explicit current input digests, not historical patches or globals."""
    directories = {HERE, Path('verification/experiments/stage1-manifest')}
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


def validate_registration(root=ROOT):
    """Fail before execution if a required native case or Rust mutant was lost."""
    if len(set(NATIVE_CASES)) != len(NATIVE_CASES):
        raise ValueError('duplicate native registration')
    for name, variants in INBOX_CASES.items():
        if variants != _variant_choices(root/HERE/name/'formal.py'):
            raise ValueError('missing/extra original variant: ' + name)
    if INBOX_CASES['inbox-append'] != INBOX_CASES['inbox-record-total']:
        raise ValueError('total must cover exactly the original append variants')
    for job in formal_plan(Path('/unused'), root=root):
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
    audit_template_paths(root)
    validate_metadata_inputs(root)


def run_native(root, work):
    """Call after the retained stage1 case, inside the original ning suite."""
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'native':
        raise ValueError('native registration must execute on the native host')
    validate_registration(root)
    shared = _shared()
    work = Path(work)/'stage1-registered'
    work.mkdir(parents=True, mode=0o700)
    # A second request for the *same* complete case uses the original cache.
    # Unlike formal consumption, native_case requires an action in native role.
    retained = shared.native_case('stage1-protocol', root, work/'retained', lambda:
        _invoke(root, HERE/'component.py', ['--native-only', '--output', work/'retained']))
    archive = _original_file(retained, 'source.tar')
    target = Path(os.environ['CARGO_TARGET_DIR'])/'stage1-protocol'
    receipts = {}
    for label, driver, needs_archive in NATIVE_DRIVERS:
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

        receipt = shared.native_case(label, root, output, action)
        _original_file(receipt, 'result.json')
        receipts[label] = receipt
    for name in CRATE_CASES:
        label = 'stage1-protocol-'+name
        output = work/label
        receipt = shared.native_case(label, root, output,
            lambda name=name, output=output: _crate_native(root, output, name))
        _original_file(receipt, 'result.json')
        receipts[label] = receipt
    return receipts


def formal_plan(output, *, root=ROOT):
    """Concrete CLI entries; native evidence paths are supplied by admission."""
    output = Path(output)
    jobs = []

    def add(name, driver, args=(), *, native=(), cases=(), actual=True):
        jobs.append(dict(name=name, driver=str(driver), args=list(map(str, args)),
                         output=str(output/name), native=list(native), cases=list(cases),
                         actual_files=actual))

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
    return jobs


def _actuals(result):
    if 'cases' in result:
        return list(result['cases'].values())
    return [result.get('actual', {})]


def _check_actual(result, job):
    if result.get('complete') is not True:
        raise ValueError('original formal driver incomplete: ' + job['name'])
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


def run_formal(output, retained_result, *, root=ROOT):
    """Call after the unchanged retained component, on the proof host only."""
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal':
        raise ValueError('formal registration consumes shared native receipts only')
    validate_registration(root)
    shared = _shared()
    cache = Path(os.environ['SANDPROOF_NATIVE_EVIDENCE'])
    shared.validate_native(cache, scope='stage1-protocol')
    # Validate the exact current source/rules/runtime of every prerequisite.
    labels = ('functional', 'stage1-protocol', *NATIVE_CASES)
    receipts = {name: shared.native_case(name, root, None, scope='stage1-protocol') for name in labels}
    paths = {name: _evidence(receipt) for name, receipt in receipts.items()}
    for name in NATIVE_CASES:
        _original_file(receipts[name], 'result.json')
    source_before = shared.repository_inputs(root)
    output = Path(output)
    output.mkdir(mode=0o700, parents=True)
    record = dict(kind='registered-stage1-formal-drivers', complete=False, results=[], covered=[],
                  shared_seal_sha256=_sha(cache/'complete.json'),
                  source_snapshot_sha256=shared.digest(source_before),
                  native_receipts={name: dict(path=str(paths[name].parent/'receipt.json'),
                      sha256=_sha(paths[name].parent/'receipt.json')) for name in receipts},
                  stdout_evidence='original stdout retained by the public module command.log',
                  retained_result=dict(path=str(retained_result), sha256=_sha(retained_result)),
                  retained_component_replaced=False, whole_V0_acceptance=False)
    try:
        _retained_result(record)
        for job in formal_plan(output, root=root):
            args = list(job['args'])
            if job['name'] in ('serde-binding-negative', 'parser-binding-negative', 'decode-assembly-negative'):
                args += ['--native-evidence', str(paths[job['native'][0]])]
            args += ['--output', job['output']]
            argv = _invoke(root, Path(job['driver']), args)
            path = Path(job['output'])/'result.json'
            result = _read(path)
            _check_actual(result, job)
            record['results'].append(dict(name=job['name'], argv=argv, result_path=str(path),
                                          result_sha256=_sha(path)))
            if job['name'].startswith('inbox-record-total-'):
                record['covered'].append(_append_coverage(result, job['cases'][0], root=root))
        # These are the original LLBC/receipt admission regressions, not Rust
        # unit tests. Their required original captures are never optional.
        record['source_admission_regressions'] = []
        for name, suffix in [('serde-binding', 'encode'), ('parser-binding', 'decode'),
                             ('decode-assembly', 'decode')]:
            capture = output/name/'source/artifacts'/('bytes-recorded_'+suffix+'.llbc')
            if capture.is_symlink() or not capture.is_file():
                raise ValueError('missing original LLBC for admission regressions: ' + name)
            prefix = 'SANDPROOF_' + name.upper().replace('-', '_')
            env = dict(os.environ, **{prefix+'_LLBC': str(capture),
                                      prefix+'_NATIVE': str(paths['stage1-protocol-'+name])})
            argv = _invoke(root, HERE/name/'test_binding.py', [], env)
            record['source_admission_regressions'].append(dict(name=name, argv=argv,
                                                               llbc_sha256=_sha(capture)))
        expected_audits = {audit for audits in audit_templates(root).values() for audit in audits}
        observed_audits = set()
        for checked in record['results']:
            result = _read(Path(checked['result_path']))
            for actual in _actuals(result):
                observed_audits.update(actual.get('audits', ()))
        record['template_audits'] = dict(expected=sorted(expected_audits), observed=sorted(observed_audits),
            missing=sorted(expected_audits-observed_audits),
            scope='all template declarations; retained component audits also enter the public total gate')
        record['retained_source_files'] = compute_retained_coverage(root, record)
        record['retained_inventory_sha256'] = _sha(root/HERE/'retained-protocol.json')
        shared.validate_native(cache, scope='stage1-protocol')
        if (_sha(cache/'complete.json') != record['shared_seal_sha256']
                or shared.repository_inputs(root) != source_before):
            raise ValueError('shared native seal or registered source changed during formal execution')
        record['complete'] = True
    finally:
        (output/'registered-result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('native', 'formal', 'plan'))
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--retained-result', type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    if args.mode == 'native':
        run_native(ROOT, args.output.absolute())
    elif args.mode == 'formal':
        if args.retained_result is None:
            parser.error('formal requires --retained-result from the same original component run')
        run_formal(args.output.absolute(), args.retained_result.absolute())
    else:
        print(json.dumps(dict(native_cases=NATIVE_CASES, native_drivers=NATIVE_DRIVERS,
                              formal_jobs=formal_plan(args.output), audit_templates=audit_templates()), indent=2))


if __name__ == '__main__':
    main()
