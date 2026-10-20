"""Admission for the isolated real child-observation candidate, not full V0."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
from urllib.parse import unquote, urlsplit

from tools.check_sqlite_statements import full_digest, check_record, audit_names
from tools.check_sqlite_extraction import surface

ASSETS = Path('verification/experiments/child-observation')
MODELS = Path('SandboxModel/Boundaries/ChildObservation')
LIBRARY = Path('verification/aeneas/child-observation')
ENVIRONMENT = Path('verification/child-observation-environment.json')
FROZEN = Path('verification/child-observation-inputs.sha256')
VARIANTS = {'correct': 'sandproof_child_observation.llbc',
            'caller': 'sandproof_child_observation_caller.llbc',
            'kill-success-release': 'sandproof_child_observation_early_release.llbc'}
VERSIONS = {'server_version':'0.30.0','client_version':'0.13.2','mcp_version':'2.0.0'}

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def read(path):
    return json.loads(path.read_text())

def required(root):
    paths = [*(root/MODELS).glob('*.lean'), root/'SandboxModel/Boundaries/ChildObservation.lean',
        *(root/LIBRARY).rglob('*'), *(root/ASSETS).rglob('*'),
        root/'tools/check_child_observation.py', root/'tools/tests/test_child_observation.py',
        root/'scripts/regenerate-child-observation.sh', root/'scripts/verify-child-observation.sh',
        root/'lake-child-observation.toml', root/ENVIRONMENT,
        root/'tools/lean_session.py', root/'scripts/lean-mcp-server.sh',root/'scripts/python-entry.sh',
        root/'SandboxModel/ProofAuditCommand.lean', root/'lean-toolchain',root/'lake-manifest.json',
        root/'tools/check_sqlite_statements.py',root/'tools/check_sqlite_extraction.py',
        root/'tools/check_foundations_extraction.py']
    paths += [root/'verification/aeneas/llbc'/n for n in VARIANTS.values()]
    paths += [root/n for n in read(root/ENVIRONMENT)['context_sources']]
    return sorted({str(p.relative_to(root)) for p in paths
                   if p.is_file() and '__pycache__' not in p.parts and p.suffix != '.pyc'})

def check_frozen(root):
    rows = {}
    for line in (root/FROZEN).read_text().splitlines():
        m = re.fullmatch(r'([0-9a-f]{64})  (.+)',line)
        if not m: raise ValueError('malformed frozen input')
        digest,name = m.groups()
        if name in rows or Path(name).is_absolute() or '..' in Path(name).parts:
            raise ValueError('duplicate or escaping frozen input')
        rows[name] = digest
        if sha(root/name) != digest: raise ValueError('frozen input changed: '+name)
    if set(rows) != set(required(root)): raise ValueError('frozen input closure differs')
    return rows

def expected_audits(root, variant):
    if variant == 'kill-success-release':
        text = (root/ASSETS/'negative/Counterexample.lean').read_text()
        names = audit_names(text)
        if len(names) != 4: raise ValueError('counterexample audit changed')
        return names
    name = 'ConcreteAudit.lean' if variant == 'caller' else 'Audit.lean'
    actual = audit_names((root/MODELS/name).read_text())
    files = ['Model.lean','Library.lean','Refinement.lean','Guarantees.lean','Examples.lean']
    if variant == 'caller': files.append('Concrete.lean')
    expected = set()
    for file in files:
        text = (root/MODELS/file).read_text()
        namespaces = re.findall(r'^namespace ([A-Za-z0-9_.]+)$', text, re.M)
        if len(namespaces) != 1: raise ValueError('unexpected namespace '+file)
        expected.update(namespaces[0]+'.'+n for n in re.findall(r'^theorem ([A-Za-z0-9_]+)',text,re.M))
    if not expected or actual != expected: raise ValueError('missing/extra public theorem audit')
    return expected

def check_ast(data, frozen):
    if data.get('has_errors') is not False: raise ValueError('erroneous Charon output')
    for key in ['skip_borrowck','no_typecheck','no_normalize','no_reorder_decls']:
        if data['translated']['options'].get(key) is not False:
            raise ValueError('compiler or AST check disabled: '+key)
    if full_digest(data) != frozen['full_llbc_digest']:
        raise ValueError('complete AST/body/Drop/parameter/options changed')
    if surface(data) != frozen['surface']:
        raise ValueError('function/normal/unwind Drop surface changed')

def check_sources(root):
    check_frozen(root)
    env = read(root/ENVIRONMENT)
    for name,digest in env['context_sources'].items():
        if sha(root/name) != digest: raise ValueError('real source context changed: '+name)
    profile = read(root/ASSETS/'candidate.json')
    if sha(root/ASSETS/profile['patch']) != profile['patch_sha256']:
        raise ValueError('candidate patch changed')
    for variant,name in VARIANTS.items():
        frozen = env['variants'][variant]
        check_ast(read(root/'verification/aeneas/llbc'/name),frozen)
        for dest,digest in frozen['retained'].items():
            if sha(root/dest) != digest: raise ValueError('original generated bytes changed: '+dest)
        inventory = read(root/frozen['inventory_path'])
        if inventory != frozen['inventory']: raise ValueError('function/type/trait inventory changed')
        opaque = sorted(f['lean_name'] for f in inventory['functions'] if f['is_opaque'])
        if opaque != frozen['opaque']: raise ValueError('opaque interface changed')
        expected_audits(root,variant)
    contracts = read(root/ASSETS/'contracts.json')
    if len(contracts.get('pure_value_interfaces',[]))!=3:raise ValueError('incomplete pure value interface registration')
    if len(contracts['symbols']) != 9: raise ValueError('incomplete adapter registration')
    for item in contracts['symbols']:
        if not all(item.get(k) for k in ['symbol','provider','normal_guarantee','fault_and_non_guarantee','function_sha256']):
            raise ValueError('incomplete guarantee')
        a,b=item['candidate_lines']
        # Exact snippets in the isolated adapter, independent of LLBC opaque summaries.
        text=''.join((root/ASSETS/'observation-io.rs').read_text().splitlines(keepends=True)[a-1:b])
        if hashlib.sha256(text.encode()).hexdigest()!=item['function_sha256']:
            raise ValueError('mechanical adapter source differs: '+item['symbol'])
    lean = [*(root/MODELS).glob('*.lean'),*(root/LIBRARY).rglob('*.lean'),
            *(root/ASSETS/'negative').glob('*.lean')]
    for p in lean:
        if re.search(r'\b(sorry|admit|axiom)\b',p.read_text()):
            raise ValueError('placeholder/custom assumption: '+str(p))
    return env

def check_extraction(root,path,variant):
    env=read(root/ENVIRONMENT); frozen=env['variants'][variant]; result=read(path/'result.json')
    if (result.get('success') is not True or result.get('sources_unchanged') is not True
            or result.get('charon_exit_code') != 0 or result.get('aeneas_exit_code') != 0
            or result.get('has_errors') is not False or result.get('charon_timed_out') or result.get('aeneas_timed_out')):
        raise ValueError('actual extraction failed or incomplete')
    if result['tools']!=env['tools'] or result['binary_sha256']!=env['tool_binaries_sha256']:
        raise ValueError('original tool changed')
    generated={str(p.relative_to(path/'generated')):sha(p) for p in (path/'generated').rglob('*') if p.is_file()}
    if generated != result['generated'] or generated != frozen['generated']:
        raise ValueError('generated output/templates differ')
    data=read(path/'original.llbc')
    if sha(path/'original.llbc')!=result['llbc_sha256']:raise ValueError('LLBC evidence damaged')
    check_ast(data,frozen)
    if result['charon_command'][result['charon_command'].index('--dest-file')+1]!=data['translated']['options']['dest_file']:
        raise ValueError('invocation destination differs')
    return data

def check_tools(root,tool_root):
    env=read(root/ENVIRONMENT)
    for name,revision in env['tools'].items():
        if subprocess.check_output(['git','-C',str(tool_root/name),'rev-parse','HEAD'],text=True).strip()!=revision:
            raise ValueError('original tool revision changed')
        if subprocess.check_output(['git','-C',str(tool_root/name),'status','--porcelain','--untracked-files=no'],text=True):
            raise ValueError('original tool tracked source changed')
    for key,name in [('charon','aeneas/charon/bin/charon'),('aeneas','aeneas/bin/aeneas')]:
        if sha(tool_root/name)!=env['tool_binaries_sha256'][key]:raise ValueError('tool binary changed')
    backend=root/'.lake/packages/aeneas'
    if subprocess.check_output(['git','-C',str(backend),'rev-parse','HEAD'],text=True).strip()!=env['tools']['aeneas']:
        raise ValueError('Lean backend revision changed')
    if subprocess.check_output(['git','-C',str(backend),'status','--porcelain','--untracked-files=no'],text=True):
        raise ValueError('Lean backend source changed')
    charon=tool_root/'aeneas/charon/bin/charon'
    if subprocess.check_output(['rustc','-vV'],text=True)!=env['production_rustc']:
        raise ValueError('production compiler changed')
    if subprocess.check_output([str(charon),'toolchain-version'],text=True).strip()!=env['extraction_toolchain']:
        raise ValueError('extraction compiler changed')
    for role,base in {'production':Path(subprocess.check_output(['rustc','--print','sysroot'],text=True).strip()),
                      'extraction':Path(subprocess.check_output([str(charon),'toolchain-path'],text=True).strip())}.items():
        for name,digest in env['library_sources'][role].items():
            if sha(base/'lib/rustlib/src/rust/library'/name)!=digest:raise ValueError('standard library source changed')

def fresh_native_artifact(log, target, source):
    """Bind the actual unit harness containing the changed business body.

    cargo --lib puts the library body and its tests in the same executable;
    a new unrelated integration binary or a reused original body is insufficient.
    """
    rows=[]
    for line in log.read_text().splitlines():
        try:row=json.loads(line)
        except ValueError:continue
        if (row.get('reason')=='compiler-artifact'
                and row.get('target',{}).get('name')=='sandbox_runtime'
                and row.get('target',{}).get('kind')==['lib']
                and row.get('profile',{}).get('test') is True):rows.append(row)
    if len(rows)!=1 or rows[0].get('fresh') is not False:
        raise ValueError('fresh unambiguous actual business unit harness required')
    row=rows[0];package=(source/'sandbox-runtime').resolve()
    identity=urlsplit(row.get('package_id','').split('#',1)[0])
    if (Path(row.get('manifest_path','')).resolve()!=package/'Cargo.toml'
            or Path(row['target'].get('src_path','')).resolve()!=package/'src/lib.rs'
            or identity.scheme!='path+file' or identity.netloc
            or Path(unquote(identity.path)).resolve()!=package):
        raise ValueError('actual compiled package/source identity differs')
    binary=row.get('executable');files=row.get('filenames',[])
    if (not binary or not files or binary not in files
            or any(not Path(n).resolve().is_relative_to(target.resolve()) for n in files)):
        raise ValueError('missing or escaped actual harness executable')
    return row

def check_native(path, root=None):
    record=read(path/'complete.json')
    if record.get('success') is not True or set(record['variants'])!={'original','correct','kill-success-release'}:
        raise ValueError('native batch incomplete')
    for name,variant in record['variants'].items():
        commands=variant['commands']
        tags=['format-test-attachments','clean-owned-package','compile-real-source']
        tags += ['same-accounted-'+n for n in ['unknown_after_none_preserves_exact_original_pair','unknown_after_errors_preserves_exact_original_pair']] if name=='kill-success-release' else ['actual-child-tests','clippy']
        if len(commands)!=len(tags):raise ValueError('native commands missing/extra')
        for tag,item in zip(tags,commands):
            expected=101 if tag.startswith('same-accounted-') else 0
            if item['expected_exit_code']!=expected or item['exit_code']!=expected or sha(path/name/(tag+'.log'))!=item['log_sha256']:
                raise ValueError('native command/log differs')
        source=path/name/'source'
        artifact=fresh_native_artifact(path/name/'compile-real-source.log',Path(record['target_dir']),source)
        if artifact!=variant.get('actual_artifact'):raise ValueError('actual artifact provenance differs')
        if sha(path/name/'actual-tests')!=variant['binary_sha256']:raise ValueError('native binary damaged')
        if name=='kill-success-release':
            bad=[x for x in commands if x['expected_exit_code']==101]
            if len(bad)!=2:raise ValueError('two independent wrong processes required')
            for case in ['unknown_after_none_preserves_exact_original_pair','unknown_after_errors_preserves_exact_original_pair']:
                p=path/name/('same-accounted-'+case+'.log')
                if p.read_text().count('unknown result lost the original Child/Slot responsibility')!=1:
                    raise ValueError('wrong process failed for another reason')
        else:
            p=path/name/'actual-child-tests.log'
            count=14 if name=='original' else 16
            if f'test result: ok. {count} passed; 0 failed; 0 ignored;' not in p.read_text():
                raise ValueError('real OS tests absent or skipped')
    cleanup=record.get('mutant_cache_cleanup',{})
    if cleanup.get('exit_code')!=0 or cleanup.get('expected_exit_code')!=0 or sha(path/'mutant-cache-cleanup.log')!=cleanup.get('log_sha256'):
        raise ValueError('owned mutant package cache cleanup incomplete')
    if root is not None:
        if record['runtime']['rustc'].strip()!=read(root/ENVIRONMENT)['production_rustc'].strip():raise ValueError('native compiler differs from registered production version')
        profile=read(root/ASSETS/'candidate.json')
        for variant in record['variants'].values():
            materialized=variant['materialized']
            if any(materialized.get(k)!=profile[k] for k in ['base','patch_sha256']):raise ValueError('native source profile differs')
            if materialized.get('archive_sha256')!=profile['source_archive_sha256']:raise ValueError('native source archive differs')
        for name,digest in record['inputs'].items():
            if sha(root/ASSETS/name)!=digest:raise ValueError('native input differs: '+name)
    return record

def check_formal(root, output):
    """Reuse only exact physical Lean inputs; prose-only edits cannot bless code changes."""
    env=check_sources(root);record=read(output/'result.json')
    if record.get('success') is not True or set(record['views'])!=set(VARIANTS):
        raise ValueError('formal batch incomplete')
    for variant,view_record in record['views'].items():
        view=output/variant
        mapping={MODELS/n:MODELS/n for n in ['Events.lean','Interface.lean','Model.lean','Library.lean','Fixtures.lean']}
        target='SandboxModel.Boundaries.ChildObservation.Audit'
        if variant=='kill-success-release':
            audit=Path('SandboxModel/ChildObservationNegative.lean');target='SandboxModel.ChildObservationNegative'
            mapping[audit]=ASSETS/'negative/Counterexample.lean'
            for n in ['Types.lean','Funs.lean','FunsExternal.lean']:
                mapping[LIBRARY/'SandproofChildObservation'/n]=ASSETS/'negative'/n
        else:
            for n in ['Refinement.lean','Guarantees.lean','Examples.lean','Audit.lean']:
                mapping[MODELS/n]=MODELS/n
            public=Path('SandboxModel/Boundaries/ChildObservation.lean');mapping[public]=public
            if variant=='caller':
                for n in ['Concrete.lean','ConcreteAudit.lean']:mapping[MODELS/n]=MODELS/n
                target='SandboxModel.Boundaries.ChildObservation.ConcreteAudit'
            audit=MODELS/('ConcreteAudit.lean' if variant=='caller' else 'Audit.lean')
            original=LIBRARY/'concrete/SandproofChildObservation' if variant=='caller' else LIBRARY/'SandproofChildObservation'
            for p in (root/original).glob('*.lean'):
                mapping[LIBRARY/'SandproofChildObservation'/p.name]=p.relative_to(root)
        for dest,source in mapping.items():
            if sha(view/dest)!=sha(root/source):raise ValueError('proof result for different source: '+str(source))
        for n in ['SandboxModel/ProofAuditCommand.lean','tools/lean_session.py','scripts/lean-mcp-server.sh','lean-toolchain','lake-manifest.json']:
            if sha(view/n)!=sha(root/n):raise ValueError('proof driver/config changed')
        config=(root/'lake-child-observation.toml').read_text().replace('SandboxModel.Boundaries.ChildObservation.Audit',target)
        if (view/'lakefile.toml').read_text()!=config:raise ValueError('proof view configuration changed')
        for n,digest in view_record['inputs'].items():
            if sha(view/n)!=digest:raise ValueError('formal view input damaged')
        log=view/'artifacts/service.log'
        if sha(log)!=view_record['service_sha256']:raise ValueError('service evidence damaged')
        rows=[json.loads(s) for s in log.read_text().splitlines() if s.startswith('{')]
        rows=[r for r in rows if r.get('request',{}).get('tool')=='lean_diagnostic_messages']
        if len(rows)!=len(mapping) or {r['request']['arguments']['file_path'] for r in rows}!={str(n) for n in mapping}:
            raise ValueError('physical file diagnostics omitted or repeated')
        count=0
        for row in rows:
            n=Path(row['request']['arguments']['file_path'])
            expected=expected_audits(root,variant) if n==audit else (expected_audits(root,'correct') if n==MODELS/'Audit.lean' else set())
            count+=check_record(view,row,expected,n,VERSIONS)
        if count!=view_record['physical_audits'] or len(rows)!=view_record['physical_files']:
            raise ValueError('formal result counts differ from actual records')
    return record

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root',type=Path,default=Path(__file__).resolve().parents[1])
    parser.add_argument('--native',type=Path)
    parser.add_argument('--formal',type=Path)
    args=parser.parse_args();check_sources(args.root.resolve(strict=True))
    if args.native:check_native(args.native,args.root)
    if args.formal:check_formal(args.root,args.formal)
    print(json.dumps({'admitted':True,'scope':'isolated-child-observation-not-full-V0'}))
