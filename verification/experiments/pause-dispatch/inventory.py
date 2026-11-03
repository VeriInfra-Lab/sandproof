"""Finite Stop inventory using the existing declaration/service validators.

The actual required theorem manifest must be supplied after the business proofs
settle. Missing or stale inventory is an error, never a smaller passing batch.
"""
import importlib.util
import json
from pathlib import Path

HERE = Path('verification/experiments/pause-dispatch')
BASE = Path('SandboxModel/Lifecycle/PauseDispatch')
PREFIX = 'SandboxModel.Lifecycle.PauseDispatch.'
AUDIT_MODULE = PREFIX+'Audit'
WRONG_FILE = HERE/'model/WrongBinding.lean'
WRONG_NAMESPACE = PREFIX+'WrongBinding'
LIBRARIES = (
    ('CaptureCaller', 'verification/aeneas/capture-handoff/caller'),
    ('PauseDispatchWrong', 'verification/aeneas/pause-dispatch/wrong-binding'),
)

# Reuse the existing reviewed declaration reader, actual-source validator and
# diagnostic checker; do not create another Lean parser or service adapter.
_path = Path(__file__).resolve().parents[1]/'pause-publication/inventory.py'
_spec = importlib.util.spec_from_file_location('pause_dispatch_shared_existing_inventory', _path)
_shared = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_shared)
regular = _shared.regular
literal_constant = _shared.literal_constant
audits = _shared.audits
declarations = _shared.declarations
check_service = _shared.check_service
CAPTURE_METHODS = _shared.CAPTURE_METHODS
ROOTS = tuple('capture::*::'+name for name in CAPTURE_METHODS) + (
    'request::handle_compensation', 'worker::late_capture::record',
    'publication::handle_publish_paused', 'worker::pause_stop::record',
    'effect::dispatch_stop', 'effect::acknowledge_effect',
    'maintenance::released_execution::*::command',
)
REQUIRED_FUNCTIONS = tuple(dict.fromkeys((
    *_shared.REQUIRED_FUNCTIONS,
    *((f'worker::pause_stop::{name}', f'worker.pause_stop.{name}')
      for name in ('record', 'execute', 'commit_snapshot', 'stop_vm')),
    *((f'worker::{name}', f'worker.{name}')
      for name in ('current_effect_id', 'contains', 'effect_awaiting', 'current_plan')),
    ('worker::contains', 'worker.contains_loop'),
    ('worker::contains', 'worker.contains_loop.body'),
)))
REQUIRED_PAGES = frozenset((
    'Conditions', 'Frames', 'Acknowledgements', 'Safety', 'Composition',
    'ReceiptPair', 'Ready', 'Publication', 'Witness', 'Acceptance',
))
# This isolated module instance supplies the new slice's finite rust/lean dual
# keys to the existing validator. Repeated Rust names for loop bodies remain
# distinct Lean definitions; no name-only dictionary discards them.
_shared.REQUIRED_FUNCTIONS = REQUIRED_FUNCTIONS


def verify_generation(destination, module, namespace, externals):
    selected = _shared.verify_generation(destination, module, namespace, externals)
    translation = json.loads(regular(destination/'lean/translation.json').read_text())
    expected_name = namespace+'.core.option.Option.as_ref'
    rows = [item for item in translation['functions'] if item.get('lean_name') == expected_name]
    text = _shared.without_comments(regular(destination/'lean'/module/'Funs.lean').read_text())
    if (len(rows) != 1 or rows[0].get('is_opaque') is not False
            or rows[0].get('is_local') is not False
            or rows[0].get('lean_file') != module+'/Funs.lean'
            or not rows[0].get('source', {}).get('file', '').endswith('/core/src/option.rs')
            or not _shared.re.search(r'\bdef\s+core\.option\.Option\.as_ref(?=\s|[({])', text)):
        raise ValueError('Original standard Option::as_ref must remain transparent')
    selected[expected_name] = rows[0]
    return selected


def verify_wrong_generation(destination, module, namespace, externals):
    original = _shared.REQUIRED_FUNCTIONS
    try:
        _shared.REQUIRED_FUNCTIONS = (*original, ('effect::misbound_stop', 'effect.misbound_stop'))
        return verify_generation(destination, module, namespace, externals)
    finally:
        _shared.REQUIRED_FUNCTIONS = original



def inventory(source):
    manifest = json.loads(regular(source/HERE/'inventory.json').read_text())
    if (type(manifest.get('schema')) is not int or manifest['schema'] != 1
            or manifest.get('namespace') != PREFIX.rstrip('.')
            or not isinstance(manifest.get('proofs'), dict) or not manifest['proofs']
            or not isinstance(manifest.get('required_theorems'), list)
            or not manifest['required_theorems']
            or not all(isinstance(name, str) for name in manifest['required_theorems'])
            or len(manifest['required_theorems']) != len(set(manifest['required_theorems']))):
        raise ValueError('Final pause-dispatch proof inventory is missing or malformed')
    expected_files = {str(path.relative_to(source)) for path in (source/BASE).glob('*.lean')
                      if path.name != 'Audit.lean'}
    if set(manifest['proofs']) != expected_files:
        raise ValueError('Final inventory does not cover every pause dispatch proof file')
    if not {str(BASE/(name+'.lean')) for name in REQUIRED_PAGES} <= expected_files:
        raise ValueError('Missing a required actual pause dispatch proof page')
    files, targets = {}, []
    for name, expected in manifest['proofs'].items():
        path = Path(name)
        if path.parent != BASE or path.suffix != '.lean' or not isinstance(expected, list):
            raise ValueError('Unexpected pause dispatch proof path or declaration list')
        text = regular(source/path).read_text()
        actual = declarations(text)
        if actual != expected or any(not target.startswith(PREFIX) for target in actual):
            raise ValueError('Pause dispatch declaration inventory changed: '+name)
        files[name] = audits(text)
        targets.extend(actual)
    if (not targets or len(targets) != len(set(targets))
            or not set(manifest['required_theorems']).issubset(targets)):
        raise ValueError('Duplicate or missing required pause dispatch theorem')
    audit = str(BASE/'Audit.lean')
    if audits(regular(source/audit).read_text()) != targets:
        raise ValueError('Pause dispatch audit does not cover the complete declared closure')
    files[audit] = targets
    wrong = manifest.get('wrong', {})
    wrong_file = str(WRONG_FILE)
    wrong_namespace = WRONG_NAMESPACE
    if (wrong.get('file') != wrong_file or wrong.get('namespace') != wrong_namespace
            or not isinstance(wrong.get('audits'), list) or not wrong['audits']
            or not isinstance(wrong.get('required_theorems'), list) or not wrong['required_theorems']
            or not all(isinstance(name, str) for name in wrong['required_theorems'])
            or len(wrong['required_theorems']) != len(set(wrong['required_theorems']))):
        raise ValueError('Missing final real wrong-binding audit inventory')
    text = regular(source/wrong_file).read_text()
    expected_wrong = declarations(text)
    if (not expected_wrong or len(expected_wrong) != len(set(expected_wrong))
            or any(not target.startswith(wrong_namespace+'.') for target in expected_wrong)
            or not set(wrong['required_theorems']).issubset(expected_wrong)
            or wrong['audits'] != expected_wrong or audits(text) != expected_wrong):
        raise ValueError('Wrong-binding declarations or audits changed')
    files[wrong_file] = expected_wrong
    for module, root in LIBRARIES:
        for leaf in ('Types', 'FunsExternal', 'Funs'):
            name = root+'/'+module+'/'+leaf+'.lean'
            regular(source/name)
            files[name] = []
    return files, manifest

