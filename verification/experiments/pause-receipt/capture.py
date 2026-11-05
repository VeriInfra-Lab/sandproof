"""Stage this scope's original correct capture for the four old caller CLIs.

The same source/output validators run before any reuse. This stages original
producer bytes only; no model theorem or public success receipt is produced.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path


def load(name,path):
    spec=importlib.util.spec_from_file_location(name,path)
    module=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('source','origin','output'):
        parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--expected-root',action='append',required=True)
    args=parser.parse_args()
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE')!='formal':
        raise ValueError('Original receipt capture may only be staged on the formal host')
    here=Path(__file__).resolve().parent
    support=load('receipt_same_capture_support',here/'support.py')
    checks=load('receipt_same_capture_checks',here/'inventory.py')
    native=load('receipt_same_capture_native_source',here/'native.py')
    if tuple(args.expected_root)!=checks.ROOTS:
        raise ValueError('Calling generator expects different receipt roots')
    result=support.admit_capture(args.origin,args.output,args.source,role='correct',
        roots=checks.ROOTS,native=native,checks=checks)
    (args.output/'capture-reuse.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result),flush=True)


if __name__=='__main__':
    main()
