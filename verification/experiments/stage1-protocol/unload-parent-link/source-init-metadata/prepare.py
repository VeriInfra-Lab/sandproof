"""Materialize the real metadata continuation beside the full audited row closure."""
import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import sys
import textwrap
from tempfile import TemporaryDirectory
from types import SimpleNamespace

import extract
from prepare_world_begin import closure, read_exact
from prepare_retention_providers import declarations

HERE = Path(__file__).resolve().parent
INITIALIZER = 'controller.lifecycle_inbox.source_work.codec.initialize'
INPUT_KEYS = {'actual','reference','prior','audit','parent_inputs'}


def load_row():
    directory = HERE.parent / 'source-init-row'
    previous = sys.modules.pop('extract',None)
    sys.path.insert(0,str(directory))
    try:
        spec = importlib.util.spec_from_file_location('source_init_row_prepare',directory/'prepare.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module
    finally:
        sys.path.remove(str(directory))
        sys.modules.pop('extract',None)
        if previous is not None:
            sys.modules['extract'] = previous


def captured(directory,variant,base):
    result = json.loads((directory/'result.json').read_bytes())
    if (result['source'],result['tree'],result['variant'],result['options'],result['driver'],
            result['parent_driver'],result['complete']) != (
            extract.SOURCE,extract.TREE,variant,extract.OPTIONS,extract.parent.sql.digest(Path(extract.__file__)),
            extract.parent.sql.digest(Path(extract.parent.__file__)),True):
        raise ValueError('the exact completed current metadata capture is required')
    if [(r['label'],r['exit_code'],r['bounded_stop']) for r in result['commands']] != [
            ('charon',0,False),('aeneas',0,False)]:
        raise ValueError('both fixed tools must complete normally')
    read_exact(directory/'source.tar',extract.ARCHIVE)
    for command in result['commands']:
        read_exact(directory/(command['label']+'.log'),command['log_sha256'])
    with TemporaryDirectory(prefix='source-init-metadata-input-') as temporary:
        source = Path(temporary)
        extract.reconstruct(directory/'source.tar',source,variant)
        if set(result['staged']) != {str(p) for p in (extract.CODEC,extract.LIB)}:
            raise ValueError('the entire two-file staging inventory is required')
        for name,wanted in result['staged'].items():
            if read_exact(directory/'source'/name,wanted) != (source/name).read_bytes():
                raise ValueError('the original isolation and sole metadata reservation mutation differ')
    read_exact(directory/'actual.llbc',result['ir'])
    generated = directory/'generated'
    expected = {'translation.json',*(extract.NAMESPACE+'/'+n+'.lean' for n in (
        'Types','Funs','TypesExternal_Template','FunsExternal_Template'))}
    if set(result['generated']) != expected or {str(p.relative_to(generated)) for p in generated.rglob('*') if p.is_file()} != expected:
        raise ValueError('the complete original generation inventory is required')
    for name,wanted in result['generated'].items():
        read_exact(generated/name,wanted)
    fixed = json.loads((extract.parent.sql.ROOT/'verification/foundations-environment.json').read_bytes())
    if extract.parent.admit(directory/'actual.llbc',generated,fixed['target']) != result['admission']:
        raise ValueError('the complete real caller/helper mapping differs')
    return result,base.parsed(directory,'Types'),base.parsed(directory,'Funs')


def suffix(function):
    body = declarations(function)[INITIALIZER].decode()
    first = '              let ni ←\n'
    last = '\n            | core.ops.control_flow.ControlFlow.Break residual =>'
    if body.count(first)!=1 or body.count(last)!=1:
        raise ValueError('one complete actual post-capacity continuation is required')
    body = textwrap.indent(textwrap.dedent(body[body.index(first):body.index(last)]),'  ')
    return ('def afterAcceptedCapacity (connection : rusqlite.Connection)\n'
        '  (config : capture.source.SourceObservationConfig)\n'
        '  (inbox : controller.lifecycle_inbox.model.LifecycleInboxConfig)\n'
        '  (val reserve : U64) : Result (core.result.Result Unit JournalError) := do\n'+body+'\n').encode()


def prepare(args):
    row = load_row()
    base = row.load_parent()
    reference,types,functions = row.captured(args.reference,'original',base)
    if args.actual==args.reference:
        selected='original'
        actual,actual_types,actual_functions=reference,types,functions
    else:
        selected='metadata-reserve-short'
        actual,actual_types,actual_functions=captured(args.actual,selected,base)
    if types!=actual_types or functions.keys()!=actual_functions.keys():
        raise ValueError('all generated types and function names must remain identical')
    changed={n for n in functions if functions[n]!=actual_functions[n]}
    if changed!=(set() if selected=='original' else {INITIALIZER}):
        raise ValueError('only the actual initialization metadata argument may change')
    old,prior_audit=base.load_policy().load_count().parent_prepare().verify_prior(
        args.prior,args.audit,'SourceInitRow',93,408)
    locations_bytes=args.row_inputs.read_bytes()
    locations=json.loads(locations_bytes)
    if set(locations)!=INPUT_KEYS or not all(isinstance(p,str) for p in locations.values()):
        raise ValueError('the exact five original row input locations are required')
    with TemporaryDirectory(prefix='source-init-metadata-parent-') as temporary:
        fresh=Path(temporary)/'parent'
        row.prepare(SimpleNamespace(**{n:Path(p).resolve(strict=True) for n,p in locations.items()},
            output=fresh,lake_packages=args.lake_packages,lean_mcp=args.lean_mcp))
        if (fresh/'plan.json').read_bytes()!=(args.prior/'plan.json').read_bytes():
            raise ValueError('the entire original row plan must reconstruct')
    if Path(locations['actual'])!=args.reference or Path(locations['reference'])!=args.reference:
        raise ValueError('the exact original correct capture must be shared')
    original_suffix=suffix(functions[INITIALIZER])
    if original_suffix not in (args.prior/'SourceInitCapacityView.lean').read_bytes():
        raise ValueError('the complete original continuation must equal the prior actual body')
    args.output.mkdir(mode=0o700)
    for name,target in (('.lake/packages',args.lake_packages),('.tools/lean-mcp',args.lean_mcp)):
        link=args.output/name;link.parent.mkdir();link.symlink_to(target,target_is_directory=True)
    for name,wanted in old['files'].items():
        path=args.output/name;path.parent.mkdir(parents=True,exist_ok=True)
        path.write_bytes(read_exact(args.prior/name,wanted))
    for name in ('lean-toolchain','lake-manifest.json','tools/lean_session.py','scripts/lean-mcp-server.sh'):
        body=(extract.parent.sql.ROOT/name).read_bytes()
        if body!=(args.prior/name).read_bytes():
            raise ValueError('fixed support must remain byte-identical')
        path=args.output/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(body)
    root='SourceInitMetadata' if selected=='original' else 'SourceInitMetadataNegative'
    modules={n:(HERE/(n+'.lean.in')).read_bytes() for n in ('SourceInitMetadataView','SourceInitMetadataSpec','SourceInitMetadata',root)}
    if selected!='original':
        view=(HERE/'SourceInitMetadataWrongView.lean.in').read_bytes()
        marker=b'-- ACTUAL WRONG SUFFIX\n'
        if view.count(marker)!=1:raise ValueError('one exact wrong suffix insertion is required')
        modules['SourceInitMetadataWrongView']=view.replace(marker,suffix(actual_functions[INITIALIZER])+b'\n')
    lake=(args.prior/'lakefile.toml').read_text()
    for n,body in modules.items():
        (args.output/(n+'.lean')).write_bytes(body);lake+=f'\n[[lean_lib]]\nname = "{n}"\n'
    (args.output/'lakefile.toml').write_text(lake)
    files=closure(args.output,[root+'.lean'],active_view='lakefile.toml')
    if set(files)!=set(old['files'])|{n+'.lean' for n in modules} or any(files[n]!=h for n,h in old['files'].items()):
        raise ValueError('all inherited and new actual physical files are required')
    plan=dict(root=root,variant=selected,source=extract.SOURCE,reference=reference,actual=actual,
        parent_plan=sha256((args.prior/'plan.json').read_bytes()).hexdigest(),parent_audit=prior_audit,
        parent_locations=sha256(locations_bytes).hexdigest(),shared_types=sorted(types),shared_functions=sorted(functions),
        original_suffix=sha256(original_suffix).hexdigest(),selected_suffix=sha256(suffix(actual_functions[INITIALIZER])).hexdigest(),
        files=files,strict_files=sorted(files),modules={n:{'sha256':h} for n,h in files.items()},
        build_order=[n.removesuffix('.lean') for n in files])
    (args.output/'plan.json').write_text(json.dumps(plan,indent=2)+'\n')
    print(root,len(files),'complete actual physical Lean files')


if __name__ == '__main__':
    cli=argparse.ArgumentParser(description=__doc__)
    for name in ('actual','reference','prior','audit','row-inputs','output','lake-packages','lean-mcp'):
        cli.add_argument('--'+name,type=Path,required=True)
    args=cli.parse_args()
    for n,p in vars(args).items():setattr(args,n,p.resolve(strict=n!='output'))
    prepare(args)
