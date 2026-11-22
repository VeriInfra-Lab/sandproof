"""Prepare two new shared-policy views with the unchanged63 parent assertions."""
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
capture=module('continue_network_native_capture',HERE/'capture.py')
parent=module('continue_network_native_parent',HERE.parent/'continue-work-binding/native.py')
base=parent.base
VARIANTS=('original','network-policy-as-deny-all')
EXPECTED_TESTS=parent.EXPECTED_TESTS
REJECTION=parent.REJECTION['network-policy-as-deny-all']

def digest(path):return sha256(path.read_bytes()).hexdigest()
def rules():
    return dict(parent.rules(),**{str(p.relative_to(parent.capture.retire.REPO)):digest(p)
        for p in sorted(HERE.iterdir()) if p.is_file()})

def prepare(output,variant):
    os.umask(0o077)
    if variant not in VARIANTS or len(EXPECTED_TESTS)!=63 or len(set(EXPECTED_TESTS))!=63:
        raise ValueError('two exact new views retain all63 original assertions')
    sealed=rules();legacy='adapted-'+variant
    parent.prepare(output,legacy)
    meta=output/'input.json';old=json.loads(meta.read_bytes());source=output/'source'
    before=base.inventory(source)
    if (old['source']!=parent.capture.retire.SOURCE or old['tree']!=parent.capture.retire.TREE
            or old['variant']!=legacy or old['prepared'] is not True or old['compiled'] is not False
            or old['tests_executed'] is not False or old['tests']!=list(EXPECTED_TESTS)
            or before!=old['installed'] or len(before)!=4906 or old['driver']!=digest(Path(parent.__file__))):
        raise ValueError('the complete unchanged parent63 preparation is mandatory')
    meta.rename(output/'parent-input.json')
    # Parent already installs the actual same JSON/SHA calls and exact wrong
    # policy at its original caller. Move that one tuple without changing it.
    capture.factor(source,parent.capture.ports)
    path=source/parent.capture.ports.SOURCE
    subprocess.run(['rustfmt','--edition','2024',str(path)],cwd=source,check=True)
    installed=base.inventory(source)
    if (before.keys()!=installed.keys() or {n for n in before if before[n]!=installed[n]}!={str(parent.capture.ports.SOURCE)}
            or any(before[n]['executable']!=installed[n]['executable'] for n in before)):
        raise ValueError('only the actual original continuation call/transparent helper may differ')
    tests=(source/base.TESTS).read_text()
    if any(len(re.findall(r'#\[test\]\s+fn\s+'+re.escape(name)+r'\(',tests))!=1 for name in EXPECTED_TESTS):
        raise ValueError('all63 unchanged actual assertions required')
    if rules()!=sealed:raise ValueError('actual native rule inputs changed during preparation')
    row=dict(prepared=True,compiled=False,tests_executed=False,source=old['source'],tree=old['tree'],archive=old['archive'],
        variant=variant,parent_variant=legacy,parent_input=digest(output/'parent-input.json'),
        driver=digest(Path(__file__)),capture_driver=digest(HERE/'capture.py'),parent_driver=digest(Path(parent.__file__)),
        rules=sealed,tests=list(EXPECTED_TESTS),original=old['original'],installed=installed,
        extended_parent_files=[str(parent.capture.ports.SOURCE)],
        changed=sorted(n for n in old['original'] if old['original'][n]!=installed[n]),
        expected=dict(passed=63 if variant=='original' else 62,failed=0 if variant=='original' else 1),
        rejection=None if variant=='original' else REJECTION,
        restore='after this wrong view independently rebuild original source/target/businesslib/libtest; all63 pass')
    meta.write_text(json.dumps(row,indent=2)+'\n')
    print(variant,'new shared-policy source with all63 original assertions prepared; no compile/test/restore')

if __name__=='__main__':
    cli=argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant',choices=VARIANTS,required=True);cli.add_argument('--output',type=Path,required=True)
    args=cli.parse_args();prepare(args.output.resolve(),args.variant)
