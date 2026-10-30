"""Finite Stop inventory using the existing declaration/service validators.

The actual required theorem manifest must be supplied after the business proofs
settle. Missing or stale inventory is an error, never a smaller passing batch.
"""
import importlib.util
import json
from pathlib import Path

HERE = Path('verification/experiments/pause-stop')
BASE = Path('SandboxModel/Lifecycle/PauseStop')
PREFIX = 'SandboxModel.Lifecycle.PauseStop.'
AUDIT_MODULE = PREFIX+'Audit'
WRONG_FILE = HERE/'model/IgnoreHandoff.lean'
WRONG_NAMESPACE = PREFIX+'IgnoreHandoff'
LIBRARIES = (
    ('CaptureCaller', 'verification/aeneas/capture-handoff/caller'),
    ('PauseHandoffWrong', 'verification/aeneas/pause-stop/wrong-permit'),
)

# Reuse the existing reviewed declaration reader, actual-source validator and
# diagnostic checker; do not create another Lean parser or service adapter.
_path = Path(__file__).resolve().parents[1]/'pause-publication/inventory.py'
_spec = importlib.util.spec_from_file_location('pause_stop_shared_existing_inventory', _path)
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
)
REQUIRED_FUNCTIONS = (
    *((f'worker::pause_stop::{name}', f'worker.pause_stop.{name}')
      for name in ('record', 'execute', 'commit_snapshot', 'stop_vm')),
    *((f'worker::{name}', f'worker.{name}')
      for name in ('current_effect_id', 'contains', 'effect_awaiting', 'current_plan')),
    ('worker::contains', 'worker.contains_loop'),
    ('worker::contains', 'worker.contains_loop.body'),
)
# This isolated module instance supplies the new slice's finite rust/lean dual
# keys to the existing validator. Repeated Rust names for loop bodies remain
# distinct Lean definitions; no name-only dictionary discards them.
_shared.REQUIRED_FUNCTIONS = REQUIRED_FUNCTIONS
verify_generation = _shared.verify_generation


def inventory(source):
    manifest = json.loads(regular(source/HERE/'inventory.json').read_text())
    if (type(manifest.get('schema')) is not int or manifest['schema'] != 1
            or manifest.get('namespace') != PREFIX.rstrip('.')
            or not isinstance(manifest.get('proofs'), dict) or not manifest['proofs']
            or not isinstance(manifest.get('required_theorems'), list)
            or not manifest['required_theorems']
            or not all(isinstance(name, str) for name in manifest['required_theorems'])
            or len(manifest['required_theorems']) != len(set(manifest['required_theorems']))):
        raise ValueError('Final pause-stop proof inventory is missing or malformed')
    expected_files = {str(path.relative_to(source)) for path in (source/BASE).glob('*.lean')
                      if path.name != 'Audit.lean'}
    if set(manifest['proofs']) != expected_files:
        raise ValueError('Final inventory does not cover every pause Stop proof file')
    files, targets = {}, []
    for name, expected in manifest['proofs'].items():
        path = Path(name)
        if path.parent != BASE or path.suffix != '.lean' or not isinstance(expected, list):
            raise ValueError('Unexpected pause Stop proof path or declaration list')
        text = regular(source/path).read_text()
        actual = declarations(text)
        if actual != expected or any(not target.startswith(PREFIX) for target in actual):
            raise ValueError('Pause Stop declaration inventory changed: '+name)
        files[name] = audits(text)
        targets.extend(actual)
    if (not targets or len(targets) != len(set(targets))
            or not set(manifest['required_theorems']).issubset(targets)):
        raise ValueError('Duplicate or missing required pause Stop theorem')
    audit = str(BASE/'Audit.lean')
    if audits(regular(source/audit).read_text()) != targets:
        raise ValueError('Pause Stop audit does not cover the complete declared closure')
    files[audit] = targets
    wrong = manifest.get('wrong', {})
    wrong_file = str(WRONG_FILE)
    wrong_namespace = WRONG_NAMESPACE
    if (wrong.get('file') != wrong_file or wrong.get('namespace') != wrong_namespace
            or not isinstance(wrong.get('audits'), list) or not wrong['audits']
            or not isinstance(wrong.get('required_theorems'), list) or not wrong['required_theorems']
            or not all(isinstance(name, str) for name in wrong['required_theorems'])
            or len(wrong['required_theorems']) != len(set(wrong['required_theorems']))):
        raise ValueError('Missing final real wrong-handoff audit inventory')
    text = regular(source/wrong_file).read_text()
    expected_wrong = declarations(text)
    if (not expected_wrong or len(expected_wrong) != len(set(expected_wrong))
            or any(not target.startswith(wrong_namespace+'.') for target in expected_wrong)
            or not set(wrong['required_theorems']).issubset(expected_wrong)
            or wrong['audits'] != expected_wrong or audits(text) != expected_wrong):
        raise ValueError('Wrong-handoff declarations or audits changed')
    files[wrong_file] = expected_wrong
    for module, root in LIBRARIES:
        for leaf in ('Types', 'FunsExternal', 'Funs'):
            name = root+'/'+module+'/'+leaf+'.lean'
            regular(source/name)
            files[name] = []
    return files, manifest

