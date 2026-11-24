"""Pin the original maintenance reader to the exact capacity-save capture."""

from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
CAPACITY = HERE.with_name('source-save-capacity')
sys.path.insert(0, str(CAPACITY))
from capacity_inputs import ROOT, configuration as capacity_configuration, retention, row_provider, stage as stage_capacity

POLICY_PROVIDER = '9a88b02b4e1937e1a5e469f7a40a5be84d39a877'
CAPACITY_PROVIDER = '66d34790c7f653759bd99820330f7952da58a95b'
POLICY_SOURCE = 'b0d791ccc4d4e058b710dbf8f46766652b2853df'
POLICY_ROOT = 'sandproof_controller_service::controller::checkpoint::config'
NAMESPACE = 'SandproofSavePolicyJoin'
MODES = ('correct', 'skip-maintenance-validation')
CHECKPOINT = Path('controller-service/src/controller/checkpoint.rs')
CONFIG = Path('controller-service/src/controller/checkpoint/config.rs')
SOURCE_FILES = (retention.CODEC, retention.LIB, CHECKPOINT, CONFIG)


def extend_configuration(row, retained, includes):
    cfg = capacity_configuration(row)
    cfg['opaque'].remove(POLICY_ROOT)
    cfg['opaque'].extend([POLICY_ROOT + '::_', 'serde_json::de::from_slice'])
    cfg['include'] = list(dict.fromkeys([*cfg['include'], *includes]))
    cfg['retained_roots'] = list(dict.fromkeys([*cfg['retained_roots'], *retained]))
    return cfg


def configuration(row):
    return extend_configuration(row, [POLICY_ROOT + '::*::validate'], [
        POLICY_ROOT, POLICY_ROOT + '_using',
        *(POLICY_ROOT + '::' + item for item in
          ('MaintenanceConfig', 'ReadConfigEffects', 'StandardConfigRead', 'read', 'read_using')),
        *(POLICY_ROOT + '::*::' + item for item in ('validate', 'query_i64', 'query_bytes', 'decode_maintenance')),
        *('sandproof_controller_service::controller::capacity::' + item for item in
          ('CapacityConfig', 'invalid_capacity_values', 'add', 'mul', 'SCALAR_BYTES', 'OPTIONS_BYTES', '*::validate')),
        'sandproof_controller_service::JournalError', 'sandproof_controller_service::*::from',
        'sandproof_controller_service::host_contract::*::MAX_ENCODED_BYTES',
        'sandproof_controller_service::journal::MAINTAINED_FORMAT_VERSION',
        'core::ops::range::_::contains', 'core::ops::range::_::start_bound',
        'core::ops::range::_::end_bound', 'core::ops::range::RangeInclusive',
        'core::option::*::and_then', 'rusqlite::row::RowIndex', 'rusqlite::types::from_sql::FromSql',
        'rusqlite::params::Params', 'core::ops::function::FnOnce',
    ])


def named_library(text):
    anchor = 'impl From<serde_json::Error> for JournalError {\n    fn from(error: serde_json::Error) -> Self {'
    provider = retention.pinned(POLICY_PROVIDER, retention.OLD_PACKAGE / 'source-policy-read/extract.py').decode()
    if text.count(anchor) != 1 or 'JournalErrorFromJson' not in provider:
        raise ValueError('the exact #990 JSON conversion name disambiguation is required')
    rename = ('#[charon::rename("JournalErrorFromJson")]\nimpl From<serde_json::Error> for JournalError {'
              '\n    #[charon::rename("from_json")]\n    fn from(error: serde_json::Error) -> Self {')
    return '#![feature(register_tool)]\n#![register_tool(charon)]\n' + text.replace(anchor, rename)


def stage(row, source, mode, *, named=True):
    if mode not in MODES:
        raise ValueError('unknown policy view')
    stage_capacity(row, source, 'correct')
    if mode == 'skip-maintenance-validation':
        config = source / CONFIG
        text = config.read_text()
        original = '    let config: MaintenanceConfig = effects.decode_maintenance(&bytes)?;\n    config.validate()?;\n'
        if text.count(original) != 1:
            raise ValueError('the sole #990 post-decode maintenance validation call is required')
        config.write_text(text.replace(original, original.replace('    config.validate()?;\n', ''), 1))
    if named:
        lib = source / retention.LIB
        lib.write_text(named_library(lib.read_text()))
