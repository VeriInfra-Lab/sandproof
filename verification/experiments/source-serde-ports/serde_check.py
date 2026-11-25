"""Check actual Serde read/required-option proofs plus exact B4 writer admission separately."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import re
import sys
import tarfile
import tempfile

from serde_inputs import (HERE, DECISION, SOURCE_WORK, SOURCE_FILES, PARENT, NAMESPACE, ROOT,
                          configuration, retention, row_provider, stage)
from serde_capture import admit
from serde_prepare import materialize
from canonical_check import view as current_capture
from rows_prepare import declarations
from load_prepare import read_exact
from policy_check import checked_diagnostics
from check import dependencies, run
from binding import CALLS
from library import capture as library_capture
from schema import schema

sys.path.insert(0,str(DECISION))
from decision_inputs import exact_package

CAPTURES = {'required-correct': ('required-only','correct'),
            'required-wrong': ('required-only','drop-required-option-value'),
            'decode': ('decode-only','correct'), 'call-admission': ('call-admission','correct')}


def checked_parent(output,rules):
    receipt=json.loads((output/'result.json').read_bytes())
    if receipt['complete'] is not True or receipt['rule_inputs']!=rules:
        raise ValueError('the complete exact #1048 parent proof is required')
    parent=output/'project';plan=json.loads((parent/'plan.json').read_bytes())
    checked_diagnostics(plan,json.loads((output/'diagnostics/result.json').read_bytes()))
    return parent,plan


def checked_view(folder, profile, mode, row):
    receipt=json.loads((folder/'result.json').read_bytes())
    rules={name:retention.sha(HERE/name) for name in ('serde_inputs.py','serde_capture.py','binding.py')}
    expected=[('charon',0,False)] + ([] if profile=='call-admission' else [('aeneas',0,False)])
    if (receipt['complete'] is not True or receipt['public_acceptance'] is not False
            or receipt['lean_generated']!=(profile!='call-admission')
            or (receipt['profile'],receipt['mode'],receipt['source'],receipt['configuration'],receipt['rules'])!=
            (profile,mode,retention.SOURCE,configuration(profile),rules)
            or [(x['label'],x['exit_code'],x['bounded_stop']) for x in receipt['commands']]!=expected):
        raise ValueError('actual selected complete capture/admission receipt required')
    for file,digest in [('actual.llbc',receipt['ir']),*( (x['label']+'.log',x['log_sha256']) for x in receipt['commands'])]:
        if retention.sha(folder/file)!=digest: raise ValueError('actual extraction evidence changed: '+file)
    archive=folder/'source.tar'
    if retention.sha(archive)!=retention.SOURCE_ARCHIVE: raise ValueError('original source archive differs')
    with tempfile.TemporaryDirectory(dir=folder.parent,prefix='serde-replay-') as temporary:
        expected=Path(temporary)
        with tarfile.open(archive) as stream: stream.extractall(expected,filter='data')
        stage(row,expected,mode)
        actual=folder/'source'
        rust={str(path):retention.sha(actual/path) for path in SOURCE_FILES}
        if receipt['rust']!=rust: raise ValueError('selected original source differs')
        names={p.relative_to(expected) for p in expected.rglob('*') if p.is_file()}
        if names!={p.relative_to(actual) for p in actual.rglob('*') if p.is_file()}:
            raise ValueError('staged actual source inventory differs')
        for name in names:
            if (actual/name).is_symlink() or retention.sha(actual/name)!=retention.sha(expected/name):
                raise ValueError('staged source differs: '+str(name))
    generated=folder/'generated'
    if profile!='call-admission':
        names={'translation.json',*(f'{NAMESPACE}/{name}' for name in ('Types.lean','Funs.lean','FunsExternal_Template.lean'))}
        if profile=='decode-only': names.add(f'{NAMESPACE}/TypesExternal_Template.lean')
        actual={str(p.relative_to(generated)):retention.sha(p) for p in generated.rglob('*') if p.is_file()}
        if names!=actual.keys() or actual!=receipt['generated']: raise ValueError('actual generated inventory differs')
    elif generated.exists() or 'generated' in receipt:
        raise ValueError('B4 source admission cannot claim Lean translation')
    if admit(profile,folder/'actual.llbc',generated)!=receipt['admission']:
        raise ValueError('complete original typed call AST/dictionary admission differs')
    return receipt


def correspondence(parent,plan,current,good,bad,decoder):
    shared=read_exact(parent,plan,'SourcePolicyReadView.lean').decode()
    actual_types=declarations(good/'Types.lean')
    if (actual_types!=declarations(bad/'Types.lean') or actual_types!=declarations(decoder/'Types.lean')
            or len(actual_types)!=4 or any(body not in shared for body in actual_types.values())):
        raise ValueError('original actual Serde trait dictionaries differ from the full parent')
    original,wrong=(declarations(path/'Funs.lean') for path in (good,bad))
    name='controller.lifecycle_inbox.source_work.required_option'
    if set(original)!={name} or original.keys()!=wrong.keys() or original[name]==wrong[name]:
        raise ValueError('only the actual original required-option function body may differ')
    funs=declarations(decoder/'Funs.lean')
    expected={'core.option.Option.Insts.Serde_coreDeDeserialize',
      'controller.lifecycle_inbox.source_work.SourceWorkRecord.Insts.Serde_coreDeDeserialize',
      'controller.lifecycle_inbox.source_work.codec.decode_source_payload'}
    if set(funs)!=expected: raise ValueError('actual decoder forwarding or concrete dictionaries differ')
    externals=(decoder/'TypesExternal_Template.lean').read_text()
    names=set(re.findall(r'^axiom (\S+) : Type$',externals,re.M))
    if names!={'serde_json.error.Error','controller.lifecycle_inbox.source_work.SourceWorkRecord'}:
        raise ValueError('unexpected actual external type in decoder capture')
    full=read_exact(parent,plan,'SandproofSaveRetentionRows/Types.lean').decode()
    generated=declarations(current/'Types.lean')
    selected=('SourceAnchor','SourceWork','SourceWorkRecord')
    for key in selected:
        if generated['controller.lifecycle_inbox.source_work.'+key] not in full:
            raise ValueError('current actual full source carrier differs from the parent: '+key)
    return {'shared_standard_dictionaries':len(actual_types),'actual_required_body_changed':True,
            'actual_decoder_declarations':len(funs),'full_current_carriers':list(selected),'no_external_axiom_import':True}


def check(args):
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE')!='formal': raise ValueError('proof-machine role required')
    os.umask(0o077)
    os.sched_setaffinity(0,sorted(os.sched_getaffinity(0))[:4]);os.environ.update(LEAN_NUM_THREADS='4',LEAN_STACK_SIZE_KB='65536')
    args.output.mkdir(mode=0o700)
    rules={file.name:retention.sha(file) for file in HERE.iterdir() if file.is_file()}
    report=dict(complete=False,conditional_formal_development=True,native_complete=False,public_acceptance=False,
                writer_library_refinement=False,rule_inputs=rules)
    try:
        with args.lock.open('r+') as guard:
            fcntl.flock(guard,fcntl.LOCK_EX|fcntl.LOCK_NB)
            fixed=json.loads((ROOT/'verification/foundations-environment.json').read_bytes())
            for name,path in (('charon',args.tools/'charon/bin/charon'),('aeneas',args.tools/'bin/aeneas')):
                if retention.sha(path)!=fixed[name]['binary_sha256']: raise ValueError('registered extraction tool differs: '+name)
            decision_rules=exact_package(DECISION,PARENT)
            parent,prior=checked_parent(args.parent_output,decision_rules)
            packages=(parent/'.lake/packages').resolve(strict=True);before=dependencies(packages)
            row=row_provider(args.output)
            current=current_capture(args.current_capture,'correct',row,fixed,args.reference,args.bounds_reference,
                args.cursor_reference,args.collector_reference,args.collector_capture)
            captures={name:checked_view(args.captures/name,profile,mode,row) for name,(profile,mode) in CAPTURES.items()}
            original=captures['required-correct']['rust']
            if any(captures[mode]['rust']!=original for mode in ('decode','call-admission')):
                raise ValueError('actual typed source views do not share the same original bytes')
            altered={name for name,value in original.items() if captures['required-wrong']['rust'][name]!=value}
            if altered!={str(SOURCE_WORK)}: raise ValueError('actual wrong source is not restricted to required_option')
            good,bad,decode=(args.captures/name/'generated'/NAMESPACE for name in ('required-correct','required-wrong','decode'))
            report['correspondence']=correspondence(parent,prior,current,good,bad,decode)
            source=args.captures/'call-admission'/'source'
            report['schema']=schema(source/SOURCE_WORK,current/'Types.lean')
            report['library']=library_capture(source,ROOT)
            report['actual_b4_calls']=sorted(captures['call-admission']['admission'])
            if set(report['actual_b4_calls'])!=set(CALLS): raise ValueError('all four complete typed calls required')
            project=args.output/'project';plan=materialize(project,parent,good,bad,decode)
            report['dependencies']=before
            print(plan['root'],len(plan['files']),'real files;',len(plan['audits']),'audits',flush=True)
            report['build']=run(['lake','build',plan['root']],project,args.output/'build.log')
            lock=args.output/'diagnostics.lock';lock.touch(mode=0o600);diag=args.output/'diagnostics'
            report['diagnostics']=run([sys.executable,ROOT/retention.OLD_PACKAGE/'diagnose.py','--project',project,
                '--output',diag,'--lock',lock],ROOT,args.output/'diagnose.log')
            checked_diagnostics(plan,json.loads((diag/'result.json').read_bytes()))
            if plan!=materialize(args.output/'reproduced',parent,good,bad,decode):
                raise ValueError('typed Serde closure did not reproduce')
            if before!=dependencies(packages) or exact_package(DECISION,PARENT)!=decision_rules:
                raise ValueError('fixed prior inputs changed')
        if rules!={file.name:retention.sha(file) for file in HERE.iterdir() if file.is_file()}:
            raise ValueError('rules changed during the check')
        report.update(complete=True,actual_files=len(plan['files']),unique_audits=len(plan['audits']))
    except BaseException as error:
        report['error']=repr(error);raise
    finally:
        retention.write_json(args.output/'result.json',report)

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('captures','parent-output','current-capture','collector-capture','reference','bounds-reference',
                 'cursor-reference','collector-reference','tools','lock','output'):
        parser.add_argument('--'+name,type=lambda value:Path(value).resolve(),required=True)
    check(parser.parse_args())
