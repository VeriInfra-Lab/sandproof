"""Reuse the exact registered value models required by the session-shape view."""
from hashlib import sha256
import importlib.util
from pathlib import Path

HERE = Path('verification/experiments/stage1-protocol/session-shape')
GENERATED = Path('verification/aeneas/stage1-protocol/request/SandproofStage1Request')
EXTERNAL = {
    'Bool.Insts.CoreCmpPartialEqBool.ne',
    'core.option.Option.Insts.CoreCmpPartialEqOption.eq',
    'core.option.Option.is_some_and', 'core.option.Option.is_none_or',
    'alloc.string.String.Insts.CoreCmpPartialEqString.eq', 'alloc.vec.Vec.is_empty',
}
EXTRA = ('core.option.Option.is_some_and', 'core.option.Option.is_none_or')
SOURCE = 'verification/aeneas/foundations/SandproofFoundations/FunsExternal.lean'
WRAPPERS = 'verification/experiments/stage1-protocol/inbox-record/FunsExternal.lean.in'


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def parent(root):
    spec = importlib.util.spec_from_file_location('shape_existing_value_bindings', root/HERE.parent/'native-retirement/library_binding.py')
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def fragment(text, name, attributes=False):
    return parent(Path(__file__).resolve().parents[4]).fragment(text, name, attributes)


def contents(root):
    saved = parent(root)
    files, pieces = saved.contents(root)
    shared = (root/SOURCE).read_text()
    fragments = {name:saved.fragment(shared, name, attributes=True) for name in EXTRA}
    pieces.update(fragments)
    files['SharedValueBindings.lean'] = files['SharedValueBindings.lean'].replace(
        'end SandproofFoundationsRust', '\n\n'.join(fragments.values())+'\n\nend SandproofFoundationsRust')
    wrappers = (root/WRAPPERS).read_text()
    fragments = {name:saved.fragment(wrappers, name) for name in EXTRA}
    files['FunsExternal.lean'] = files['FunsExternal.lean'].replace(
        'end SandproofStage1RequestRust', '\n\n'.join(fragments.values())+'\n\nend SandproofStage1RequestRust')
    return files, pieces


def check(root):
    previous = parent(root).check(root)
    files, pieces = contents(root)
    wrappers = (root/WRAPPERS).read_text()
    return dict(previous=previous, symbols=sorted(EXTERNAL), source=SOURCE, source_sha256=sha(root/SOURCE),
        shared_fragments={n:dict(body=s, sha256=sha256(s.encode()).hexdigest()) for n,s in pieces.items()},
        wrappers_source=WRAPPERS, wrappers_sha256=sha(root/WRAPPERS),
        extra_wrappers={n:fragment(wrappers,n) for n in EXTRA},
        outputs={n:sha256(s.encode()).hexdigest() for n,s in files.items()},
        new_external_models=False, original_body_and_attributes_unchanged=True,
        internal_error_alias=False, library_implementation_proved=False,
        standard_encoding_and_digest_adopted=False, result_tags_adopted=False)


def install(root, project):
    files, _ = contents(root)
    for name, text in files.items():
        (project/GENERATED/name).write_text(text)
    return {str(GENERATED/name):sha(project/GENERATED/name) for name in files}


def result_tags(raw):
    return parent(Path(__file__).resolve().parents[4]).result_tags(raw)
