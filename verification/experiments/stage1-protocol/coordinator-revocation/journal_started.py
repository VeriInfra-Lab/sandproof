"""Join the admitted WorldTransaction::begin and recording views in the original BEGIN case."""
import json
from pathlib import Path
import shutil

from tools.acceptance_receipts import sha
from tools.proof_batch import closure


PROVIDER = Path('verification/experiments/create-transaction-link/current')
COMPONENT = Path('verification/experiments/stage1-protocol/coordinator-revocation')
MODULE = 'SandproofActualRecord.JournalStartedHeader'
PAGE = 'verification/aeneas/actual-record/SandproofActualRecord/JournalStartedHeader.lean'
TEMPLATE = COMPONENT/'JournalStartedHeader.lean.in'
AUDITS = {'SandproofActualRecordRust.JournalStartedHeader.' + name for name in
          ('erases_to_entire_owner', 'begin_to_first_prepare_refusal')}


def materialize(root, project, begin_files):
    """The original BEGIN materializer already installs the complete transaction provider."""
    binding = json.loads((root/PROVIDER/'rust-source-binding.json').read_bytes())
    overrides = binding['current_file_overrides']
    if not overrides.keys() <= binding['files'].keys():
        raise ValueError('unknown recording Rust input override')
    for name, expected in (binding['files'] | overrides).items():
        if sha(root/name) != expected:
            raise ValueError('current recording Rust input differs: ' + name)

    data = json.loads((root/PROVIDER/'materialization.json').read_bytes())
    recording = {entry['path']: entry['sha256'] for entry in data['modules'].values()}
    if any(recording[name] != begin_files[name] for name in recording.keys() & begin_files.keys()):
        raise ValueError('BEGIN and recording providers disagree on a shared Lean input')
    if PAGE in recording or PAGE in begin_files:
        raise ValueError('joint page already present in producer inputs')
    target = project/PAGE
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(root/TEMPLATE, target)
    files = closure(project, [PAGE], active_view='lakefile.toml')
    known = recording | begin_files | {PAGE: sha(root/TEMPLATE)}
    if not begin_files.keys() <= files.keys() or any(known.get(name) != observed for name, observed in files.items()):
        raise ValueError('joint proof imports differ from the admitted physical views')
    return files


def check_page(root, actual):
    if (actual.get('files', {}).get(PAGE) != sha(root/TEMPLATE)
            or not AUDITS <= set(actual.get('audits', []))):
        raise ValueError('missing BEGIN-to-record page or audits in the begin case')
