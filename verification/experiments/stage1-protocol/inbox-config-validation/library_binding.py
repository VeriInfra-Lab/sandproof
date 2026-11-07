"""Exact reused observations, transparent stdlib, and two unadopted call bindings."""
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import re
import subprocess

HERE = Path('verification/experiments/stage1-protocol/inbox-config-validation')
EXTERNAL = {'Bool.Insts.CoreCmpPartialEqBool.ne', 'core.option.Option.as_ref',
    'core.option.Option.Insts.CoreCmpPartialEqOption.eq',
    'core.str.iter.Bytes.Insts.CoreIterTraitsIteratorIteratorU8.all', 'core.str.Str.bytes',
    'alloc.string.String.Insts.CoreCmpPartialEqString.eq', 'alloc.string.String.len',
    'alloc.string.String.is_empty', 'alloc.string.String.Insts.CoreOpsDerefDerefStr.deref',
    'alloc.vec.Vec.is_empty'}
PROPOSED = {'alloc.string.String.len', 'alloc.string.String.is_empty'}
SOURCES = {'alloc/src/string.rs', 'alloc/src/vec/mod.rs', 'core/src/num/mod.rs',
    'core/src/ops/range.rs', 'core/src/str/iter.rs'}


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def parent(root):
    p = root/HERE.parent/'inbox-config/library_binding.py'
    spec = importlib.util.spec_from_file_location('config_capacity_library', p)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def check(root):
    inherited = parent(root).check(root)
    current = (root/HERE/'FunsExternal.lean.in').read_text()
    if set(re.findall(r'^def (\S+)', current, re.M)) != EXTERNAL:
        raise ValueError('unreviewed config validation function model')
    frozen = json.loads((root/HERE/'library-sources.json').read_text())
    if set(frozen['models']) != {'FunsExternal.lean.in','TypesExternal.lean.in'}:
        raise ValueError('incomplete config call binding inventory')
    for name, expected in frozen['models'].items():
        if sha(root/HERE/name) != expected:
            raise ValueError('reviewed candidate call binding changed: '+name)
    fixed = json.loads((root/'verification/foundations-environment.json').read_text())
    sources = {}
    for kind, selector in [('production', []), ('extraction', ['+'+fixed['extraction_frontend']])]:
        sysroot = Path(subprocess.check_output(['rustc', *selector, '--print', 'sysroot'], text=True).strip())
        if set(frozen[kind]) != SOURCES:
            raise ValueError('incomplete config standard-library input inventory')
        sources[kind] = {}
        for name, expected in frozen[kind].items():
            path = sysroot/'lib/rustlib/src/rust/library'/name
            if sha(path) != expected:
                raise ValueError('fixed config standard-library source changed: '+kind+'/'+name)
            sources[kind][name] = dict(path=str(path), sha256=sha(path))
    backend = root/'.tools/aeneas/backends/lean/Aeneas'
    return dict(symbols=sorted(EXTERNAL), inherited=inherited, sources=sources,
        models=frozen['models'], proposed_symbols=sorted(PROPOSED), v0_string_bindings_adopted=False,
        transparent_stdlib=['u8::is_ascii_graphic','RangeInclusive::contains',
            'RangeBounds::contains','RangeInclusive::start_bound','RangeInclusive::end_bound'],
        backend={n:sha(backend/n) for n in ['Std/Core/Iter.lean','Std/Range.lean','Std/Scalar/Core.lean']},
        stdlib_proved=False, business_opaque=False, host_authentication_proved=False)


def result_tags(raw):
    result = parent(Path(__file__).resolve().parents[4]).result_tags(raw)
    name = [{'Ident':[n,0]} for n in ['core','ops','range','Bound']]
    types = [t for t in raw['translated']['type_decls'] if t and t['item_meta']['name'] == name]
    if len(types) != 1 or 'Enum' not in types[0]['kind']:
        raise ValueError('missing Bound logical tags')
    variants = types[0]['kind']['Enum']
    expected = [(n, {'Signed':['Isize',str(i)]}) for i,n in enumerate(['Included','Excluded','Unbounded'])]
    if [(v['name'],v['discriminant']) for v in variants] != expected:
        raise ValueError('changed Bound logical tags')
    return dict(result=result, bound=variants)
