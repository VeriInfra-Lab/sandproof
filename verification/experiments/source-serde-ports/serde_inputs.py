"""Freeze actual typed Serde call/required-option selection and its one isolated mutant."""
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
CANONICAL = HERE.with_name('source-save-load-canonical')
DECISION = HERE.with_name('source-save-load-decision')
sys.path.insert(0, str(CANONICAL))
from canonical_inputs import ROOT, SOURCE_FILES as parent_files, retention, row_provider, stage as parent_stage
from binding import CODEC, REQUIRED, CALLS

PARENT = 'f7462e126a6e50a7beec8ab38e4b77cce5324211'
SOURCE_WORK = Path('controller-service/src/controller/lifecycle_inbox/source_work.rs')
SOURCE_FILES = tuple(dict.fromkeys((*parent_files, SOURCE_WORK)))
NAMESPACE = 'SandproofSourceSerdePorts'
MODES = ('correct','drop-required-option-value')
PROFILES = ('required-only','decode-only','call-admission')
FORWARD = 'Option::<T>::deserialize(deserializer)'
DROPPED = 'match Option::<T>::deserialize(deserializer) { Ok(_) => Ok(None), Err(error) => Err(error) }'


def configuration(profile):
    if profile not in PROFILES: raise ValueError('unknown selected concrete Serde proof profile')
    names = ([REQUIRED] if profile=='required-only' else [CODEC+'decode_source_payload']
             if profile=='decode-only' else [CODEC+name for name in CALLS])
    return dict(roots=names, include=names, opaque=['sandproof_controller_service','sandbox_controller_core',
                'sandbox_protocol','serde_json','serde_core','serde'])


def stage(row, source, mode, *, named=True):
    if mode not in MODES: raise ValueError('unknown isolated required_option view')
    parent_stage(row, source, 'correct', named=named)
    path=source/SOURCE_WORK
    original=retention.pinned(retention.SOURCE,SOURCE_WORK)
    if path.read_bytes()!=original: raise ValueError('the original SourceWorkRecord and required_option source must match')
    text=path.read_text()
    if text.count(FORWARD)!=1: raise ValueError('the exact original required_option forwarding is required')
    if mode=='drop-required-option-value': path.write_text(text.replace(FORWARD,DROPPED,1))
