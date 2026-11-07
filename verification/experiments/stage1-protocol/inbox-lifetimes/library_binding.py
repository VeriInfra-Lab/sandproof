"""Reuse the existing per-symbol bodies; verify their fixed source closures."""
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import re
import subprocess

HERE = Path('verification/experiments/stage1-protocol/inbox-lifetimes')
SYMBOL = 'SharedAVec.Insts.CoreIterTraitsCollectIntoIteratorSharedATIter.into_iter'


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def body(text, name):
    pattern = r'(?ms)^def '+re.escape(name)+r'\b.*?(?=^def |^/-|^end |\Z)'
    match = re.search(pattern, text)
    if match is None:
        raise ValueError('missing registered binding: '+name)
    return match.group().strip()


def check(root):
    path = root/HERE.parent/'inbox-lineage/library_binding.py'
    spec = importlib.util.spec_from_file_location('lifetime_borrowed_vec', path)
    parent = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(parent)
    borrowed = parent.check(root)
    current = (root/HERE/'FunsExternal.lean.in').read_text()
    previous = (root/HERE.parent/'inbox-lineage/FunsExternal.lean.in').read_text()
    inherited = re.findall(r'^def (\S+)', previous, re.M)
    extra = {
        'I64.Insts.CoreConvertTryFromU64TryFromIntError.try_from':
            'verification/aeneas/sqlite-statements/SandproofSqliteStatements/FunsExternal.lean',
        'core.slice.Slice.first':
            'verification/aeneas/tls/SandproofTls/FunsExternal.lean',
    }
    if re.findall(r'^def (\S+)', current, re.M) != inherited+list(extra):
        raise ValueError('unreviewed lifetime external binding inventory')
    for name in inherited:
        if body(current, name) != body(previous, name):
            raise ValueError('inherited per-symbol body changed: '+name)
    models = {}
    for name, source in extra.items():
        if body(current, name) != body((root/source).read_text(), name):
            raise ValueError('existing per-symbol body changed: '+name)
        models[name] = dict(path=source, sha256=sha(root/source), body=body(current, name))
    fixed = json.loads((root/'verification/foundations-environment.json').read_text())
    registered = json.loads((root/'verification/sqlite-statements-environment.json').read_text())
    sources = {}
    for kind, selector in [('production', []), ('extraction', ['+'+fixed['extraction_frontend']])]:
        sysroot = Path(subprocess.check_output(['rustc', *selector, '--print', 'sysroot'], text=True).strip())
        sources[kind] = {}
        for name in ['core/src/convert/num.rs', 'core/src/num/error.rs',
                     'core/src/num/int_macros.rs', 'core/src/slice/mod.rs', 'core/src/option.rs']:
            path = sysroot/'lib/rustlib/src/rust/library'/name
            if sha(path) != registered['library_sources'][kind][name]:
                raise ValueError('fixed lifetime library input changed: '+kind+'/'+name)
            sources[kind][name] = dict(path=str(path), sha256=sha(path))
    return dict(borrowed_vec=borrowed, existing_models=models, sources=sources,
                option_and_then='transparent original Rust, required by generated admission',
                production_refactor_adopted=False, original_equivalence_proved=False,
                library_implementation_proved=False)
