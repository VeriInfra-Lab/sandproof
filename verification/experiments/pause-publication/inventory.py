"""Finite source/audit inventory for the real pause publisher.

The final inventory.json is written only after the business proofs settle.
Missing or stale inventory is an error, never a smaller successful proof batch.
"""
import ast
import json
from pathlib import Path
import re

HERE = Path('verification/experiments/pause-publication')
BASE = Path('SandboxModel/Lifecycle/PausePublication')
PREFIX = 'SandboxModel.Lifecycle.PausePublication.'
LIBRARIES = (
    ('CaptureCaller', 'verification/aeneas/capture-handoff/caller'),
    ('PauseStopWrong', 'verification/aeneas/pause-publication/wrong-stop'),
)
CAPTURE_METHODS = (
    'accepted', 'registration_dispatched', 'register_bundle', 'observe_bundle',
    'ordinary_failure', 'terminate', 'may_stop_original', 'accepts_registration',
    'commit', 'eligibility', 'has_capture_work', 'has_unload_source',
)
ROOTS = tuple('capture::*::'+name for name in CAPTURE_METHODS) + (
    'request::handle_compensation', 'worker::late_capture::record',
    'publication::handle_publish_paused',
)
REQUIRED_FUNCTIONS = (
    ('request::handle_compensation', 'request.handle_compensation'),
    ('worker::late_capture::record', 'worker.late_capture.record'),
    ('worker::late_capture::record_inner', 'worker.late_capture.record_inner'),
    ('worker::late_capture::commit', 'worker.late_capture.commit'),
    ('publication::handle_publish_paused', 'publication.handle_publish_paused'),
    ('publication::publish_paused', 'publication.publish_paused'),
    ('publication::update_operation_result', 'publication.update_operation_result'),
    ('effect::effect_succeeded_check', 'effect.effect_succeeded_check'),
    *((f'publication::{name}', f'publication.{name}{suffix}')
      for name in ('indexed_vms_closed', 'indexed_slots_closed')
      for suffix in ('', '_loop', '_loop.body')),
)


def regular(path):
    if path.is_symlink() or not path.is_file():
        raise ValueError('Missing or linked required source: '+str(path))
    return path


def literal_constant(path, name):
    """Reuse the original reviewed literal without importing its CLI modules."""
    tree = ast.parse(regular(path).read_text())
    found = [node.value for node in tree.body if isinstance(node, ast.Assign)
             and any(isinstance(target, ast.Name) and target.id == name for target in node.targets)]
    if len(found) != 1:
        raise ValueError('Missing or ambiguous original reviewed constant: '+name)
    try:
        return ast.literal_eval(found[0])
    except (ValueError, TypeError) as error:
        raise ValueError('Original reviewed constant is no longer a literal: '+name) from error


def without_comments(text):
    return re.sub(r'/\-.*?\-/', '', text, flags=re.S)


def audits(text):
    return re.findall(r'^#audit_controller_proof (\S+)$', without_comments(text), re.M)


def declarations(text):
    return re.findall(r'^(?:@\[[^\]]+\]\s*)?(?:theorem|lemma)\s+([\w.]+)',
                      without_comments(text), re.M)


def inventory(source):
    manifest = json.loads(regular(source/HERE/'inventory.json').read_text())
    if (type(manifest.get('schema')) is not int or manifest['schema'] != 1
            or manifest.get('namespace') != PREFIX.rstrip('.')
            or not isinstance(manifest.get('proofs'), dict) or not manifest['proofs']
            or not isinstance(manifest.get('required_theorems'), list)
            or not manifest['required_theorems']
            or not all(isinstance(name, str) for name in manifest['required_theorems'])
            or len(manifest['required_theorems']) != len(set(manifest['required_theorems']))):
        raise ValueError('Final pause-publication proof inventory is missing or malformed')
    expected_files = {str(path.relative_to(source)) for path in (source/BASE).glob('*.lean')
                      if path.name != 'Audit.lean'}
    if set(manifest['proofs']) != expected_files:
        raise ValueError('Final inventory does not cover every publication proof file')
    files, targets = {}, []
    for name, expected in manifest['proofs'].items():
        path = Path(name)
        if path.parent != BASE or path.suffix != '.lean' or not isinstance(expected, list):
            raise ValueError('Unexpected publication proof path or declaration list')
        text = regular(source/path).read_text()
        actual = [PREFIX+name for name in declarations(text)]
        if actual != expected:
            raise ValueError('Publication declaration inventory changed: '+name)
        files[name] = audits(text)
        targets.extend(actual)
    if (not targets or len(targets) != len(set(targets))
            or not set(manifest['required_theorems']).issubset(targets)):
        raise ValueError('Duplicate or missing required publication theorem')
    audit = str(BASE/'Audit.lean')
    if audits(regular(source/audit).read_text()) != targets:
        raise ValueError('Publication audit does not cover the complete declared closure')
    files[audit] = targets
    wrong = manifest.get('wrong', {})
    wrong_file = str(HERE/'model/IgnoreStop.lean')
    wrong_namespace = PREFIX+'IgnoreStop'
    if (wrong.get('file') != wrong_file or wrong.get('namespace') != wrong_namespace
            or not isinstance(wrong.get('audits'), list) or not wrong['audits']):
        raise ValueError('Missing final real wrong-Stop audit inventory')
    text = regular(source/wrong_file).read_text()
    expected_wrong = [wrong_namespace+'.'+name for name in declarations(text)]
    if (not expected_wrong or len(expected_wrong) != len(set(expected_wrong))
            or wrong['audits'] != expected_wrong or audits(text) != expected_wrong):
        raise ValueError('Wrong-Stop declarations or audits changed')
    files[wrong_file] = expected_wrong
    for module, root in LIBRARIES:
        for leaf in ('Types', 'FunsExternal', 'Funs'):
            name = root+'/'+module+'/'+leaf+'.lean'
            regular(source/name)
            files[name] = []
    return files, manifest


