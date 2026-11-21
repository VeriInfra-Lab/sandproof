"""Exact source-only reconstruction of the original saved-question call."""
import importlib.util
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
PARENT = HERE.parent / 'source-full-binding'
sys.path.insert(0, str(HERE.parent / 'source-remove-retired'))
_spec = importlib.util.spec_from_file_location('source_saved_question_original_parent', PARENT / 'capture.py')
parent = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(parent)
retire = parent.retire
END = Path('controller-service/src/controller/lifecycle_inbox/source_work/full/drain/question.rs')
VARIANTS = ('original', 'allow-old-stop-epoch')
ROOT = 'sandproof_controller_service::controller::lifecycle_inbox::source_work::full::drain::question::from_saved'
NAMESPACE = 'SandproofSourceSavedQuestion'
OPTIONS = dict(parent.OPTIONS, roots=[ROOT])
STAGED = (*parent.STAGED, END)
OMITTED = '        || stop.epoch <= id.epoch\n'


def mutate(text, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown original saved-question view')
    begin = text.index('fn assemble(')
    end = text.index('pub(super) fn from_saved(', begin)
    body = text[begin:end]
    if body.count(OMITTED) != 1:
        raise ValueError('one exact strict stop epoch check is mandatory')
    return text[:begin] + (body if variant == 'original' else body.replace(OMITTED, '', 1)) + text[end:]


def reconstruct(archive, source, variant):
    try:
        parent.reconstruct(archive, source, 'original')
    finally:
        retire.STAGED = STAGED
    path = source / END
    path.write_text(mutate(path.read_text(), variant))
