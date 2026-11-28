"""Check the public journal at the original BEGIN call and recording owner."""
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import sys

from tools.proof_batch import closure

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import codec_binding
import codec_reads
import codec_receipt as codec
import journal_entry
import journal_decision
from wiring import require

RECORD = Path('verification/aeneas/actual-record/SandproofActualRecord')
PAGE = str(RECORD/'LoadValidation.lean')
MODULE = 'SandproofActualRecord.LoadValidation'
PKG = codec.PACKAGE/'public-begin'
NEW = ('PublicHandle', 'PublicCalls', 'PublicBegin', 'PublicReplay', 'PublicLoaded', 'PublicUnload',
       'PublicUnloadRefusal', 'RecordCodecPorts', 'PublicUnloadSave', 'ValidationErrors', 'LoadValidation')
AUDITS = {'SandproofActualRecordRust.PublicBegin.'+n for n in (
    'begin_refusal', 'world_refusal', 'preparation_refusal', 'sqlite_refusal', 'allocator_refusal',
    'changed_version_refusal', 'started_reply', 'first_record_read_refusal')}
AUDITS.update('SandproofActualRecordRust.PublicReplay.'+n for n in (
    'hook_return', 'retained', 'conflict', 'mapped_unloads', 'changed_capture', 'journal_error'))
AUDITS.update('SandproofActualRecordRust.PublicLoaded.'+n for n in (
    'replay_return', 'retained', 'conflict', 'load_refusal'))
AUDITS.update('SandproofActualRecordRust.PublicUnload.'+n for n in (
    'append_current', 'record_after_save', 'commit_reply', 'fresh_unload', 'mapped_append_witness',
    'commit_validation_error', 'fresh_sqlite_outcome', 'recorded_commit_dispatch', 'fresh_sqlite_dispatch',
    'save_encode_returned', 'save_encoding_refusals', 'fresh_save_refusal'))
AUDITS.update('SandproofActualRecordRust.PublicUnloadRefusal.'+n for n in (
    'refusal_kind', 'fresh_refusal', 'mapped_guard_order', 'missing_session', 'missing_session_guard_order'))
AUDITS.update('SandproofActualRecordRust.RecordCodecPorts.'+n for n in (
    'load_save_encode', 'live_encode', 'missing_transaction', 'save_stops',
    'record_stops', 'journal_stops', 'fresh_encode_stops'))
AUDITS.update('SandproofActualRecordRust.PublicUnloadSave.'+n for n in (
    'save_digest_returned', 'save_update_returned', 'bounded_save_result', 'fresh_write_dispatch'))
AUDITS.add('SandboxModel.Stage1Protocol.InboxValidation.validate_error')
AUDITS.update('SandproofActualRecordRust.PublicSaveValidation.'+n for n in (
    'save_validation_error', 'validation_witnesses', 'fresh_validation_refusal'))
AUDITS.update(('SandboxModel.Stage1Protocol.InboxLoad.load_at_validation',
               'SandboxModel.Stage1Protocol.InboxLoad.Values.load_validation_witnesses',
               'SandproofActualRecordRust.PublicLoadValidation.record_load_stops',
               'SandproofActualRecordRust.PublicLoadValidation.journal_load_validation'))

APPEND_WITNESSES = {
    str(journal_decision.PROTOCOL/(name+'.lean')): codec.AREA/'inbox-append'/(name+'.lean.in')
    for name in ('InboxAppendValues', 'InboxAppendSelections')
}
SHA = {'Types': '6c311fa06db63d48c43ca33fe50f096d186e65165a847fd2e7532df29aa05113',
       'Funs': '126609d05e094ecfa445c512bfccd4e11a85c2cc796a67df09b14a5d43f861af'}
BINDERS = {'Types': b'variable [FileHandle]',
           'Funs': b'variable [FileHandle] [PublicCalls] [SandproofStandardRecordCodec.DecodeCalls]'}
NAMESPACE = b'namespace SandproofActualRecordRust\n'
PROVIDER = Path('verification/experiments/create-transaction-link/current')


def bound_page(name, public, previous):
    require(codec.digest(public) == SHA[name] and public.count(NAMESPACE) == 1,
            'public journal translation changed: '+name)
    # Each previous generated declaration (including its Rust source annotation) is retained.
    blocks = re.split(rb'^/-- ', previous.split(b'\nend SandproofActualRecordRust')[0], flags=re.M)[1:]
    require(bool(blocks) and all(b'/-- '+block.rstrip() in public for block in blocks),
            'public journal omits a recording provider declaration')
    return public.replace(NAMESPACE, NAMESPACE+b'\n'+BINDERS[name]+b'\n', 1)


