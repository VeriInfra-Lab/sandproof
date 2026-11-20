"""Reuse five exact per-symbol models and inspect the fixed original iterator sources."""
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import re
import subprocess

HERE = Path('verification/experiments/stage1-protocol/inbox-retired-identities')
EXTERNAL = {
    'Bool.Insts.CoreCmpPartialEqBool.ne',
    'core.option.Option.Insts.CoreCmpPartialEqOption.eq',
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
    parent = module('retired_existing_result', root/HERE.parent/'inbox-renewal-session/library_binding.py')
    result = parent.check(root)
    current = (root/HERE/'FunsExternal.lean.in').read_text()
    previous = (root/HERE.parent/'inbox-lineage/FunsExternal.lean.in').read_text()
    if set(re.findall(r'^def (\S+)', current, re.M)) != EXTERNAL:
        raise ValueError('unreviewed retired-identity function binding')
    for name in EXTERNAL:
        if parent.body(current, name) != parent.body(previous, name):
            raise ValueError('inherited per-symbol body changed: '+name)
    fixed = json.loads((root/'verification/foundations-environment.json').read_text())
    frozen = json.loads((root/HERE/'library-sources.json').read_text())
    sources = {}
    for kind, selector in [('production', []), ('extraction', ['+'+fixed['extraction_frontend']])]:
        sysroot = Path(subprocess.check_output(['rustc', *selector, '--print', 'sysroot'], text=True).strip())
        sources[kind] = {}
        if set(frozen[kind]) != {'core/src/iter/traits/iterator.rs', 'core/src/slice/iter/macros.rs'}:
            raise ValueError('incomplete original any source observation')
        for name, expected in frozen[kind].items():
            path = sysroot/'lib/rustlib/src/rust/library'/name
            if sha(path) != expected:
                raise ValueError('original any library source changed: '+kind+'/'+name)
            sources[kind][name] = dict(path=str(path), sha256=sha(path))
    return dict(result=result, symbols=sorted(EXTERNAL), sources=sources,
        stdlib_proved=False, borrowed_vec_adopted=False, result_tags_adopted=False,
        original_equivalence_proved=False)


def result_tags(raw):
    root = Path(__file__).resolve().parents[4]
    parent = module('retired_result_tags', root/HERE.parent/'inbox-renewal-session/library_binding.py')
    return parent.result_tags(raw)
