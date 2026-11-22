"""Strict full caller/shared-policy captures; no external adoption upgrade."""
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
capture=module('continue_network_validation_capture',HERE/'capture.py')
parent=module('continue_network_validation_parent',HERE.parent/'continue-work-binding/validation.py')
inputs=parent.inputs
digest=inputs.digest
REPO=parent.REPO
SUPPORT=parent.parent.SUPPORT
ROOT=parent.ROOT
INPUT='capture.continuation.OriginalContinueWorkRequest.binding_input_with_policy'
VIEWS=(('shared','original'),('shared','network-policy-as-deny-all'),('inline','network-policy-as-deny-all'))
FILES=('Types','Funs','TypesExternal_Template','FunsExternal_Template')

def parsed(data):return inputs.parsed(data,capture.NAMESPACE)

def current(directory,view,variant):
    if (view,variant) not in VIEWS:raise ValueError('all three actual original policy forwarding views required')
    engine=capture.configure(view);record=json.loads((directory/'result.json').read_bytes())
    expected=dict(source=engine.retire.SOURCE,tree=engine.retire.TREE,variant=variant,options=engine.OPTIONS,
        driver=digest(Path(engine.__file__)),base_driver=digest(Path(engine.retire.__file__)),
        ports_driver=digest(Path(engine.retire.ports.__file__)),parent_driver=digest(Path(engine.parent.__file__)),
        candidate_ports_driver=digest(Path(engine.ports.__file__)),capture_roots=list(engine.ROOTS),
        complete_binding_fields=6,complete=False,behavior_proved=False,public_acceptance=False,
        interface_adoption_complete=False,network_forwarding_driver=digest(HERE/'capture.py'),
        network_forwarding_view=view,actual_forwarding_proved=False,property_refutation=False)
    if any(record.get(k)!=value for k,value in expected.items()) or record.get('preflight_only'):
        raise ValueError('complete exact current raw capture and explicit unadopted boundary status required')
    if [(r['label'],r['exit_code'],r['bounded_stop']) for r in record['commands']]!=[('charon',0,False),('aeneas',0,False)]:
        raise ValueError('both fixed tools must actually complete, not a tool failure counterexample')
    fixed=json.loads((REPO/'verification/foundations-environment.json').read_bytes())
    ir,generated=directory/'actual.llbc',directory/'generated'
    selectors=[part for flag,key in (('--start-from','roots'),('--start-from-if-exists','retained_roots'),
        ('--include','include'),('--opaque','opaque')) for pattern in engine.OPTIONS[key] for part in (flag,pattern)]
    tails=[['cargo','--preset=aeneas','--error-on-warnings','--sysroot','default',*selectors,'--dest-file',str(ir),
        '--','--manifest-path','controller-service/Cargo.toml','--lib','--locked','--offline','--target',fixed['target']],
        ['-backend','lean','-abort-on-error','-warnings-as-errors','-split-files','-emit-json','-namespace',capture.NAMESPACE,
        '-subdir',capture.NAMESPACE,'-dest',str(generated),str(ir)]]
    for command,tail in zip(record['commands'],tails,strict=True):
        binary=Path(command['argv'][0])
        if not binary.is_absolute() or command['argv'][1:]!=tail or digest(binary)!=fixed[command['label']]['binary_sha256']:
            raise ValueError('exact original command, options, target and actual fixed binary required')
        inputs.read_exact(directory/(command['label']+'.log'),command['log_sha256'])
    inputs.read_exact(directory/'source.tar',engine.retire.ARCHIVE)
    if set(record['staged'])!={str(p) for p in engine.STAGED}:raise ValueError('all thirteen exact staged files required')
    with TemporaryDirectory(prefix='continue-network-readback-',dir=directory.parent) as temporary:
        source=Path(temporary);engine.reconstruct(directory/'source.tar',source,variant)
        with tarfile.open(directory/'source.tar') as archive:
            members={m.name:m for m in archive.getmembers() if m.isfile()}
        paths=list((directory/'source').rglob('*'))
        if len(members)!=4906 or any(p.is_symlink() for p in paths):raise ValueError('all4906 original ordinary files required')
        if {str(p.relative_to(directory/'source')) for p in paths if p.is_file()}!=set(members):raise ValueError('full source inventory changed')
        for name,member in members.items():
            p=directory/'source'/name
            if digest(p)!=digest(source/name) or bool(p.stat().st_mode&0o111)!=bool(member.mode&0o111):
                raise ValueError('whole actual source/execute modes differ: '+name)
        for name,wanted in record['staged'].items():inputs.read_exact(directory/'source'/name,wanted)
    inputs.read_exact(ir,record['ir'])
    names={'translation.json',*(capture.NAMESPACE+'/'+n+'.lean' for n in FILES)}
    if set(record['generated'])!=names or {str(p.relative_to(generated)) for p in generated.rglob('*') if p.is_file()}!=names:
        raise ValueError('all actual generated files required')
    for name,wanted in record['generated'].items():inputs.read_exact(generated/name,wanted)
    if engine.candidate(ir,generated,fixed['target'])!=record.get('candidate_mapping'):
        raise ValueError('all actual business methods and exactly31 typed candidate boundaries required')
    sources={name:(generated/capture.NAMESPACE/(name+'.lean')).read_bytes() for name in FILES}
    libraries=set(re.findall(rb'^axiom\s+([^\s(:]+)',sources['FunsExternal_Template'],re.M))
    if libraries!={name.encode() for name in engine.parent.LIBRARIES|engine.CANDIDATES}:
        raise ValueError('unchanged old29 plus only the same two unadopted concrete uses required')
    return record,sources

