"""Finite real pause receipt inventory; incomplete proof/variant lists reject."""
import importlib.util
import json
from pathlib import Path

HERE = Path('verification/experiments/pause-receipt')
BASE = Path('SandboxModel/Lifecycle/PauseReceipt')
PREFIX = 'SandboxModel.Lifecycle.PauseReceipt.'
AUDIT_MODULE = PREFIX+'Audit'
CORRECT_LIBRARY = 'verification/aeneas/capture-handoff/caller'
WRONG_LIBRARY = 'verification/aeneas/pause-receipt/wrong-outcome'
LIBRARIES = (('CaptureCaller', CORRECT_LIBRARY),)
WRONG_AUDIT_MODULE = PREFIX+'WrongAudit'
WRONG_MODULES = {
    str(BASE/'WrongOutcome.lean'): str(HERE/'model/WrongOutcome.lean.in'),
    str(BASE/'WrongAudit.lean'): str(HERE/'model/WrongAudit.lean.in'),
}
_path = Path(__file__).resolve().parents[1]/'pause-dispatch/inventory.py'
_spec = importlib.util.spec_from_file_location('receipt_original_dispatch_inventory', _path)
_shared = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_shared)
regular = _shared.regular
literal_constant = _shared.literal_constant
audits = _shared.audits
declarations = _shared.declarations
check_service = _shared.check_service
ROOTS = _shared.ROOTS
REQUIRED_FUNCTIONS = tuple(dict.fromkeys((*_shared.REQUIRED_FUNCTIONS,
    *((f'effect::{name}', f'effect.{name}') for name in (
      'receipt_pause', 'receipt_awaiting_result', 'receipt_finish',
      'effect_outcome_valid', 'command_result_present', 'acknowledge_effect')))))
_shared._shared.REQUIRED_FUNCTIONS = REQUIRED_FUNCTIONS


def verify_generation(destination, module, namespace, externals):
    selected = _shared.verify_generation(destination, module, namespace, externals)
    raw = json.loads(regular(destination/'capture_handoff.llbc').read_text())
    options = raw['translated']['options']
    if (len(ROOTS)!=20 or ROOTS.count('effect::receipt_pause')!=1
            or options['start_from']!=['crate::'+name for name in ROOTS]
            or options['include']!=['sandbox_controller_core','core::option::*::and_then','core::option::*::as_ref']
            or options['opaque']
            or options['skip_borrowck'] or options['no_typecheck']
            or options['monomorphize'] or options['exclude']):
        raise ValueError('Original twenty receipt roots and compiler checks must remain enabled')
    translation=json.loads(regular(destination/'lean/translation.json').read_text())
    if any(item['is_opaque'] for item in translation['functions']
           if item['rust_name'].startswith('sandbox_controller_core::')):
        raise ValueError('Owned receipt business must remain transparent')
    return selected
REQUIRED_PAGES = frozenset(('Conditions','Model','Observations','Admission','Acceptance'))
REQUIRED_THEOREMS = frozenset(PREFIX+name for name in (
    'same_property_accepts', 'first_acceptance_facts', 'identical_replay',
    'conflicting_replay', 'first_stale_current_check_unchanged',
    'first_success_observes_result', 'first_failure_observes_absence',
    'first_receipt_frame', 'first_failure_preserves_logical',
    'missing_effect_rejects', 'undispatched_committed_rejects',
    'retired_stop_rejects', 'retired_capture_end_rejects', 'retired_capture_drain_rejects',
    'Acceptance.fresh_commit_dispatch_stop_receipts_publish',
))
REQUIRED_WRONG_THEOREMS = frozenset(PREFIX+'WrongOutcome.'+name for name in (
    'actual_wrong_accepts', 'same_property_rejects',
))


def wrong_inventory(source, manifest):
    wrong = manifest.get('wrong', {})
    if (wrong.get('templates') != WRONG_MODULES
            or not isinstance(wrong.get('proofs'),dict)
            or set(wrong['proofs']) != set(WRONG_MODULES.values())
            or not isinstance(wrong.get('required_theorems'),list)
            or len(wrong['required_theorems']) != len(set(wrong['required_theorems']))
            or not REQUIRED_WRONG_THEOREMS.issubset(wrong['required_theorems'])):
        raise ValueError('Exact same-World wrong receipt template inventory is required')
    files, targets, emitted = {}, [], []
    for runtime, original in WRONG_MODULES.items():
        text = regular(source/original).read_text()
        names = declarations(text)
        if names != wrong['proofs'][original] or any(not n.startswith(PREFIX+'WrongOutcome.')for n in names):
            raise ValueError('Wrong receipt declarations changed: '+original)
        targets.extend(names)
        files[runtime] = audits(text)
        emitted.extend(files[runtime])
    if (len(targets)!=len(set(targets)) or not REQUIRED_WRONG_THEOREMS.issubset(targets)
            or not set(wrong['required_theorems']).issubset(targets)
            or emitted!=targets):
        raise ValueError('Complete actual wrong receipt audit closure is required')
    for leaf in ('Types','FunsExternal','Funs'):
        regular(source/(WRONG_LIBRARY+'/CaptureCaller/'+leaf+'.lean'))
        # The independent private project keeps C's original library path and
        # cache. Its actual files are the separately checked wrong originals.
        files[CORRECT_LIBRARY+'/CaptureCaller/'+leaf+'.lean']=[]
    return files


def inventory(source):
    manifest = json.loads(regular(source/HERE/'inventory.json').read_text())
    if (type(manifest.get('schema')) is not int or manifest['schema']!=1
            or manifest.get('namespace')!=PREFIX.rstrip('.')
            or not isinstance(manifest.get('proofs'),dict)or not manifest['proofs']
            or not isinstance(manifest.get('required_theorems'),list)
            or len(manifest['required_theorems'])!=len(set(manifest['required_theorems']))
            or not REQUIRED_THEOREMS.issubset(manifest['required_theorems'])):
        raise ValueError('Final real pause receipt inventory is missing or incomplete')
    expected={str(p.relative_to(source))for p in (source/BASE).glob('*.lean')if p.name!='Audit.lean'}
    if (set(manifest['proofs'])!=expected
            or not {str(BASE/(name+'.lean'))for name in REQUIRED_PAGES}.issubset(expected)):
        raise ValueError('Every correct receipt proof page must be declared')
    files,targets={},[]
    for name,expected_names in manifest['proofs'].items():
        path=Path(name)
        if path.parent!=BASE or path.suffix!='.lean' or not isinstance(expected_names,list):
            raise ValueError('Unexpected correct receipt proof path')
        text=regular(source/path).read_text()
        names=declarations(text)
        if names!=expected_names or any(not n.startswith(PREFIX)for n in names):
            raise ValueError('Correct receipt declarations changed: '+name)
        targets.extend(names)
        files[name]=audits(text)
    if (not targets or len(targets)!=len(set(targets))
            or not REQUIRED_THEOREMS.issubset(targets)
            or not set(manifest['required_theorems']).issubset(targets)):
        raise ValueError('Required actual receipt obligations are missing')
    audit=str(BASE/'Audit.lean')
    if audits(regular(source/audit).read_text())!=targets:
        raise ValueError('Receipt audit must cover its complete declaration closure')
    files[audit]=targets
    for leaf in ('Types','FunsExternal','Funs'):
        name=CORRECT_LIBRARY+'/CaptureCaller/'+leaf+'.lean'
        regular(source/name)
        files[name]=[]
    wrong_inventory(source,manifest)
    return files,manifest
