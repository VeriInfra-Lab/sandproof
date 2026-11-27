"""Read back the decoder pages, generated inputs and complete local imports."""
import hashlib
from pathlib import Path

LIB = 'SandproofStandardRecordCodec'
GENERATED_PREFIX = (LIB + '.controller.lifecycle_inbox.recording.StandardRecordCodec.Insts.'
                    'Sandproof_controller_serviceControllerLifecycle_inboxRecordingRecordCodec.')
AREA = Path('verification/experiments/stage1-protocol')
PACKAGE = AREA/'coordinator-revocation'
TEMPLATES = PACKAGE/'codec-read'
FOLDER = Path('verification/aeneas/record-codec-read')
PAGE = str(FOLDER/LIB/'DecoderCalls.lean')
ROOT = LIB+'.DecoderCalls'
AUDITS = {ROOT+'.maintenance_return', ROOT+'.inbox_return'}
HANDWRITTEN = ('TypesExternal', 'FunsExternal', 'DecoderCalls')
GENERATED = {'translation.json', *(LIB+'/'+name+'.lean' for name in
             ('Types', 'Funs', 'TypesExternal_Template', 'FunsExternal_Template'))}
BIND = ('namespace '+LIB+'\n').encode()
WITH_CALLS = BIND+b'\nvariable [DecodeCalls]\n'
RAW_PAGES = {LIB+'/Types.lean': '02797c3a4527f64f592478d61096ed3a8ba528248d088916dc7ec2653dc97195',
             LIB+'/Funs.lean': 'de6c01a84eb0601ab51e9d0dc96ef9e8902859ee32f27419e878b8efb9b9eef0'}


def require(ok, message):
    if not ok:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def sha(path):
    return digest(path.read_bytes())


def bound_functions(raw):
    require(raw.count(BIND) == 1 and WITH_CALLS not in raw, 'unexpected decoder namespace')
    return raw.replace(BIND, WITH_CALLS, 1)


def check_page(root, row):
    actual = row.get('actual', {})
    files = actual.get('files', {})
    require(RAW_PAGES.items() <= row.get('generated', {}).items()
            and row.get('selected') == {name: GENERATED_PREFIX+name for name in ('decode_maintenance', 'decode_inbox')}
            and AUDITS <= set(actual.get('audits', [])) and all(
                files.get(str(FOLDER/LIB/(n+'.lean'))) == sha(root/TEMPLATES/(n+'.lean.in')) for n in HANDWRITTEN),
            'missing concrete decoder page, methods or audits')


def check_evidence(root, folder, row, evidence):
    from tools.proof_batch import closure
    check_page(root, row)
    generated = folder/'generated'
    hashes = row.get('generated', {})
    require(set(hashes) == GENERATED and all(evidence.get(str(generated/n)) == hashes[n] == sha(generated/n)
            for n in GENERATED) and row.get('ir') == sha(folder/'actual.llbc') == evidence.get(str(folder/'actual.llbc')),
            'missing or changed concrete decoder extraction')
    read = row['actual']['files']
    require(read == closure(folder/'project', [PAGE], active_view='lakefile.toml')
            and read.get(str(FOLDER/LIB/'Types.lean')) == hashes[LIB+'/Types.lean']
            and read.get(str(FOLDER/LIB/'Funs.lean')) == digest(bound_functions((generated/LIB/'Funs.lean').read_bytes())),
            'checked concrete reads differ from generated code')
