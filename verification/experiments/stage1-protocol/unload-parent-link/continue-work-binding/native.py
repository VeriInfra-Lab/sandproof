"""Prepare all63 assertions in eight complete views; never compile or test."""
import argparse
from hashlib import sha256
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess

HERE=Path(__file__).resolve().parent
def module(name,path):
    spec=importlib.util.spec_from_file_location(name,path)
    value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value
capture=module('continue_binding_native_capture',HERE/'capture.py')
tuples=module('continue_binding_native_tuple',HERE/'tuple.py')
parent=module('continue_binding_native_parent',HERE.parent/'source-end-accessor/native.py')
base=parent.base_native
VARIANTS=('original','adapted-original','factored-original','network-policy-as-deny-all',
    'adapted-network-policy-as-deny-all','file-policy-as-zero','adapted-file-policy-as-zero','factored-file-policy-as-zero')
OWN_TESTS=('source_continue_binding_preserves_file_policy','source_continue_binding_preserves_network_policy')
EXPECTED_TESTS=(*parent.EXPECTED_TESTS,*OWN_TESTS)
REJECTION={'file-policy-as-zero':'continuation binding must include the complete supplied file policy',
    'network-policy-as-deny-all':'continuation binding must include the complete supplied network policy'}

def digest(path):return sha256(path.read_bytes()).hexdigest()

def rules():
    return dict(parent.rules(),**{str(p.relative_to(capture.retire.REPO)):digest(p)
        for p in sorted(HERE.iterdir()) if p.is_file()})

def prepare(output,variant):
    os.umask(0o077)
    if variant not in VARIANTS:raise ValueError('all eight exact original/adapted/factored views required')
    fixture=(HERE/'native.rs.in').read_bytes()
    if tuple(re.findall(rb'#\[test\]\s+fn\s+(\w+)\(',fixture))!=tuple(n.encode() for n in OWN_TESTS):
        raise ValueError('both complete actual binding assertions required')
    if len(parent.EXPECTED_TESTS)!=61 or len(EXPECTED_TESTS)!=63 or len(set(EXPECTED_TESTS))!=63:
        raise ValueError('all61 parent assertions must remain, plus exactly two')
    if any(message.encode() not in fixture for message in REJECTION.values()):
        raise ValueError('both exact same-property runtime assertions required')
    sealed=rules();adapted=variant.startswith(('adapted-','factored-'))
    actual=variant.removeprefix('adapted-').removeprefix('factored-')
    legacy='adapted-original' if adapted else 'original'
    parent.prepare(output,legacy)
    meta=output/'input.json';old=json.loads(meta.read_bytes());source=output/'source'
    before=base.inventory(source)
    if (old['source']!=capture.retire.SOURCE or old['tree']!=capture.retire.TREE
            or old['variant']!=legacy or old['tests']!=list(parent.EXPECTED_TESTS)
            or old['prepared'] is not True or old['compiled'] is not False or old['tests_executed'] is not False
            or before!=old['installed'] or len(before)!=4906
            or old['driver']!=digest(Path(parent.__file__))):
        raise ValueError('complete original61 fixture input required')
    meta.rename(output/'parent-input.json')
    changed={str(base.TESTS)}
    if actual!='original' or adapted:
        path=source/capture.ports.SOURCE
        path.write_text(capture.ports.mutate(path.read_text(),actual))
        if adapted:capture.ports.stage(source)
        if variant.startswith('factored-'):tuples.factor(source)
        subprocess.run(['rustfmt','--edition','2024',str(path)],cwd=source,check=True)
        changed.add(str(capture.ports.SOURCE))
    tests=source/base.TESTS;tests.write_bytes(tests.read_bytes()+b'\n'+fixture)
    subprocess.run(['rustfmt','--edition','2024',str(tests)],cwd=source,check=True)
    installed=base.inventory(source)
    actual_changed={n for n in before if before[n]!=installed[n]}
    if (before.keys()!=installed.keys() or actual_changed!=changed
            or any(before[n]['executable']!=installed[n]['executable'] for n in before)
            or installed[str(capture.retire.LIB)]!=old['original'][str(capture.retire.LIB)]):
        raise ValueError('only the exact continuation staging/mutation and two extra tests may extend parent')
    if any(len(re.findall(r'#\[test\]\s+fn\s+'+re.escape(name)+r'\(',tests.read_text()))!=1 for name in EXPECTED_TESTS):
        raise ValueError('all63 actual assertions must remain uniquely present')
    if rules()!=sealed:raise ValueError('actual source rules changed during preparation')
    row=dict(prepared=True,compiled=False,tests_executed=False,source=capture.retire.SOURCE,
        tree=capture.retire.TREE,archive=capture.retire.ARCHIVE,variant=variant,parent_variant=legacy,
        parent_input=digest(output/'parent-input.json'),driver=digest(Path(__file__)),
        capture_driver=digest(HERE/'capture.py'),tuple_driver=digest(HERE/'tuple.py'),
        ports_driver=digest(HERE/'ports.py'),parent_driver=digest(Path(parent.__file__)),
        rules=sealed,own_fixture=digest(HERE/'native.rs.in'),tests=list(EXPECTED_TESTS),
        original=old['original'],installed=installed,extended_parent_files=sorted(actual_changed),
        changed=sorted(n for n in old['original'] if old['original'][n]!=installed[n]),
        expected=dict(passed=63 if actual=='original' else 62,failed=0 if actual=='original' else 1),
        rejection=REJECTION.get(actual),restore='independent original source/target rebuild; all63 after each wrong')
    meta.write_text(json.dumps(row,indent=2)+'\n')
    print(variant,'all63 assertions prepared; compilation/execution/restore pending')

if __name__=='__main__':
    cli=argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant',choices=VARIANTS,required=True);cli.add_argument('--output',type=Path,required=True)
    args=cli.parse_args();prepare(args.output.resolve(),args.variant)