def project_inputs(root, journal, decoder, captures):
    """Describe every byte added to or superseding the original BEGIN project."""
    manifest = json.loads((root/PROVIDER/'materialization.json').read_bytes())['modules']
    result = {}
    for name, extra in [('Types', 'PublicHandle'), ('Funs', 'PublicCalls')]:
        for stem in (name, name+'External'):
            entry = manifest['SandproofActualRecord.'+stem]
            old = (root/entry['source']).read_bytes()
            require(codec.digest(old) == entry['sha256'] and Path(entry['path']) == RECORD/(stem+'.lean'),
                    'original recording page changed: '+stem)
            if stem == name:
                content = bound_page(name, (journal/'SandproofActualRecord'/(name+'.lean')).read_bytes(), old)
            else:
                content = ('import SandproofActualRecord.'+extra+'\n').encode()+old
            result[entry['path']] = content
    for name, template in APPEND_WITNESSES.items():
        result[name] = (root/template).read_bytes()
    for name in NEW:
        result[str(RECORD/(name+'.lean'))] = (root/PKG/(name+'.lean.in')).read_bytes()
    for name in ('Types', 'Funs', 'TypesExternal', 'FunsExternal'):
        file = name+'.lean'
        if name in ('Types','Funs'):
            data = (decoder/codec.LIB/file).read_bytes()
            require(codec.digest(data) == codec.RAW_PAGES[codec.LIB+'/'+file], 'original decoder differs: '+name)
            if name == 'Funs':
                data = codec.bound_functions(data)
        else:
            data = (root/codec.TEMPLATES/(file+'.in')).read_bytes()
        result[str(codec.FOLDER/codec.LIB/file)] = data
    request = manifest['SandproofStage1Request.Types']
    provider = (root/request['source']).read_bytes()
    require(Path(request['path']) == journal_decision.REQUEST and codec.digest(provider) == request['sha256'],
            'missing transaction request type provider')
    result.update(journal_decision.project_inputs(root, captures, provider))
    return result


def prepare(root, project, folder, original_files, tools, head):
    """Use the shared batch lock and the original BEGIN project, preserving its import closure."""
    spec = importlib.util.spec_from_file_location('public_journal_driver', root/codec.PACKAGE/'check.py')
    driver = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(driver)
    captures = folder/'journal-entry'
    captures.mkdir()
    source = captures/'source'
    require(driver.git(root, 'rev-parse', 'HEAD') == head, 'public journal source commit differs')
    driver.unpack(head, source)
    fixed = json.loads((source/'verification/foundations-environment.json').read_bytes())
    for name, binary in [('aeneas','aeneas/bin/aeneas'), ('charon','aeneas/charon/bin/charon')]:
        require(codec.sha(tools/binary) == fixed[name]['binary_sha256'], 'public journal tool differs: '+name)
    previous_target = os.environ.get('CARGO_TARGET_DIR')
    try:
        journal = driver.capture_public(source, captures/'journal', tools, fixed, include_lease=True)
        original = driver.load(source/codec.AREA/'serde-binding/binding.py').Original
        public = original(json.loads((captures/'journal/actual.llbc').read_bytes()))
        coordinator = driver.capture_parent(source, captures/'coordinator', tools, fixed, refactored=True)
        require(coordinator['journal_entry'] == journal['journal_entry'], 'coordinator and public journal entry differ')
        journal_decision.capture(source, captures, tools, fixed, driver.run)
        decision = journal_decision.readback(source, captures, original, fixed['target'])
        dec = captures/'decoder'
        dec.mkdir()
        decoder = codec_reads.capture(source, dec, tools, fixed, public, original, driver.run)
    finally:
        if previous_target is None:
            os.environ.pop('CARGO_TARGET_DIR', None)
        else:
            os.environ['CARGO_TARGET_DIR'] = previous_target
    inputs = project_inputs(root, captures/'journal/generated', dec/'generated', captures)
    expected = dict(original_files)
    for name, data in inputs.items():
        path = project/name
        if name in original_files:
            require(codec.sha(path) == original_files[name], 'initial BEGIN project differs: '+name)
        else:
            require(not path.exists(), 'new public journal input shadows an existing file: '+name)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        expected[name] = codec.digest(data)
    with (project/'lakefile.toml').open('a') as config:
        config.write(f'\n[[lean_lib]]\nname = "{codec.LIB}"\nsrcDir = "{codec.FOLDER}"\n')
        config.write(f'\n[[lean_lib]]\nname = "{journal_decision.LIB}"\nsrcDir = "{journal_decision.FOLDER}"\n')
    files = closure(project, [PAGE], active_view='lakefile.toml')
    manifest = json.loads((root/PROVIDER/'materialization.json').read_bytes())['modules']
    known = {entry['path']:entry['sha256'] for entry in manifest.values()} | expected
    require(expected.keys() <= files.keys() and all(known.get(n) == h for n, h in files.items()),
            'public journal import closure differs from its transaction, decision and BEGIN inputs')
    return files, dict(journal=journal, decoder=decoder, **decision)


