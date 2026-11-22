"""Strict raw candidate evidence; never admit unadopted JSON/SHA boundaries."""
import importlib.util
import json
from pathlib import Path
import re
import sys
import tarfile
from tempfile import TemporaryDirectory

HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE.parent/'source-remove-retired'))
def module(name,path):
    spec=importlib.util.spec_from_file_location(name,path)
    value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value
capture=module('continue_binding_validate_capture',HERE/'capture.py')
tuples=module('continue_binding_validate_tuple',HERE/'tuple.py')
parent=module('continue_binding_validate_parent',HERE.parent/'source-end-accessor/validation.py')
inputs=parent.original_inputs
digest=inputs.digest
REPO=capture.retire.REPO
ROOT='capture.continuation.OriginalContinueWorkRequest.binding_digest'
INPUT='capture.continuation.OriginalContinueWorkRequest.binding_input'
VIEWS=(('inline','original'),('inline','file-policy-as-zero'),
       ('tuple','original'),('tuple','file-policy-as-zero'))
FILES=('Types','Funs','TypesExternal_Template','FunsExternal_Template')

def parsed(data,engine):
    return inputs.parsed(data,engine.NAMESPACE)

def current(directory,view,variant):
    if (view,variant) not in VIEWS:
        raise ValueError('all four complete actual views are mandatory')
    engine=capture if view=='inline' else tuples.base
    reconstruct=capture.reconstruct if view=='inline' else tuples.reconstruct
    record=json.loads((directory/'result.json').read_bytes())
    expected=dict(source=capture.retire.SOURCE,tree=capture.retire.TREE,variant=variant,
        options=engine.OPTIONS,driver=digest(HERE/'capture.py'),
        base_driver=digest(Path(engine.retire.__file__)),
        ports_driver=digest(Path(engine.retire.ports.__file__)),
        parent_driver=digest(Path(engine.parent.__file__)),
        candidate_ports_driver=digest(HERE/'ports.py'),capture_roots=list(engine.ROOTS),
        complete_binding_fields=6,complete=False,behavior_proved=False,
        interface_adoption_complete=False,public_acceptance=False)
    if any(record.get(k)!=v for k,v in expected.items()) or record.get('preflight_only'):
        raise ValueError('exact real candidate capture, explicit unadopted status and current drivers required')
    if view=='tuple' and (record.get('tuple_driver')!=digest(HERE/'tuple.py')
            or record.get('transparent_tuple_factor') is not True):
        raise ValueError('the original transparent tuple transformation is mandatory')
    if [(c['label'],c['exit_code'],c['bounded_stop']) for c in record['commands']] != [
            ('charon',0,False),('aeneas',0,False)]:
        raise ValueError('both real fixed tools must complete; tool failure is not a counterexample')
    fixed=json.loads((REPO/'verification/foundations-environment.json').read_bytes())
    ir,generated=directory/'actual.llbc',directory/'generated'
    selectors=[x for flag,key in (('--start-from','roots'),('--start-from-if-exists','retained_roots'),
        ('--include','include'),('--opaque','opaque')) for item in engine.OPTIONS[key] for x in (flag,item)]
    tails=[['cargo','--preset=aeneas','--error-on-warnings','--sysroot','default',*selectors,
            '--dest-file',str(ir),'--','--manifest-path','controller-service/Cargo.toml',
            '--lib','--locked','--offline','--target',fixed['target']],
           ['-backend','lean','-abort-on-error','-warnings-as-errors','-split-files','-emit-json',
            '-namespace',engine.NAMESPACE,'-subdir',engine.NAMESPACE,'-dest',str(generated),str(ir)]]
    for command,tail in zip(record['commands'],tails,strict=True):
        binary=Path(command['argv'][0])
        if command['argv'][1:]!=tail or not binary.is_absolute() or digest(binary)!=fixed[command['label']]['binary_sha256']:
            raise ValueError('exact fixed commands and actual binaries required')
        inputs.read_exact(directory/(command['label']+'.log'),command['log_sha256'])
    inputs.read_exact(directory/'source.tar',capture.retire.ARCHIVE)
    if set(record['staged'])!={str(n) for n in capture.STAGED}:
        raise ValueError('all thirteen exact staged files required')
    with TemporaryDirectory(prefix='continue-binding-readback-',dir=directory.parent) as temporary:
        source=Path(temporary);reconstruct(directory/'source.tar',source,variant)
        with tarfile.open(directory/'source.tar') as archive:
            members={m.name:m for m in archive.getmembers() if m.isfile()}
        files=list((directory/'source').rglob('*'))
        if len(members)!=4906 or any(p.is_symlink() for p in files):
            raise ValueError('all4906 real original files, without symlink substitution, required')
        if {str(p.relative_to(directory/'source')) for p in files if p.is_file()}!=set(members):
            raise ValueError('whole source inventory differs')
        for name,member in members.items():
            p=directory/'source'/name
            if digest(p)!=digest(source/name) or bool(p.stat().st_mode&0o111)!=bool(member.mode&0o111):
                raise ValueError('actual source differs from exact full reconstruction: '+name)
        for name,wanted in record['staged'].items():
            inputs.read_exact(directory/'source'/name,wanted)
    inputs.read_exact(ir,record['ir'])
    wanted={'translation.json',*(engine.NAMESPACE+'/'+name+'.lean' for name in FILES)}
    if set(record['generated'])!=wanted or {str(p.relative_to(generated)) for p in generated.rglob('*') if p.is_file()}!=wanted:
        raise ValueError('every actual generated file is mandatory')
    for name,value in record['generated'].items():
        inputs.read_exact(generated/name,value)
    if engine.candidate(ir,generated,fixed['target'])!=record.get('candidate_mapping'):
        raise ValueError('all9/10 transparent business roots and exact31 candidates required')
    sources={name:(generated/engine.NAMESPACE/(name+'.lean')).read_bytes() for name in FILES}
    opaque=set(re.findall(rb'^axiom\s+([^\s(:]+)',sources['FunsExternal_Template'],re.M))
    if opaque!={n.encode() for n in engine.parent.LIBRARIES|engine.CANDIDATES}:
        raise ValueError('exact old29 plus only two unadopted thin interfaces required')
    return record,sources,engine

