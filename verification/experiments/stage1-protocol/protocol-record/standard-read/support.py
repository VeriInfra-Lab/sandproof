"""Two actual StandardRecord reads, with a fixed query-only translation scope.

Preparation constants only. No capture, proof or acceptance runs on import.
"""
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


# The first real capture reused Create's whole controller/protocol includes and
# pulled non-called SQL/Serde trait methods into this query-only graph. Keep
# core World/Clone/Store transparent, but select the service methods precisely.
# In fixed Charon b104e24, * matches one name element (including an impl); the
# pattern also selects descendants, so each method's own closures stay visible.
INCLUDE = [
    'sandbox_controller_core',
    *ROOTS,
    # Retain the three original value carriers, including private fields.
    # This does not include their impl methods or additional SQL operations.
    PREFIX + 'StandardRecord',
    'sandproof_controller_service::controller::transaction::WorldTransaction',
    'sandproof_controller_service::controller::transaction::CommittedWorld',
    PREFIX + '*::releasing',
    PREFIX + '*::current_operation',
    'sandproof_controller_service::controller::transaction::*::world',
    'core::option::*::as_ref',
    'core::option::*::and_then',
    'core::option::*::map',
    # Preserve the original transaction value carrier, not SQL/commit methods.
    'rusqlite::transaction::Transaction',
]
OPAQUE = ['rusqlite::error::Error']
