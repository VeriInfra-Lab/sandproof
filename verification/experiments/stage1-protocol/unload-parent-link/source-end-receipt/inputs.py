"""Keep the original saved-End receipt and its exact library calls."""
import importlib.util
from pathlib import Path
import sys
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / 'source-remove-retired'))
spec = importlib.util.spec_from_file_location('source_end_receipt_parent', HERE.parent / 'source-end-validation/capture.py')
parent = importlib.util.module_from_spec(spec)
spec.loader.exec_module(parent)
retire = parent.retire
END = parent.END
VARIANTS = ('original', 'no-request-as-returned')
ROOT = 'sandproof_controller_service::controller::lifecycle_inbox::source_work::full::end::receipt'
NAMESPACE = 'SandproofSourceEndReceipt'
SHA = 'sandproof_controller_service::controller::lifecycle_inbox::sha256::digest'
PORT = 'sandproof_controller_service::controller::lifecycle_inbox::source_work::full::end::encode_receipt_end'
STAGED = parent.STAGED
OPTIONS = dict(parent.OPTIONS, roots=[ROOT], retained_roots=[parent.ROOT],
    include=[*parent.OPTIONS['include'], 'core::option::_::map', 'core::option::_::transpose'],
    opaque=list(dict.fromkeys((*parent.OPTIONS['opaque'], SHA, PORT))))
ORIGINAL_TAG = '                    core::capture::CaptureWorkEnd::NoRequest { seal }'
WRONG_TAG = '                    core::capture::CaptureWorkEnd::Returned { seal }'

def once(text, old, new):
    if text.count(old) != 1:
        raise ValueError('one exact original occurrence required: ' + old)
    return text.replace(old, new, 1)

def mutate(text, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown saved End receipt view')
    begin = text.index('fn receipt(')
    end = text.index('\npub(super) fn continuation_allowed', begin)
    body = text[begin:end]
    if body.count(ORIGINAL_TAG) != 1:
        raise ValueError('one original NoRequest receipt tag required')
    if variant == 'no-request-as-returned':
        body = body.replace(ORIGINAL_TAG, WRONG_TAG, 1)
    return text[:begin] + body + text[end:]

def reconstruct(archive, source, variant):
    try:
        parent.reconstruct(archive, source, 'original')
    finally:
        retire.STAGED = STAGED
    path = source / END
    text = mutate(path.read_text(), variant)
    begin = text.index('fn receipt(')
    end = text.index('\npub(super) fn continuation_allowed', begin)
    body = once(text[begin:end], 'serde_json::to_vec(end)?', 'encode_receipt_end(end)?')
    helper = '\nfn encode_receipt_end(value: &CaptureEndRecord) -> serde_json::Result<Vec<u8>> {\n    serde_json::to_vec(value)\n}\n'
    path.write_text(text[:begin] + body + text[end:] + helper)
    # Existing production source_work::digest already delegates the complete
    # slice to the registered lifecycle_inbox::sha256::digest primitive.
    # Keep that owned wrapper and the canonical ContentDigest methods unchanged.
