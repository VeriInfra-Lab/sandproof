"""Reuse the eight reviewed per-symbol bodies; no new stdlib or Serde assumption."""
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import re
import subprocess

HERE = Path('verification/experiments/stage1-protocol/inbox-entry-validation')
EXTERNAL = {
    'Bool.Insts.CoreCmpPartialEqBool.ne', 'core.option.Option.as_ref',
    'core.option.Option.Insts.CoreCmpPartialEqOption.eq',
    'core.option.Option.is_some_and', 'core.option.Option.is_none_or',
    'alloc.string.String.Insts.CoreCmpPartialEqString.eq', 'alloc.vec.Vec.is_empty',
    'SharedAVec.Insts.CoreIterTraitsCollectIntoIteratorSharedATIter.into_iter',
}


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def check(root):
    parent = module('entry_existing_result', root/HERE.parent/'inbox-renewal-session/library_binding.py')
    result = parent.check(root)
    unload = module('entry_existing_unload', root/HERE.parent/'inbox-unload-resolution/library_binding.py')
    unload_result = unload.check(root)
    current = (root/HERE/'FunsExternal.lean.in').read_text()
    previous = (root/HERE.parent/'inbox-lineage/FunsExternal.lean.in').read_text()
    if set(re.findall(r'^def (\S+)', current, re.M)) != EXTERNAL:
        raise ValueError('unreviewed entry validation function binding')
    for name in EXTERNAL:
        if parent.body(current, name) != parent.body(previous, name):
            raise ValueError('inherited per-symbol body changed: '+name)
    fixed = json.loads((root/'verification/foundations-environment.json').read_text())
    frozen = json.loads((root/HERE/'library-sources.json').read_text())
    sources = {}
    for kind, selector in [('production', []), ('extraction', ['+'+fixed['extraction_frontend']])]:
        sysroot = Path(subprocess.check_output(['rustc', *selector, '--print', 'sysroot'], text=True).strip())
        sources[kind] = {}
        if set(frozen[kind]) != {'core/src/iter/traits/iterator.rs', 'core/src/cmp.rs'}:
            raise ValueError('incomplete original count/max source observation')
        for name, expected in frozen[kind].items():
            path = sysroot/'lib/rustlib/src/rust/library'/name
            if sha(path) != expected:
                raise ValueError('original count/max library source changed: '+kind+'/'+name)
            sources[kind][name] = dict(path=str(path), sha256=sha(path))
    return dict(result=result, unload=unload_result, symbols=sorted(EXTERNAL), sources=sources,
        stdlib_proved=False, borrowed_vec_adopted=False, result_tags_adopted=False,
        original_equivalence_proved=False, standard_serde_instance_adopted=False)


def result_tags(raw):
    root = Path(__file__).resolve().parents[4]
    parent = module('entry_result_tags', root/HERE.parent/'inbox-renewal-session/library_binding.py')
    return parent.result_tags(raw)


def phase_tags(raw):
    root = Path(__file__).resolve().parents[4]
    parent = module('entry_phase_tags', root/HERE.parent/'inbox-unload-resolution/library_binding.py')
    return parent.phase_tags(raw)
