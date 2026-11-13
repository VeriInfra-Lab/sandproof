"""Reuse seven reviewed bodies and the #696 logical Result representation.

Checking these closures does not adopt pending V0 bindings or prove stdlib.
"""
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import re
import subprocess

HERE = Path('verification/experiments/stage1-protocol/inbox-renewal-session')
SYMBOL = 'SharedAVec.Insts.CoreIterTraitsCollectIntoIteratorSharedATIter.into_iter'
EXTERNAL = {
    'Bool.Insts.CoreCmpPartialEqBool.ne',
    'core.option.Option.Insts.CoreCmpPartialEqOption.eq',
    'alloc.string.String.Insts.CoreCmpPartialEqString.eq', 'alloc.vec.Vec.is_empty',
    'core.option.Option.is_some_and', 'core.option.Option.as_ref', SYMBOL,
}


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def body(text, name):
    found = re.search(r'(?ms)^def '+re.escape(name)+r'\b.*?(?=^def |^/-|^end |\Z)', text)
    if found is None:
        raise ValueError('missing per-symbol binding: '+name)
    return found.group().strip()


def check(root):
    path = root/HERE.parent/'inbox-lineage/library_binding.py'
    spec = importlib.util.spec_from_file_location('renewal_session_borrowed_vec', path)
    parent = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(parent)
    borrowed = parent.check(root)
    current = (root/HERE/'FunsExternal.lean.in').read_text()
    previous = (root/HERE.parent/'inbox-lineage/FunsExternal.lean.in').read_text()
    if set(re.findall(r'^def (\S+)', current, re.M)) != EXTERNAL:
        raise ValueError('unreviewed renewal-session function binding')
    for name in EXTERNAL:
        if body(current, name) != body(previous, name):
            raise ValueError('inherited per-symbol body changed: '+name)
    fixed = json.loads((root/'verification/foundations-environment.json').read_text())
    sources = {}
    for kind, selector in [('production', []), ('extraction', ['+'+fixed['extraction_frontend']])]:
        sysroot = Path(subprocess.check_output(['rustc', *selector, '--print', 'sysroot'], text=True).strip())
        path = sysroot/'lib/rustlib/src/rust/library/core/src/result.rs'
        if sha(path) != fixed['library_sources'][kind]['core/src/result.rs']:
            raise ValueError('fixed Result source changed: '+kind)
        sources[kind] = dict(path=str(path), sha256=sha(path))
    backend = root/'.tools/aeneas/backends/lean/Aeneas'
    return dict(borrowed_vec=borrowed, symbols=sorted(EXTERNAL), sources=sources,
        backend={name: sha(backend/name) for name in ['Std/Core/Result.lean', 'Data/Discriminant.lean']},
        result_tags_adopted=False, physical_layout_claim=False,
        library_implementation_proved=False, standard_serde_instance_adopted=False)


def result_tags(raw):
    declarations = [t for t in raw['translated']['type_decls'] if t and
        t['item_meta']['name'] == [{'Ident': [x, 0]} for x in ['core', 'result', 'Result']]]
    if len(declarations) != 1:
        raise ValueError('missing or ambiguous original Result representation')
    variants = [dict(name=v['name'], discriminant=v['discriminant'], fields=len(v['fields']))
        for v in declarations[0]['kind']['Enum']]
    expected = [dict(name='Ok', discriminant={'Signed': ['Isize', '0']}, fields=1),
        dict(name='Err', discriminant={'Signed': ['Isize', '1']}, fields=1)]
    if variants != expected:
        raise ValueError('original Result logical tags differ from #696 representation')
    return variants