def inherited(project,audit,reference):
    plan,evidence=inputs.prior(project,audit,'ContinueWorkBinding',220,1222)
    engine=capture.configure('shared')
    if (plan['source'],plan['tree'])!=(engine.retire.SOURCE,engine.retire.TREE):raise ValueError('same complete production source/tree required')
    if plan.get('interface_adoption_complete') is not False or plan.get('public_acceptance') is not False:
        raise ValueError('the parent conditional result must not be upgraded to public adoption')
    raw={name:(project/name).read_bytes() for name in plan['files']};shared={}
    for group,count,new in [('Types',117,set()),('Funs',191,{ROOT,INPUT})]:
        rows=parsed(reference[group]);matched={};extra=set()
        for name,body in rows.items():
            owners=[n for n,value in raw.items() if value.count(body)==1]
            if owners:matched[name]=owners
            else:extra.add(name)
        if len(matched)!=count or extra!=new or len(rows)!=count+len(new):
            raise ValueError('all original whole parent types/functions and only exact forwarding additions required: '+group)
        shared[group]=matched
    return plan,dict(parent=evidence,shared=shared)

def interfaces_and_bodies(views):
    original=views[('shared','original')][1];correct=parsed(original['Funs'])
    for (view,variant),(_,sources) in views.items():
        functions=parsed(sources['Funs']);expected=set(correct)-({INPUT} if view=='inline' else set())
        if parsed(sources['Types'])!=parsed(original['Types']) or len(parsed(sources['Types']))!=117:
            raise ValueError('all117 complete types must remain unchanged')
        if set(functions)!=expected or len(functions)!=(193 if view=='shared' else 192):
            raise ValueError('exact193/192 complete functions required')
        if any(functions[name]!=body for name,body in correct.items() if name not in (ROOT,INPUT)):
            raise ValueError('all191 complete inherited functions must remain unchanged')
        if view=='shared' and functions[INPUT]!=correct[INPUT]:
            raise ValueError('the same transparent tuple helper is required in both variants')
    wrong=views[('shared','network-policy-as-deny-all')][1]
    for group in ('Types','TypesExternal_Template','FunsExternal_Template'):
        if original[group]!=wrong[group]:raise ValueError('all whole types/external declarations must agree: '+group)
    actual=parsed(wrong['Funs'])
    if [name for name in correct if correct[name]!=actual[name]]!=[ROOT]:raise ValueError('only the original actual caller policy argument may differ')
    return dict(complete_types=117,inherited_functions=191,shared_functions=193,inline_functions=192,
        identical_transparent_helper=True,only_wrong_caller_policy_argument_changed=True,
        exact_candidate_interfaces=31,interface_adoption_complete=False)

def extra_inputs():
    return sorted(set(parent.extra_inputs())|{str(p.relative_to(REPO)) for p in HERE.iterdir() if p.is_file()})
