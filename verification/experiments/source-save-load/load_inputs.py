"""Pin the prior save, metadata projection, and single original guard mutation."""

import ast
from hashlib import sha256
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
POLICY = HERE.with_name('source-save-policy')
sys.path.insert(0, str(POLICY))
from policy_inputs import CHECKPOINT, CONFIG, POLICY_ROOT, ROOT, SOURCE_FILES, retention, row_provider
from policy_inputs import configuration as policy_configuration, stage as policy_stage
from capacity_check import verify_artifacts

NAMESPACE = 'SandproofSaveLoadJoin'
REFERENCE_NAMESPACE = 'SandproofSourceLoadErrors'
MODES = ('correct', 'early-count-guard')
PARENT = '1d792dff8938981d314d1783e893a5711eeef982'
REFERENCE = 'c887260527b5071fd78a68f3f8d84698fd0c9bd2'
REFERENCE_SOURCE = 'b0d791ccc4d4e058b710dbf8f46766652b2853df'
REFERENCE_DRIVER = retention.OLD_PACKAGE / 'source-load-errors/extract.py'
REFERENCE_METADATA = retention.OLD_PACKAGE / 'source_metadata_capture.py'
SOURCE_CONFIG = Path('controller-service/src/capture/source.rs')
CAPACITY = Path('controller-service/src/controller/capacity.rs')


def historical_source(folder, variant):
    receipt = json.loads((folder / 'result.json').read_bytes())
    if (receipt['complete'] is not True or (receipt['source'], receipt['variant'], receipt['driver']) != (
            REFERENCE_SOURCE, variant, sha256(retention.pinned(REFERENCE, REFERENCE_DRIVER)).hexdigest())
            or [(item['label'], item['exit_code'], item['bounded_stop']) for item in receipt['commands']]
            != [('charon', 0, False), ('aeneas', 0, False)]):
        raise ValueError('complete exact #984 extraction required')
    verify_artifacts(folder, receipt, REFERENCE_NAMESPACE)
    for path in (retention.CODEC, SOURCE_CONFIG):
        if receipt['staged'][str(path)] != retention.sha(folder / 'source' / path):
            raise ValueError('historical staged Rust differs: ' + str(path))
    return receipt


def configuration(row, reference):
    cfg = policy_configuration(row)
    old = historical_source(reference, 'original')['options']
    cfg['opaque'].remove(row.CODEC_ROOT + '::load')
    cfg['opaque'] = list(dict.fromkeys([*cfg['opaque'],
        *(item for item in old['opaque'] if item.startswith(row.CODEC_ROOT)),
        'sandproof_controller_service::controller::capacity::config']))
    cfg['include'] = list(dict.fromkeys([*cfg['include'], *old['include']]))
    return cfg


def historical_transform(path, symbol, dependencies):
    # Use the pinned source transformation itself. Its input is already staged
    # by the current save package, so the old archive unpacker is unnecessary.
    source = retention.pinned(REFERENCE, path)
    statement, = (item for item in ast.parse(source, filename=str(path)).body
                  if isinstance(item, ast.FunctionDef) and item.name == symbol)
    scope = {'__builtins__': {'ValueError': ValueError}, **dependencies}
    exec(compile(ast.Module(body=[statement], type_ignores=[]), str(path), 'exec'), scope)
    return scope[symbol]


def stage(row, source, mode, *, named=True):
    if mode not in MODES:
        raise ValueError('unknown actual source-load variant')
    for path in (SOURCE_CONFIG, CAPACITY, retention.CODEC):
        if (source / path).read_bytes() != retention.pinned(REFERENCE_SOURCE, path):
            raise ValueError('the production metadata input differs from #984: ' + str(path))
    policy_stage(row, source, 'correct', named=named)
    codec = source / retention.CODEC
    base = codec.read_bytes()
    specialize = historical_transform(REFERENCE_METADATA, 'reconstruct_codec',
                                      {'stage_records': lambda _source, _archive, _mode: base})
    prepared = specialize(source, None, 'original')
    if mode == 'early-count-guard':
        mutate = historical_transform(REFERENCE_DRIVER, 'early_count_guard', {})
        prepared = mutate(prepared.decode()).encode()
    codec.write_bytes(prepared)
