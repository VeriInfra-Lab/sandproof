"""Use the pinned retention and SQL proof inputs without copying their contracts."""

from pathlib import Path
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
ROW_PARENT = '8f94cb4dc947c09c46981931f5e7d98b144de66d'
NAMESPACE = 'SandproofSaveRetentionRows'
MODES = ('correct', 'binding-only', 'parent')
RETENTION = HERE.with_name('source-save-retention')
sys.path.insert(0, str(RETENTION))
import inputs as retention


def row_provider(parent):
    provider = parent / 'row-provider'
    provider.mkdir(mode=0o700)
    archive = subprocess.check_output(['git', 'archive', ROW_PARENT], cwd=ROOT)
    from io import BytesIO
    with tarfile.open(fileobj=BytesIO(archive)) as inputs:
        inputs.extractall(provider, filter='data')
    sys.path.insert(0, str(provider / retention.OLD_PACKAGE))
    import source_row_sql_composition as row
    return row


def stage(row, source, mode):
    if mode not in MODES:
        raise ValueError('unknown joint view')
    if row.stage(source, 'core') != '7809df9a648357a43d525605dc0d2ba36cf29a0b6aed7c956e9d92d0a93c41c5':
        raise ValueError('the exact reviewed #969 row SQL staging is required')
    path = source / retention.CODEC
    text = path.read_text()
    old = 'save_using::<_, _, _, _, _, _, JournalSourceSave>(connection, sources, max_sessions)'
    if text.count(old) != 1 or text.count('fn retains_with_saved_keys<') != 1:
        raise ValueError('the original save and retention providers must occur once')
    text = text.replace(old, 'save_sql_retaining_using::<_, _, _, JournalSourceSave, JournalSavedKeys>(connection, sources, max_sessions)')
    if mode == 'binding-only':
        match = 'if Check::same_work(next, record) {'
        if text.count(match) != 1:
            raise ValueError('the original complete comparison must occur once')
        text = text.replace(match, 'if Check::same_binding(next, record) {')
    path.write_text(text + '\n' + (HERE / 'bridge.rs.in').read_text())


def configuration(row, mode):
    root = row.CODEC_ROOT + ('::save' if mode == 'parent' else '::save_sql_retaining_using')
    return dict(roots=[root], retained_roots=[*row.CONFIG['retained_roots'], 'crate::proof_source_work_eq'],
                include=list(dict.fromkeys([*row.CONFIG['include'], 'core::tuple'])),
                opaque=[*row.CONFIG['opaque'], 'serde_json::error::Error', *[row.CODEC_ROOT + '::' + name for name in
                        ('load', 'reserved_records', 'encode_candidate_records')]])