def verify_generation(destination, module, namespace, externals):
    llbc = json.loads(regular(destination/'capture_handoff.llbc').read_text())
    if llbc.get('has_errors') is not False:
        raise ValueError('Charon errors cannot stand for a property counterexample')
    translation = json.loads(regular(destination/'lean/translation.json').read_text())
    functions = translation['functions']
    text = without_comments(regular(destination/'lean'/module/'Funs.lean').read_text())
    selected = {}
    for rust, lean in REQUIRED_FUNCTIONS:
        rust = 'sandbox_controller_core::'+rust
        matches = [item for item in functions if item['rust_name'] == rust
                   and item['lean_name'] == namespace+'.'+lean]
        if (len(matches) != 1 or matches[0].get('is_opaque') is not False
                or matches[0].get('is_local') is not True
                or matches[0].get('lean_file') != module+'/Funs.lean'
                or not re.search(r'\bdef\s+'+re.escape(lean)+r'(?=\s|[({])', text)):
            raise ValueError('Required actual local definition missing or opaque: '+rust+' / '+lean)
        selected[namespace+'.'+lean] = matches[0]
    for name in CAPTURE_METHODS:
        matches = [item for item in functions if item['rust_name'].endswith('::'+name)
                   and item.get('source', {}).get('file', '').endswith('controller-core/src/capture.rs')
                   and item.get('lean_name', '').startswith(namespace+'.')]
        if (len(matches) != 1 or matches[0].get('is_opaque') is not False
                or matches[0].get('is_local') is not True
                or matches[0].get('lean_file') != module+'/Funs.lean'
                or not re.search(r'\bdef\s+'+re.escape(matches[0]['lean_name'].removeprefix(namespace+'.'))
                                 +r'(?=\s|[({])', text)):
            raise ValueError('Real capture method missing, opaque or ambiguous: '+name)
        selected[matches[0]['lean_name']] = matches[0]
    template = regular(destination/'lean'/module/'FunsExternal_Template.lean')
    names = re.findall(r'\baxiom\s+([\w.]+)', without_comments(template.read_text()))
    if set(names) != externals or len(names) != len(externals):
        raise ValueError('Unreviewed or duplicate external in actual publication extraction')
    return selected


def check_service(source, files, text, sha):
    actual = []
    for line in text.splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if row.get('request', {}).get('tool') == 'lean_diagnostic_messages':
            actual.append(row)
    if len(actual) != len(files):
        raise ValueError('Missing actual-file diagnostics')
    seen = set()
    for row in actual:
        name = row['request']['arguments']['file_path']
        if name not in files or name in seen:
            raise ValueError('Unexpected or duplicate actual file')
        seen.add(name)
        value = row['result'].get('result', row['result'])
        if (row['stale'] or value.get('success') is not True or value.get('partial')
                or value.get('timed_out') or value.get('failed_dependencies')
                or row['revision']['file_sha256'] != sha(source/name)
                or row['request']['expected_sha256'] != sha(source/name)):
            raise ValueError('Incomplete or changed actual file: '+name)
        items = value.get('items', [])
        if any(item.get('severity') in ('error', 'warning') for item in items):
            raise ValueError('Unresolved actual-file diagnostics: '+name)
        checked = [item['message'] for item in items if item['message'].startswith('Checked ')]
        parsed = [re.fullmatch(r'Checked (\S+): ([0-3]) standard logical dependencies', item)
                  for item in checked]
        if any(item is None for item in parsed) or [item.group(1) for item in parsed] != files[name]:
            raise ValueError('Missing, changed or extra actual proof audit: '+name)
    return actual
