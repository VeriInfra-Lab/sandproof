"""Two actual StandardRecord reads, using the existing standard-Create scope.

Preparation constants only. No capture, proof or acceptance runs on import.
"""
import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[5]
HERE = Path('verification/experiments/stage1-protocol/protocol-record/standard-read')
SOURCE = Path('controller-service/src/controller/lifecycle_inbox/recording.rs')
GEN = HERE / 'generated/SandproofStandardRecord'
NAMESPACE = 'SandproofStandardRecordRust'
PREFIX = 'sandproof_controller_service::controller::lifecycle_inbox::recording::'
ROOTS = [PREFIX + name for name in (
    'proof_standard_record_releasing', 'proof_standard_record_current_operation')]
METHODS = ['releasing', 'current_operation']


def standard_create_scope():
    path = ROOT / 'verification/experiments/stage1-protocol/create-standard-adapter/support.py'
    spec = importlib.util.spec_from_file_location('record_existing_create_scope', path)
    original = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(original)
    return original


original = standard_create_scope()
INCLUDE = list(original.INCLUDE)
OPAQUE = list(original.OPAQUE)
