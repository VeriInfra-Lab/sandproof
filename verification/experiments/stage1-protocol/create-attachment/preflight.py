"""Capture original V3 attachment/transaction/identity roots without body edits.

Capability evidence only. Generated axiom templates are never imported.
"""
import argparse
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/create-attachment')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


parent = module('attachment_existing_load_inputs', ROOT / HERE.parent / 'inbox-load/formal.py')
mapping = module('attachment_existing_runtime_mapping', ROOT / HERE.parent / 'formal_runtime.py')
sha = parent.sha


def run(output, mode, lock_path):
    output.mkdir(parents=True, mode=0o700)
    project = output / 'source'
    rec = dict(kind='original-create-attachment-capability-preflight', mode=mode,
        complete=False, capture_complete=False, translation_complete=False,
        semantic_proof=False, property_rejected=False, original_business_body_modified=False,
        production_modified=False, new_contract_adopted=False, new_axioms=False,
        generated_bodies_modified=False, generated_axiom_templates_imported=False,
        lean_checks_run=False, native_checks_run=False, formal_acceptance=False,
        ning_acceptance=False, shared_acceptance=False, full_v3=False,
        phase='waiting-for-heavy-lock', commands=[])
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', CARGO_NET_OFFLINE='true',
               LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')

    def command(label, argv):
        rec['phase'] = label
        (output/'progress.json').write_text(json.dumps(rec,indent=2)+'\n')
        log = output / (label + '.log')
        begin = time.monotonic()
        with log.open('xb') as stream:
            done = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        rec['commands'].append(dict(label=label, argv=argv, cwd=str(project),
            exit_code=done.returncode, seconds=time.monotonic()-begin, log_sha256=sha(log)))
        (output/'progress.json').write_text(json.dumps(rec,indent=2)+'\n')
        if done.returncode:
            raise ValueError(label+' failed; capability evidence, not a property rejection')

    waiting = time.monotonic()
    (output/'progress.json').write_text(json.dumps(rec,indent=2)+'\n')
    try:
        with lock_path.open('r+') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            rec['lock_wait_seconds'] = time.monotonic()-waiting
            started = time.monotonic()
            inv = json.loads((ROOT/HERE/'inventory.json').read_text())
            recipe = inv['modes'][mode]
            for name,digest in inv['source_inputs'].items():
                if sha(ROOT/name) != digest:
                    raise ValueError('reviewed original input changed: '+name)
            if sha(Path(__file__)) != inv['driver_sha256']:
                raise ValueError('reviewed preflight driver changed')
            for name,digest in inv['templates'].items():
                if sha(ROOT/HERE/name) != digest:
                    raise ValueError('reviewed forwarding root changed: '+name)
            inputs = parent.base.snapshot(ROOT)
            expected = dict(inputs)
            rec.update(inputs=inputs,
                input_modes={n:('100755' if (ROOT/n).stat().st_mode&0o111 else '100644') for n in inputs},
                source_commit=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
                source_tree=subprocess.check_output(['git','rev-parse','HEAD^{tree}'],cwd=ROOT,text=True).strip(),
                driver_sha256=sha(Path(__file__)), inventory_sha256=sha(ROOT/HERE/'inventory.json'))
            registry = parent.native.registry_module.registry()
            rec['registry'] = registry
            fixed = json.loads((ROOT/'verification/foundations-environment.json').read_text())
            rec['tools'] = fixed
            for name,suffix in [('charon','charon/bin/charon'),('aeneas','bin/aeneas')]:
                if sha(ROOT/'.tools/aeneas'/suffix) != fixed[name]['binary_sha256']:
                    raise ValueError('fixed binary changed: '+name)
            project.mkdir(mode=0o700)
            for name,digest in inputs.items():
                path = project/name
                path.parent.mkdir(parents=True,exist_ok=True)
                shutil.copy2(ROOT/name,path)
                if sha(path) != digest:
                    raise ValueError('source changed while copying: '+name)
            (project/'.tools').symlink_to(mapping.tool_root(ROOT),target_is_directory=True)
            (project/'.lake').mkdir()
            (project/'.lake/build').mkdir()
            (project/'.lake/packages').symlink_to(mapping.package_root(ROOT),target_is_directory=True)
            (project/'artifacts/extract-target').mkdir(parents=True)
            source = recipe['source']
            root_text = (ROOT/HERE/recipe['template']).read_bytes()
            (project/source).write_bytes((project/source).read_bytes()+root_text)
            expected[source] = sha(project/source)
            rec['private_source_derivation'] = dict(path=source,
                original_sha256=inputs[source],derived_sha256=expected[source],
                original_body_preserved=True,suffix=recipe['template'],suffix_sha256=sha(ROOT/HERE/recipe['template']))
            paths = [Path.home(),mapping.tool_root(ROOT),mapping.package_root(ROOT)]
            for key in ('CARGO_HOME','RUSTUP_HOME','ELAN_HOME','SANDPROOF_PROOF_RUNTIME_ROOT'):
                if os.environ.get(key): paths.append(Path(os.environ[key]).resolve(strict=True))
            wrapper = ['bwrap','--ro-bind','/','/','--dev','/dev','--proc','/proc',
                '--tmpfs','/tmp','--tmpfs','/home',
                *[a for p in dict.fromkeys(paths) for a in ('--ro-bind',str(p),str(p))],
                '--bind',str(project),str(mapping.VIRTUAL),'--chdir',str(mapping.VIRTUAL),
                '--setenv','CARGO_TARGET_DIR',str(mapping.VIRTUAL/'artifacts/extract-target'),
                '--setenv','CARGO_NET_OFFLINE','true','--setenv','CARGO_BUILD_JOBS','4',
                '--unsetenv','RUSTFLAGS','--unsetenv','CARGO_ENCODED_RUSTFLAGS','--']
            includes,opaque = list(parent.admission.INCLUDE),list(parent.admission.OPAQUE)
            rec['diagnostic_options'] = dict(root=recipe['root'],include=includes,opaque=opaque,
                contracts_adopted=False,new_business_opacity=False)
            ir = Path('artifacts/create-attachment.llbc')
            argv = [str(mapping.VIRTUAL/'.tools/aeneas/charon/bin/charon'),'cargo',
                '--preset=aeneas','--error-on-warnings','--sysroot','default']
            for name in includes: argv += ['--include',name]
            for name in opaque: argv += ['--opaque',name]
            argv += ['--start-from',recipe['root'],'--dest-file',str(mapping.VIRTUAL/ir),'--',
                '--manifest-path','verification/experiments/stage1-protocol/inbox-record/Cargo.toml',
                '--lib','--locked','--offline','--target',fixed['target']]
            command('charon',wrapper+argv)
            raw = json.loads((project/ir).read_text())
            translated = raw['translated']
            options = translated['options']
            if (raw['has_errors'] or options['start_from'] != [recipe['root']]
                    or options['include'] != includes or options['opaque'] != opaque
                    or options['exclude'] or options['skip_borrowck'] or options['no_typecheck']
                    or options['monomorphize'] or options['remove_unused_clauses']
                    or {t['key'] for t in translated['target_information']} != {fixed['target']}):
                raise ValueError('unreviewed or incomplete capture')
            functions = [f for f in translated['fun_decls'] if f]
            roots = [f for f in functions if f['item_meta']['name'] ==
                [{'Ident':[n,0]} for n in recipe['root'].split('::')]]
            if (len(roots)!=1 or roots[0]['item_meta']['opacity']!='Transparent'
                    or 'Structured' not in roots[0]['body']):
                raise ValueError('transparent forwarding root missing')
            root = roots[0]
            calls = []
            def walk(node):
                if isinstance(node,dict):
                    if 'Call' in node: calls.append(node['Call']['call'])
                    for child in node.values(): walk(child)
                elif isinstance(node,list):
                    for child in node: walk(child)
            walk(root['body'])
            if (len(calls)!=1 or len(calls[0]['args'])!=recipe['arguments']
                    or calls[0]['dest']['kind']!={'Local':0}
                    or root['body']['Structured']['locals']['arg_count']!=recipe['arguments']):
                raise ValueError('forwarder must call the actual root once with all arguments')
            actual = next(f for f in functions if f['def_id']==calls[0]['func']['Regular']['kind']['Fun'])
            prefix = [{'Ident':[n,0]} for n in recipe['callee_prefix'].split('::')]
            if (actual['item_meta']['name'][:len(prefix)]!=prefix
                    or actual['item_meta']['name'][-1]!={'Ident':[recipe['callee_name'],0]}
                    or actual['item_meta']['opacity']!='Transparent' or 'Structured' not in actual['body']):
                raise ValueError('actual business root missing, changed or opaque')
            # Retain the exact metadata and argument/borrow binding for review.
            rec.update(capture_complete=True,llbc_sha256=sha(project/ir),llbc_bytes=(project/ir).stat().st_size,
                actual_call_binding=dict(root=root['def_id'],actual=actual['def_id'],
                    actual_name=actual['item_meta']['name'],single_call=True,call=calls[0]),
                captured_functions=len(functions),captured_types=len([x for x in translated['type_decls'] if x]))
            del raw,translated,functions,roots,root,actual,calls
            generated = output/'generated'
            generated.mkdir()
            command('aeneas',[str(ROOT/'.tools/aeneas/bin/aeneas'),'-backend','lean',
                '-abort-on-error','-warnings-as-errors','-split-files','-emit-json',
                '-namespace','SandproofStage1CreateAttachmentRust','-subdir','SandproofStage1CreateAttachment',
                '-dest',str(generated),str(project/ir)])
            translation = json.loads((generated/'translation.json').read_text())
            actuals = [f for f in translation['functions']
                if f['rust_name'].startswith(recipe['callee_prefix']+'::')
                and f['rust_name'].endswith('::'+recipe['callee_name'])
                and (not recipe.get('callee_type') or recipe['callee_type'] in f['rust_name'])]
            if len(actuals)!=1 or actuals[0]['is_opaque']:
                raise ValueError('actual generated function missing, ambiguous or external')
            file = generated/actuals[0]['lean_file']
            short = actuals[0]['lean_name'].removeprefix('SandproofStage1CreateAttachmentRust.')
            text = file.read_text()
            if (len(re.findall(r'^namespace SandproofStage1CreateAttachmentRust$',text,re.M))!=1
                    or len(re.findall(r'^def\s+'+re.escape(short)+r'\b',text,re.M))!=1):
                raise ValueError('actual body absent from unmodified generated source')
            rec.update(translation_complete=True,complete=True,phase='original-capability-observed',
                generated_root=actuals[0],generated_functions=translation['functions'])
    except BaseException as error:
        rec.update(error=repr(error),traceback=traceback.format_exc())
    finally:
        try:
            if 'inputs' in locals():
                rec['original_inputs_unchanged'] = parent.base.snapshot(ROOT)==inputs
                rec['original_modes_unchanged'] = all(
                    ('100755' if (ROOT/n).stat().st_mode&0o111 else '100644')==m
                    for n,m in rec['input_modes'].items())
                rec['private_inputs_unchanged'] = all(sha(project/n)==h for n,h in expected.items())
                rec['registry_unchanged'] = registry==parent.native.registry_module.registry()
                if not all(rec[k] for k in ('original_inputs_unchanged','original_modes_unchanged',
                                            'private_inputs_unchanged','registry_unchanged')):
                    raise ValueError('source/modes/registry changed')
            if 'driver_sha256' in rec and (sha(Path(__file__))!=rec['driver_sha256']
                    or sha(ROOT/HERE/'inventory.json')!=rec['inventory_sha256']):
                raise ValueError('driver/inventory changed')
        except BaseException as error:
            rec.update(complete=False,final_input_error=repr(error))
        rec['generated_files'] = {str(p.relative_to(output/'generated')):sha(p)
            for p in (output/'generated').rglob('*') if p.is_file()}
        rec['execution_seconds'] = time.monotonic()-started if 'started' in locals() else None
        (output/'result.json').write_text(json.dumps(rec,indent=2)+'\n')
    return rec


if __name__=='__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode',choices=('attachment','world','identity'),required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--lock',type=Path,required=True,help='existing shared heavy-proof lock')
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute(),args.mode,args.lock)
    print(json.dumps({k:result.get(k) for k in
        ('complete','mode','phase','capture_complete','translation_complete','error','execution_seconds')}),flush=True)
    raise SystemExit(not result['complete'])