def inherited(project,audit,retention,retention_audit,reference,engine):
    plan,evidence=inputs.prior(project,audit,'SourceEndAccessor',213,1199)
    kept,kept_evidence=inputs.prior(retention,retention_audit,'RetentionProviders',26,127)
    if (plan['source'],plan['tree'])!=(capture.retire.SOURCE,capture.retire.TREE):
        raise ValueError('same complete fixed production source required')
    if any(plan['files'].get(n)!=h for n,h in kept['files'].items()):
        raise ValueError('all26/127 original retained providers remain mandatory')
    parent_sources={n:(project/n).read_bytes() for n in plan['files']}
    shared={}
    for group,count,added in [('Types',117,set()),('Funs',191,{ROOT})]:
        rows=parsed(reference[group],engine);matched={};unmatched=set()
        for name,body in rows.items():
            owners=[n for n,raw in parent_sources.items() if raw.count(body)==1]
            if owners:matched[name]=owners
            else:unmatched.add(name)
        if len(matched)!=count or unmatched!=added or len(rows)!=count+len(added):
            raise ValueError('only the whole original191 functions/117 types and one new actual entry allowed: '+group)
        shared[group]=matched
    return plan,dict(parent=evidence,retention=kept_evidence,shared=shared)

def interfaces_and_bodies(views):
    original=views[('inline','original')]
    for key,(record,sources,engine) in views.items():
        types=parsed(sources['Types'],engine)
        if types!=parsed(original[1]['Types'],original[2]):
            raise ValueError('all117 complete types must remain identical across all actual views')
        funcs=parsed(sources['Funs'],engine)
        expected=set(parsed(original[1]['Funs'],original[2]))|({INPUT} if key[0]=='tuple' else set())
        if set(funcs)!=expected or len(funcs)!=(192 if key[0]=='inline' else 193):
            raise ValueError('exact192/193 complete function inventories required')
        before=parsed(original[1]['Funs'],original[2])
        if any(funcs[n]!=body for n,body in before.items() if n!=ROOT):
            raise ValueError('every191 original complete function must remain unchanged')
    for view in ('inline','tuple'):
        _,before,engine=views[(view,'original')];_,after,_=views[(view,'file-policy-as-zero')]
        for group in ('Types','TypesExternal_Template','FunsExternal_Template'):
            if before[group]!=after[group]:
                raise ValueError('whole types and external declarations unchanged within each paired view')
        old,new=parsed(before['Funs'],engine),parsed(after['Funs'],engine)
        changed=[n for n in old if old[n]!=new[n]]
        target=ROOT if view=='inline' else INPUT
        if changed!=[target]:
            raise ValueError('only the exact single real tuple policy omission may differ')
    return dict(complete_types=117,inherited_functions=191,
        inline_functions=192,factored_functions=193,exact_candidate_interfaces=31,
        complete_original_inputs=True,interface_adoption_complete=False)

def extra_inputs():
    return sorted(set(parent.extra_inputs())|{
        str(p.relative_to(REPO)) for p in HERE.iterdir() if p.is_file()})
