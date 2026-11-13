"""Generate an import view of existing A6 bodies without unrelated enum instances."""
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import re
import subprocess

HERE = Path('verification/experiments/stage1-protocol/retention-lane')
GENERATED = Path('verification/aeneas/stage1-protocol/request/SandproofStage1Request')
EXTERNAL = {
    'Bool.Insts.CoreCmpPartialEqBool.ne',
    'core.option.Option.Insts.CoreCmpPartialEqOption.eq', 'core.result.Result.ok',
    'alloc.string.String.Insts.CoreCmpPartialEqString.eq', 'alloc.vec.Vec.is_empty',
}
SHARED = ('core.result.Result.ok', 'alloc.string.String.Insts.CoreCmpPartialEqString.eq',
          'alloc.vec.Vec.is_empty')
SOURCE = 'verification/aeneas/foundations/SandproofFoundations/FunsExternal.lean'


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def parent(root):
    spec = importlib.util.spec_from_file_location('lane_existing_bindings', root/HERE.parent/'inbox-config/library_binding.py')
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def fragment(text, name, attributes=False):
    matches = list(re.finditer(r'^def '+re.escape(name)+r'(?=\s|\{)', text, re.M))
    if len(matches) != 1:
        raise ValueError('nonunique existing model: '+name)
    start = matches[0].start()
    end = text.index('\n\n', start)
    if attributes:
        start = text.rfind('@[', 0, start)
        if start < 0:
            raise ValueError('missing existing model attributes')
    result = text[start:end]
    if result.count('\ndef ') != (1 if attributes else 0) or '\nimport ' in result:
        raise ValueError('model view crossed a definition boundary')
    return result


def contents(root):
    shared = (root/SOURCE).read_text()
    pieces = {name:fragment(shared, name, attributes=True) for name in SHARED}
    view = ('-- Generated view of existing A6 definitions; verbatim bodies and attributes.\n'
            'import Aeneas\n\nopen Aeneas Aeneas.Std\nnamespace SandproofFoundationsRust\n\n'
            +'\n\n'.join(pieces.values())+'\n\nend SandproofFoundationsRust\n')
    bindings = (root/HERE.parent/'inbox-config/FunsExternal.lean.in').read_text()
    if not bindings.startswith('import SandproofFoundations.FunsExternal\n'):
        raise ValueError('existing import changed')
    bindings = bindings.replace('import SandproofFoundations.FunsExternal\n',
        'import SandproofStage1Request.SharedValueBindings\n', 1)
    result_ok = fragment((root/HERE.parent/'inbox-record/FunsExternal.lean.in').read_text(), 'core.result.Result.ok')
    bindings = bindings.replace('end SandproofStage1RequestRust', result_ok+'\n\nend SandproofStage1RequestRust')
    if set(re.findall(r'^def (\S+)', bindings, re.M)) != EXTERNAL:
        raise ValueError('unreviewed external model set')
    types = ('import SandproofBoundedWriter.TypesExternal\n\nnamespace SandproofStage1RequestRust\n'
             'abbrev serde_json.error.Error := SandproofBoundedWriterRust.serde_json.error.Error\n'
             'end SandproofStage1RequestRust\n')
    return {'SharedValueBindings.lean':view, 'FunsExternal.lean':bindings, 'TypesExternal.lean':types}, pieces


def check(root):
    previous = parent(root).check(root)
    files, pieces = contents(root)
    fixed = json.loads((root/'verification/foundations-environment.json').read_text())
    sources = {}
    for kind, selector in [('production', []), ('extraction', ['+'+fixed['extraction_frontend']])]:
        sysroot = Path(subprocess.check_output(['rustc', *selector, '--print', 'sysroot'], text=True).strip())
        sources[kind] = {}
        for name in ('core/src/option.rs', 'core/src/result.rs'):
            path = sysroot/'lib/rustlib/src/rust/library'/name
            if sha(path) != fixed['library_sources'][kind][name]:
                raise ValueError('fixed Option/Result source changed')
            sources[kind][name] = dict(path=str(path), sha256=sha(path))
    return dict(previous=previous, symbols=sorted(EXTERNAL), sources=sources,
        source=SOURCE, source_sha256=sha(root/SOURCE),
        shared_fragments={n:dict(body=s, sha256=sha256(s.encode()).hexdigest()) for n,s in pieces.items()},
        outputs={n:sha256(s.encode()).hexdigest() for n,s in files.items()},
        new_external_models=False, original_body_and_attributes_unchanged=True,
        internal_error_alias=False, library_implementation_proved=False,
        standard_serde_instance_adopted=False, result_tags_adopted=False)


def install(root, project):
    files, _ = contents(root)
    for name, text in files.items():
        (project/GENERATED/name).write_text(text)
    return {str(GENERATED/name):sha(project/GENERATED/name) for name in files}


def result_tags(raw):
    return parent(Path(__file__).resolve().parents[4]).result_tags(raw)
