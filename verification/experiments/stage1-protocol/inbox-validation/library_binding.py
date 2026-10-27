"""Check exact previously registered value models and the extracted Result primitive."""
from hashlib import sha256
from pathlib import Path
import importlib.util
import re

HERE = Path('verification/experiments/stage1-protocol/inbox-validation')
GENERATED = Path('verification/aeneas/stage1-protocol/request/SandproofStage1Request')
GRAPHIC = Path('verification/aeneas/stage1-protocol/graphic/SandproofStage1Graphic')
SOURCE = 'verification/aeneas/foundations/SandproofFoundations/FunsExternal.lean'
EXTERNAL = {
    'Bool.Insts.CoreCmpPartialEqBool.ne', 'core.option.Option.is_some_and',
    'core.option.Option.is_none_or', 'core.option.Option.as_ref',
    'core.option.Option.Insts.CoreCmpPartialEqOption.eq', 'core.result.Result.ok',
    'core.result.Result.is_err', 'alloc.string.String.Insts.CoreCmpPartialEqString.eq',
    'alloc.string.String.len', 'alloc.string.String.is_empty',
    'alloc.string.String.Insts.CoreCloneClone.clone',
    'alloc.string.String.Insts.CoreOpsDerefDerefStr.deref', 'alloc.vec.Vec.is_empty',
    'SharedAVec.Insts.CoreIterTraitsCollectIntoIteratorSharedATIter.into_iter',
}
PRIMITIVE_EXTERNAL = {'core.str.Str.bytes',
    'core.str.iter.Bytes.Insts.CoreIterTraitsIteratorIteratorU8.all'}


def parent(root):
    spec = importlib.util.spec_from_file_location('inbox_registered_bindings', root/HERE.parent/'session-shape/library_binding.py')
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def fragment(text, name, attributes=False):
    return parent(Path(__file__).resolve().parents[4]).fragment(text, name, attributes)


def check(root):
    existing = parent(root)
    existing_record = existing.check(root)
    source = (root/SOURCE).read_text()
    shared = (root/HERE/'SharedValueBindings.lean.in').read_text()
    symbols = re.findall(r'^def (\S+)', shared, re.M)
    pieces = {n:fragment(source,n,True) for n in symbols}
    expected = ('import SandproofFoundations.IdentifierLibrary\nopen Aeneas Aeneas.Std\nnamespace SandproofFoundationsRust\n\n'
        +'\n\n'.join(pieces.values())+'\n\nend SandproofFoundationsRust\n')
    if shared != expected:
        raise ValueError('changed existing model fragment or attributes')
    wrappers = (root/HERE/'FunsExternal.lean.in').read_text()
    if set(re.findall(r'^def (\S+)',wrappers,re.M)) != EXTERNAL:
        raise ValueError('unreviewed wrapper symbol')
    providers = {}
    for name in EXTERNAL - {'core.result.Result.is_err'}:
        package = ('inbox-config-validation' if name in ('alloc.string.String.len',
            'alloc.string.String.is_empty','alloc.string.String.Insts.CoreOpsDerefDerefStr.deref')
            else 'inbox-record' if name in ('core.result.Result.ok','alloc.string.String.Insts.CoreCloneClone.clone')
            else 'inbox-entry-validation')
        path = HERE.parent/package/'FunsExternal.lean.in'
        if fragment(wrappers,name) != fragment((root/path).read_text(),name):
            raise ValueError('changed registered wrapper: '+name)
        providers[name] = dict(path=str(path), sha256=sha(root/path))
    actual_result = 'def core.result.Result.is_err {T E : Type} (value : core.result.Result T E) : Result Bool :=\n  SandproofStage1GraphicRust.core.result.Result.is_err value'
    if fragment(wrappers,'core.result.Result.is_err').strip() != actual_result:
        raise ValueError('Result wrapper must call the independently extracted implementation')
    graphic = (root/HERE/'GraphicFunsExternal.lean.in').read_text()
    if set(re.findall(r'^def (\S+)',graphic,re.M)) != PRIMITIVE_EXTERNAL:
        raise ValueError('unreviewed primitive wrapper')
    for name in PRIMITIVE_EXTERNAL:
        # Identical to the existing wrapper; only its namespace/import view differs.
        provider = root/HERE.parent/'inbox-config-validation/FunsExternal.lean.in'
        if fragment(graphic,name) != fragment(provider.read_text(),name):
            raise ValueError('changed primitive wrapper: '+name)
    files,_ = existing.contents(root)
    if (root/HERE/'TypesExternal.lean.in').read_text() != files['TypesExternal.lean']:
        raise ValueError('changed registered error carriers')
    graphic_types = 'import SandproofFoundations.TypesExternal\nnamespace SandproofStage1GraphicRust\nabbrev core.str.iter.Bytes := SandproofFoundationsRust.core.str.iter.Bytes\nend SandproofStage1GraphicRust\n'
    if (root/HERE/'GraphicTypesExternal.lean.in').read_text() != graphic_types:
        raise ValueError('changed registered Bytes carrier')
    return dict(previous=existing_record, source_sha256=sha(root/SOURCE),
        shared_fragments={n:sha256(v.encode()).hexdigest() for n,v in pieces.items()},
        wrappers=providers, symbols=sorted(EXTERNAL), primitive_symbols=sorted(PRIMITIVE_EXTERNAL),
        new_business_models=False, original_equivalence_proved=False,
        outputs={n:sha(root/HERE/n) for n in ('TypesExternal.lean.in','FunsExternal.lean.in',
            'SharedValueBindings.lean.in','GraphicTypesExternal.lean.in','GraphicFunsExternal.lean.in')})


def install(root,project):
    check(root)
    installed = {}
    for prefix,dest,names in [('',GENERATED,['TypesExternal','FunsExternal','SharedValueBindings']),
                              ('Graphic',GRAPHIC,['TypesExternal','FunsExternal'])]:
        (project/dest).mkdir(parents=True,exist_ok=True)
        for name in names:
            target = project/dest/(name+'.lean')
            target.write_bytes((root/HERE/(prefix+name+'.lean.in')).read_bytes())
            installed[str(target.relative_to(project))] = sha(target)
    return installed


def result_tags(raw):
    return parent(Path(__file__).resolve().parents[4]).result_tags(raw)
