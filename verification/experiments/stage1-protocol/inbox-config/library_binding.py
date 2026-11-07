"""Check four existing per-symbol bodies and fixed source/arithmetic inputs."""
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import re
import subprocess

HERE = Path('verification/experiments/stage1-protocol/inbox-config')
EXTERNAL = {
    'Bool.Insts.CoreCmpPartialEqBool.ne',
    'core.option.Option.Insts.CoreCmpPartialEqOption.eq',
    'alloc.string.String.Insts.CoreCmpPartialEqString.eq', 'alloc.vec.Vec.is_empty',
}
SOURCES = {
    'core/src/convert/num.rs', 'core/src/num/mod.rs', 'core/src/ops/arith.rs',
    'core/src/result.rs',
}


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def parent(root):
    path = root/HERE.parent/'inbox-renewal-session/library_binding.py'
    spec = importlib.util.spec_from_file_location('config_existing_body_and_tags', path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def check(root):
    previous = parent(root)
    current = (root/HERE/'FunsExternal.lean.in').read_text()
    inherited = (root/HERE.parent/'inbox-origin-selection/FunsExternal.lean.in').read_text()
    if set(re.findall(r'^def (\S+)', current, re.M)) != EXTERNAL:
        raise ValueError('unreviewed config function binding')
    for name in EXTERNAL:
        if previous.body(current, name) != previous.body(inherited, name):
            raise ValueError('existing per-symbol body changed: '+name)
    fixed = json.loads((root/'verification/foundations-environment.json').read_text())
    frozen = json.loads((root/HERE/'library-sources.json').read_text())
    sources = {}
    for kind, selector in [('production', []), ('extraction', ['+'+fixed['extraction_frontend']])]:
        sysroot = Path(subprocess.check_output(['rustc', *selector, '--print', 'sysroot'], text=True).strip())
        if set(frozen[kind]) != SOURCES:
            raise ValueError('incomplete config source inventory')
        sources[kind] = {}
        for name, expected in frozen[kind].items():
            path = sysroot/'lib/rustlib/src/rust/library'/name
            if sha(path) != expected:
                raise ValueError('fixed config library source changed: '+kind+'/'+name)
            sources[kind][name] = dict(path=str(path), sha256=sha(path))
    backend = root/'.tools/aeneas/backends/lean/Aeneas'
    return dict(symbols=sorted(EXTERNAL), builtin_casts=['FromU64U16.from', 'FromU64U32.from'], sources=sources,
        backend={name: sha(backend/name) for name in ['Std/Scalar/CoreConvertNum.lean', 'Std/Scalar/Ops/Mul.lean',
            'Std/Scalar/Ops/Add.lean', 'Std/Scalar/Core.lean', 'Std/Core/Result.lean', 'Data/Discriminant.lean']},
        stdlib_proved=False, result_tags_adopted=False, library_models_reused=True,
        physical_layout_claim=False, borrowed_vec_model_used=False)


def result_tags(raw):
    return parent(Path(__file__).resolve().parents[4]).result_tags(raw)
