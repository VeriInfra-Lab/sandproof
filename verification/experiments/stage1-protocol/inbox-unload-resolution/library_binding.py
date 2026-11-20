"""Five existing per-symbol bindings; no new Serde or business model."""
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import re
import subprocess

HERE = Path('verification/experiments/stage1-protocol/inbox-unload-resolution')
EXTERNAL = {
    'Bool.Insts.CoreCmpPartialEqBool.ne', 'core.option.Option.as_ref',
    'core.option.Option.Insts.CoreCmpPartialEqOption.eq',
    'alloc.string.String.Insts.CoreCmpPartialEqString.eq', 'alloc.vec.Vec.is_empty',
}


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def check(root):
    parent_path = root / HERE.parent / 'inbox-renewal-session/library_binding.py'
    spec = importlib.util.spec_from_file_location('unload_existing_bindings', parent_path)
    parent = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(parent)
    current = (root / HERE / 'FunsExternal.lean.in').read_text()
    previous = (root / HERE.parent / 'inbox-renewal-session/FunsExternal.lean.in').read_text()
    if set(re.findall(r'^def (\S+)', current, re.M)) != EXTERNAL:
        raise ValueError('unreviewed unload-resolution function binding')
    for name in EXTERNAL:
        if parent.body(current, name) != parent.body(previous, name):
            raise ValueError('inherited per-symbol body changed: ' + name)
    fixed = json.loads((root / 'verification/foundations-environment.json').read_text())
    sources = {}
    for kind, selector in [('production', []), ('extraction', ['+' + fixed['extraction_frontend']])]:
        sysroot = Path(subprocess.check_output(['rustc', *selector, '--print', 'sysroot'], text=True).strip())
        sources[kind] = {}
        for name in ('core/src/option.rs', 'core/src/bool.rs', 'alloc/src/string.rs', 'alloc/src/vec/mod.rs'):
            path = sysroot / 'lib/rustlib/src/rust/library' / name
            if sha(path) != fixed['library_sources'][kind][name]:
                raise ValueError('fixed library source changed: ' + kind + '/' + name)
            sources[kind][name] = dict(path=str(path), sha256=sha(path))
    backend = root / '.tools/aeneas/backends/lean/Aeneas'
    return dict(symbols=sorted(EXTERNAL), sources=sources,
        original_bindings_sha256=sha(root / HERE.parent / 'inbox-renewal-session/FunsExternal.lean.in'),
        discriminant_macro_sha256=sha(backend / 'Data/Discriminant.lean'),
        library_implementation_proved=False, standard_serde_instance_adopted=False,
        physical_layout_claim=False)


def phase_tags(raw):
    name = [{'Ident': [x, 0]} for x in ['sandbox_controller_core', 'model', 'Phase']]
    declarations = [t for t in raw['translated']['type_decls'] if t and t['item_meta']['name'] == name]
    if len(declarations) != 1:
        raise ValueError('missing original Phase logical tags')
    variants = [dict(name=v['name'], discriminant=v['discriminant'], fields=len(v['fields']))
                for v in declarations[0]['kind']['Enum']]
    names = ['Starting', 'Running', 'Pausing', 'Paused', 'Revoking', 'Destroyed']
    expected = [dict(name=n, discriminant={'Signed': ['Isize', str(i)]}, fields=0)
                for i, n in enumerate(names)]
    if variants != expected:
        raise ValueError('original Phase logical tags differ from generated six-way representation')
    return variants
