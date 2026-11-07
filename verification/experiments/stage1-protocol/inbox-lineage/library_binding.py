"""Check the narrow borrowed-Vec candidate against fixed source/model inputs.

This is source correspondence evidence, not a machine proof of Rust stdlib or
approval to add a new V0 dependency. The caller theorem names that obligation.
"""
from hashlib import sha256
import json
from pathlib import Path
import re
import subprocess

HERE = Path('verification/experiments/stage1-protocol/inbox-lineage')
SYMBOL = 'SharedAVec.Insts.CoreIterTraitsCollectIntoIteratorSharedATIter.into_iter'


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def check(root):
    fixed = json.loads((root/'verification/foundations-environment.json').read_text())
    sources = {}
    for kind, selector in [('production', []), ('extraction', ['+'+fixed['extraction_frontend']])]:
        sysroot = Path(subprocess.check_output(['rustc', *selector, '--print', 'sysroot'], text=True).strip())
        source = sysroot/'lib/rustlib/src/rust/library/alloc/src/vec/mod.rs'
        if sha(source) != fixed['library_sources'][kind]['alloc/src/vec/mod.rs']:
            raise ValueError('fixed borrowed-Vec source differs: '+kind)
        text = source.read_text()
        begin = text.index("impl<'a, T, A: Allocator> IntoIterator for &'a Vec<T, A> {")
        end = text.index('\n}', begin)+2
        body = text[begin:end]
        expected = """impl<'a, T, A: Allocator> IntoIterator for &'a Vec<T, A> {
    type Item = &'a T;
    type IntoIter = slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}"""
        if body != expected:
            raise ValueError('borrowed Vec wrapper is no longer the reviewed self.iter call')
        sources[kind] = dict(path=str(source), sha256=sha(source), body=body)
    current = (root/HERE/'FunsExternal.lean.in').read_text()
    parent = (root/HERE.parent/'inbox-record/FunsExternal.lean.in').read_text()
    names = re.findall(r'^def (\S+)', current, re.M)
    previous = re.findall(r'^def (\S+)', parent, re.M)
    if names != previous+[SYMBOL]:
        raise ValueError('unreviewed external binding inventory')
    for name in previous:
        pattern = r'(?ms)^def '+re.escape(name)+r'\b.*?(?=^def |^/-|^end )'
        if re.search(pattern, current).group().strip() != re.search(pattern, parent).group().strip():
            raise ValueError('existing per-symbol contract changed: '+name)
    candidate = re.search(r'(?ms)^def '+re.escape(SYMBOL)+r'\b.*?(?=^end )', current).group().strip()
    expected = '''def SharedAVec.Insts.CoreIterTraitsCollectIntoIteratorSharedATIter.into_iter
    {T : Type} (_allocator : Type) (value : alloc.vec.Vec T) :
    Result (core.slice.iter.Iter T) :=
  core.slice.Slice.iter (alloc.vec.Vec.deref value)'''
    if candidate != expected:
        raise ValueError('candidate is not the fixed Vec/Slice model delegation')
    backend = root/'.tools/aeneas/backends/lean/Aeneas/Std'
    return dict(v0_adopted=False, stdlib_proved=False, symbol=SYMBOL, sources=sources,
                candidate_sha256=sha(root/HERE/'FunsExternal.lean.in'),
                backend={name: sha(backend/name) for name in ['Vec.lean', 'SliceIter.lean']})