def check_page(root, actual):
    files = actual.get('files', {})
    require(journal_decision.PHYSICAL.items() <= files.items() and
            all(files.get(n) == codec.sha(root/p) for n, p in APPEND_WITNESSES.items()) and
            AUDITS <= set(actual.get('audits', [])) and all(
        files.get(str(RECORD/(n+'.lean'))) == codec.sha(root/PKG/(n+'.lean.in')) for n in NEW),
        'missing public BEGIN proof, representation or audit')


def check_evidence(root, folder, row, actual, evidence):
    check_page(root, actual)
    captures = folder/'journal-entry'
    for name, stem in [('journal', 'SandproofActualRecord'), ('decoder', codec.LIB)]:
        raw = captures/name/'actual.llbc'
        generated = captures/name/'generated'
        view = row[name]
        required = {'translation.json', *(stem+'/'+n+'.lean' for n in
                    ('Types','Funs','TypesExternal_Template','FunsExternal_Template'))}
        require(view.get('ir') == evidence.get(str(raw)) == codec.sha(raw)
                and view.get('generated', {}).keys() == required and all(
                evidence.get(str(generated/n)) == view['generated'][n] == codec.sha(generated/n) for n in required),
                'missing or changed public BEGIN extraction: '+name)
    project = folder/'project'
    files = actual['files']
    for path, data in project_inputs(root, captures/'journal/generated', captures/'decoder/generated', captures).items():
        require(files.get(path) == evidence.get(str(project/path)) == codec.digest(data),
                'public BEGIN physical page differs: '+path)
    spec = importlib.util.spec_from_file_location('original_receipt_binding', root/codec.AREA/'serde-binding/binding.py')
    binding = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(binding)
    fixed = json.loads((root/'verification/foundations-environment.json').read_bytes())
    decision = journal_decision.readback(root, captures, binding.Original, fixed['target'])
    for mode, view in decision.items():
        raw = captures/mode/'actual.llbc'
        require(row.get(mode) == view and evidence.get(str(raw)) == view['ir'] and
                all(evidence.get(str(captures/mode/'generated'/n)) == h for n, h in view.get('generated', {}).items()),
                'detached journal decision or parent evidence: '+mode)
    source = (root/codec_binding.SOURCE).read_text()
    appended = source+(root/PROVIDER/'caller-root.rs.in').read_text()+(root/codec.PACKAGE/'public-journal-root.rs.in').read_text()
    public = binding.Original(json.loads((captures/'journal/actual.llbc').read_bytes()))
    selected = codec_binding.admit(binding.Original(json.loads((captures/'decoder/actual.llbc').read_bytes())),
        public, json.loads((captures/'decoder/generated/translation.json').read_bytes()), source, appended)
    entry = journal_entry.check_entry(public) | dict(root_sha256=codec.sha(root/PROVIDER/'caller-root.rs.in'),
        recording_sha256=codec.sha(root/codec_binding.SOURCE),
        journal_sha256=codec.sha(root/'controller-service/src/controller/lifecycle_inbox.rs'))
    require(decision['coordinator']['journal'] == journal_entry.check_entry(public),
            'parent and public journal select different recording calls')
    mapping = json.loads((captures/'journal/generated/translation.json').read_bytes())
    require(row['journal']['journal_entry'] == entry and row['journal']['translated'] ==
            journal_entry.translated_entry(public, mapping, journal_entry.RECORDING+'::proof_public_journal'),
            'public BEGIN call no longer matches its original journal entry')
    require(row['decoder'].get('selected') == selected and files == closure(project, [PAGE], active_view='lakefile.toml'),
            'detached public journal decoder or incomplete BEGIN closure')
